use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use rusqlite::{Connection, ErrorCode, OpenFlags};

use crate::{Error, Result};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SNAPSHOT_PREFIX: &str = "cursor-session-";
const STALE_SNAPSHOT: Duration = Duration::from_secs(60 * 60);

/// Size and modification time of a database file.
type Stamp = (u64, Option<SystemTime>);

/// Runs `read` on a read-only connection to `path` (see [`open_readonly`]).
///
/// An immutable connection takes no locks, so Cursor starting up during the
/// read can change the file under it. When such a read fails as corrupt, or
/// the file changed before it finished, `read` runs once more on a new
/// connection, which by then usually reads in place. If the file changes
/// under that read too, its result is not trusted. A read that finds a
/// journal a crash left, which only a connection that can write may roll
/// back, runs once more on a copy.
pub fn with_readonly<T>(path: &Path, mut read: impl FnMut(&Connection) -> Result<T>) -> Result<T> {
    let db = open_readonly(path, false)?;
    let result = read(&db);
    let copy = needs_rollback(&result);
    if !copy && !db.torn(&result) {
        return result;
    }
    drop(db);
    let db = open_readonly(path, copy)?;
    let result = read(&db);
    if db.changed() {
        return Err(Error::Changed {
            path: path.to_path_buf(),
        });
    }
    result
}

/// Whether `result` failed on a hot journal, which SQLite rolls back only on
/// a connection that can write.
fn needs_rollback<T>(result: &Result<T>) -> bool {
    matches!(
        result,
        Err(Error::Database { source, .. }) if source
            .sqlite_error()
            .is_some_and(|error| error.extended_code == rusqlite::ffi::SQLITE_READONLY_ROLLBACK)
    )
}

/// A read-only connection. When the database had to be copied first (see
/// [`open_readonly`]), the copy lives as long as the connection.
struct Db {
    // Field order matters: the connection must close before the copy is removed.
    conn: Connection,
    /// The file an immutable connection reads, stamped before it was opened.
    immutable: Option<(PathBuf, Stamp)>,
    _snapshot: Option<Snapshot>,
}

impl Deref for Db {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.conn
    }
}

impl Db {
    /// Whether `result` was read without locks from a file that changed, or
    /// that looked corrupt, as a concurrent checkpoint can make it look.
    fn torn<T>(&self, result: &Result<T>) -> bool {
        let corrupt = matches!(
            result,
            Err(Error::Database { source, .. }) if matches!(
                source.sqlite_error_code(),
                Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
            )
        );
        (self.immutable.is_some() && corrupt) || self.changed()
    }

    /// Whether the file read without locks changed since it was opened.
    fn changed(&self) -> bool {
        self.immutable
            .as_ref()
            .is_some_and(|(file, opened)| stamp(file).ok().as_ref() != Some(opened))
    }
}

/// How [`open_readonly`] reads a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// SQLite's normal read-only open, sharing locks with writers.
    InPlace,
    /// Only the main file, without locks (`immutable=1`).
    Immutable,
    /// A private copy of the main file and its `-wal` or `-journal`.
    Snapshot,
}

impl Mode {
    /// A `-wal` next to a `-shm` belongs to a writer that has the database
    /// open, even when a checkpoint has just emptied the `-wal`. Otherwise a
    /// WAL database with no `-wal`, or an empty one, has every commit in its
    /// main file, and a non-empty `-wal` was left behind by a crash.
    ///
    /// On a `lockless` filesystem (see [`is_lockless_fs`]) a database is never
    /// read in place: it is read immutable while its journal is empty, and
    /// copied with it otherwise.
    fn of(path: &Path, lockless: bool) -> io::Result<Mode> {
        let wal = is_wal(path)?;
        if !lockless && (!wal || has_sidecars(path)) {
            return Ok(Mode::InPlace);
        }
        Ok(if journal_len(path, wal)? == 0 {
            Mode::Immutable
        } else {
            Mode::Snapshot
        })
    }
}

/// Opens a SQLite database for reading without ever writing next to it.
///
/// A rollback-journal database, and a WAL database that another process
/// (Cursor) has open, are opened in place with `SQLITE_OPEN_READ_ONLY`, a busy
/// timeout and `PRAGMA query_only`, so reads coexist with writers and see
/// every commit. A WAL database closed cleanly is opened `immutable`: SQLite
/// would otherwise create `-wal` and `-shm` next to it to read it, or fail in a
/// read-only directory. One whose `-wal` a crash left behind is copied, with
/// that `-wal`, to a private temporary directory and the copy is opened; so is
/// one whose path a `file:` URI cannot carry, unless it has both sidecars. A
/// database on a filesystem shared with another machine or VM, or mounted
/// read-only, is never read in place (see [`Mode::of`]). With `copy`, the
/// database is read from a copy that SQLite may write to, to roll back a
/// journal a crash left.
fn open_readonly(path: &Path, copy: bool) -> Result<Db> {
    let file = sqlite_path(path).map_err(|source| Error::access(path, source))?;
    let lockless = is_lockless_fs(&file);
    for _ in 0..3 {
        let before = stamp(&file).map_err(|source| Error::access(path, source))?;
        let mode = if copy {
            Mode::Snapshot
        } else {
            Mode::of(&file, lockless).map_err(|source| Error::access(path, source))?
        };
        let uri = match mode {
            Mode::InPlace => break,
            Mode::Immutable => immutable_uri(&file),
            Mode::Snapshot => None,
        };
        if let Some(uri) = uri {
            let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
            let conn = connect(Path::new(&uri), path, flags)?;
            return Ok(Db {
                conn,
                immutable: Some((file, before)),
                _snapshot: None,
            });
        }
        // With both sidecars present, reading in place creates no files.
        if mode == Mode::Immutable && !lockless && has_sidecars(&file) {
            break;
        }
        let snapshot = Snapshot::copy(&file, path)?;
        #[cfg(test)]
        AFTER_COPY.with_borrow_mut(|hook| hook.as_mut().map(|hook| hook(&file)));
        // Cursor may have opened the file during the copy, which can tear it.
        // Copy again, or read in place once its -wal and -shm exist.
        if stamp(&file).ok() == Some(before)
            && (copy || Mode::of(&file, lockless).ok() == Some(mode))
        {
            // The copy is private, so SQLite may recover it.
            let conn = connect(&snapshot.db, path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            return Ok(Db {
                conn,
                immutable: None,
                _snapshot: Some(snapshot),
            });
        }
    }
    if lockless || copy {
        return Err(Error::Changed {
            path: path.to_path_buf(),
        });
    }
    Ok(Db {
        conn: connect(&file, path, OpenFlags::SQLITE_OPEN_READ_ONLY)?,
        immutable: None,
        _snapshot: None,
    })
}

fn connect(file: &Path, path: &Path, flags: OpenFlags) -> Result<Connection> {
    let db_err = |source| Error::Database {
        path: path.to_path_buf(),
        source,
    };
    let conn = Connection::open_with_flags(file, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(db_err)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(db_err)?;
    conn.pragma_update(None, "query_only", true)
        .map_err(db_err)?;
    Ok(conn)
}

/// The path SQLite derives the `-wal` and `-shm` names from. Its Unix VFS
/// resolves symlinks first and its Windows VFS does not. Being absolute, it
/// never starts with `file:`, so the bundled SQLite (built with
/// SQLITE_USE_URI) cannot mistake it for a URI.
fn sqlite_path(path: &Path) -> io::Result<PathBuf> {
    if cfg!(unix) {
        fs::canonicalize(path)
    } else {
        std::path::absolute(path)
    }
}

#[cfg(test)]
thread_local! {
    /// Overrides [`is_lockless_fs`] on this thread.
    static LOCKLESS: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    /// Runs on this thread after each copy, before the original is checked again.
    #[allow(clippy::type_complexity)]
    static AFTER_COPY: std::cell::RefCell<Option<Box<dyn FnMut(&Path)>>> =
        const { std::cell::RefCell::new(None) };
}

/// Whether reading `path` in place cannot rely on SQLite's locks and the
/// shared memory (`-shm`) of WAL mode. On a filesystem that another machine or
/// VM may write to at the same time, such as a network share or the Windows
/// drives WSL mounts, they do not reach across. On macOS, SQLite opens a
/// database on a read-only filesystem without locks and so without a `-shm`,
/// which a WAL database cannot be read in place without. Elsewhere SQLite reads
/// such a database in place with a read-only `-shm`, which also keeps it in
/// step with a writer that reaches the same files through another mount, as a
/// read-only bind mount into a container does.
fn is_lockless_fs(path: &Path) -> bool {
    #[cfg(test)]
    if let Some(lockless) = LOCKLESS.get() {
        return lockless;
    }
    lockless_fs(path)
}

/// Filesystem types (`statfs` magic numbers) that are served from elsewhere.
#[cfg(any(target_os = "linux", test))]
const SHARED_FS_TYPES: [u32; 19] = [
    0x0102_1997, // 9p: WSL 2's /mnt/c, VM shares
    0x5346_4846, // WSL 1
    0xFF53_4D42, // CIFS
    0xFE53_4D42, // SMB 2
    0x0000_517B, // SMB
    0x0000_6969, // NFS
    0x6573_5546, // FUSE: sshfs, virtiofs, rclone, vmhgfs-fuse
    0x5346_414F, // AFS
    0x6B41_4653, // kAFS
    0x00C3_6400, // Ceph
    0x786F_4256, // VirtualBox shared folders
    0x7C7C_6673, // Parallels shared folders
    0xBACB_ACBC, // VMware shared folders
    0x4750_4653, // GPFS
    0x7375_7245, // Coda
    0x0BD0_0BD0, // Lustre
    0x7461_636F, // OCFS2
    0x0116_1970, // GFS2
    0x0000_564C, // NCP
];

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn lockless_fs(path: &Path) -> bool {
    c_path(path)
        .and_then(|path| fs_stat(&path))
        .is_some_and(|stat| is_lockless(&stat))
}

/// Network filesystems lack `MNT_LOCAL`; read-only ones have `MNT_RDONLY`,
/// for which SQLite's macOS locking-style finder chooses no locks at all.
#[cfg(target_os = "macos")]
fn is_lockless(stat: &libc::statfs) -> bool {
    let flags = stat.f_flags;
    flags & (libc::MNT_LOCAL as u32) == 0 || flags & (libc::MNT_RDONLY as u32) != 0
}

#[cfg(target_os = "linux")]
fn is_lockless(stat: &libc::statfs) -> bool {
    // The field's type differs between targets; the magic numbers fit 32 bits.
    is_shared_fs_type(stat.f_type as u32)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn c_path(path: &Path) -> Option<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).ok()
}

/// What `statfs` reports for `path`.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fs_stat(path: &std::ffi::CStr) -> Option<libc::statfs> {
    let mut buf = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `buf` is valid for writes of a statfs.
    if unsafe { libc::statfs(path.as_ptr(), buf.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the call succeeded, so it filled `buf`.
    Some(unsafe { buf.assume_init() })
}

#[cfg(any(target_os = "linux", test))]
fn is_shared_fs_type(magic: u32) -> bool {
    SHARED_FS_TYPES.contains(&magic)
}

#[cfg(windows)]
fn lockless_fs(path: &Path) -> bool {
    path.to_str().is_some_and(is_unc)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn lockless_fs(_: &Path) -> bool {
    false
}

/// Whether a Windows path names a network share (`\\server\share`,
/// `\\wsl$\...`), also in its verbatim form (`\\?\UNC\...`).
#[cfg(any(windows, test))]
fn is_unc(path: &str) -> bool {
    let path = path.replace('/', "\\");
    match path.strip_prefix(r"\\?\") {
        Some(verbatim) => verbatim
            .get(..4)
            .is_some_and(|unc| unc.eq_ignore_ascii_case(r"UNC\")),
        None => path.starts_with(r"\\") && !path.starts_with(r"\\.\"),
    }
}

/// `path` as a `file:` URI that opens it `immutable`, or `None` when a URI
/// cannot name it safely.
#[cfg(unix)]
fn immutable_uri(path: &Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;
    unix_uri(path.as_os_str().as_bytes())
}

#[cfg(windows)]
fn immutable_uri(path: &Path) -> Option<String> {
    windows_uri(path.to_str()?)
}

#[cfg(not(any(unix, windows)))]
fn immutable_uri(_: &Path) -> Option<String> {
    None
}

/// `/dir/state.vscdb` as `file:///dir/state.vscdb?immutable=1`, from the raw
/// bytes of an absolute path.
#[cfg(any(unix, test))]
fn unix_uri(path: &[u8]) -> Option<String> {
    if path.first() != Some(&b'/') {
        return None;
    }
    let mut uri = String::from("file://");
    push_encoded(&mut uri, path);
    uri.push_str("?immutable=1");
    Some(uri)
}

/// `C:\dir\state.vscdb` as `file:///C:/dir/state.vscdb?immutable=1`. UNC
/// (`\\server\share`) and verbatim (`\\?\`) paths give `None`.
#[cfg(any(windows, test))]
fn windows_uri(path: &str) -> Option<String> {
    let (drive, rest) = path.split_at_checked(3)?;
    let [letter, b':', b'\\' | b'/'] = drive.as_bytes() else {
        return None;
    };
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    let mut uri = format!("file:///{}:/", char::from(*letter));
    push_encoded(&mut uri, rest.replace('\\', "/").as_bytes());
    uri.push_str("?immutable=1");
    Some(uri)
}

/// Appends `path`, percent-encoding every byte outside `[A-Za-z0-9-._~/]`. In
/// a URI, SQLite ends the path at `?` or `#` and decodes `%HH`.
#[cfg(any(unix, windows, test))]
fn push_encoded(uri: &mut String, path: &[u8]) {
    for &byte in path {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
}

fn stamp(path: &Path) -> io::Result<Stamp> {
    let meta = fs::metadata(path)?;
    Ok((meta.len(), meta.modified().ok()))
}

/// Reads the SQLite header's file format bytes, which are 2 in WAL mode.
fn is_wal(path: &Path) -> io::Result<bool> {
    let mut header = [0u8; 20];
    match File::open(path)?.read_exact(&mut header) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
        Err(err) => return Err(err),
    }
    Ok(header.starts_with(b"SQLite format 3\0") && (header[18] == 2 || header[19] == 2))
}

fn has_sidecars(path: &Path) -> bool {
    sidecar(path, "-wal").is_file() && sidecar(path, "-shm").is_file()
}

/// The size of the `-wal` or `-journal` next to `path`; 0 when there is none.
fn journal_len(path: &Path, wal: bool) -> io::Result<u64> {
    match fs::metadata(sidecar(path, journal_suffix(wal))) {
        Ok(meta) => Ok(meta.len()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err),
    }
}

fn journal_suffix(wal: bool) -> &'static str {
    if wal { "-wal" } else { "-journal" }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
thread_local! {
    /// Snapshots made on this thread.
    static COPIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A private temporary copy of a database, removed on drop.
struct Snapshot {
    dir: PathBuf,
    db: PathBuf,
}

impl Snapshot {
    /// Copies `file`, the database at `path`, with its `-wal` or `-journal`.
    /// A file that cannot be read is named as such; only failures to write
    /// the copy are [`Error::Snapshot`].
    fn copy(file: &Path, path: &Path) -> Result<Self> {
        #[cfg(test)]
        COPIES.with(|copies| copies.set(copies.get() + 1));
        let suffix = journal_suffix(is_wal(file).map_err(|source| Error::access(path, source))?);
        let mut sources = vec![(
            "",
            File::open(file).map_err(|source| Error::access(path, source))?,
        )];
        let journal = sidecar(file, suffix);
        match File::open(&journal) {
            Ok(opened) => sources.push((suffix, opened)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::access(&journal, source)),
        }

        let snapshot_err = |source| Error::Snapshot {
            path: path.to_path_buf(),
            source,
        };
        let dir = private_temp_dir().map_err(snapshot_err)?;
        let snapshot = Snapshot {
            db: dir.join(file.file_name().unwrap_or_else(|| "db".as_ref())),
            dir,
        };
        for (suffix, mut source) in sources {
            let mut copy = File::create(sidecar(&snapshot.db, suffix)).map_err(snapshot_err)?;
            io::copy(&mut source, &mut copy).map_err(snapshot_err)?;
        }
        Ok(snapshot)
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn private_temp_dir() -> io::Result<PathBuf> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    create_private_dir(&std::env::temp_dir(), &COUNTER)
}

/// Creates a new directory in `temp` that only this user can enter. A name
/// that is taken, by a directory, a file or a symlink, is never reused.
fn create_private_dir(temp: &Path, counter: &AtomicUsize) -> io::Result<PathBuf> {
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
    loop {
        let n = counter.fetch_add(1, Ordering::Relaxed);
        let dir = temp.join(format!("{SNAPSHOT_PREFIX}{}-{n}", std::process::id()));
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && n < 1000 => continue,
            Err(err) => return Err(err),
        }
    }
}

/// Removes the copies that killed processes (e.g. by Ctrl-C) left in the
/// temporary directory. The binary runs this once at startup; the library
/// never does on its own.
pub fn remove_stale_snapshot_copies() {
    remove_stale_snapshots(&std::env::temp_dir(), STALE_SNAPSHOT);
}

/// Removes snapshot directories that a killed process (e.g. by Ctrl-C) left
/// in `temp`. No read holds a snapshot for anywhere near `max_age`.
fn remove_stale_snapshots(temp: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(temp) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let ours = name
            .to_str()
            .and_then(|name| name.strip_prefix(SNAPSHOT_PREFIX))
            .and_then(|rest| rest.split_once('-'))
            .is_some_and(|(pid, n)| is_number(pid) && is_number(n));
        if !ours {
            continue;
        }
        // `DirEntry` metadata does not follow symlinks.
        let stale = entry.metadata().is_ok_and(|meta| {
            meta.is_dir()
                && meta
                    .modified()
                    .ok()
                    .and_then(|time| time.elapsed().ok())
                    .is_some_and(|age| age >= max_age)
        });
        if stale {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(dir: &Path) -> Vec<(String, Vec<u8>, Option<SystemTime>)> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let meta = fs::metadata(&path).unwrap();
                (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    fs::read(&path).unwrap(),
                    meta.modified().ok(),
                )
            })
            .collect();
        entries.sort();
        entries
    }

    fn create(path: &Path, journal_mode: &str) {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "journal_mode", journal_mode)
            .unwrap();
        conn.execute_batch(
            "CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
             INSERT INTO cursorDiskKV VALUES ('k', 'v');",
        )
        .unwrap();
    }

    /// Opens `path` for writing and commits a second row that stays in the -wal.
    fn wal_writer(path: &Path) -> Connection {
        let writer = Connection::open(path).unwrap();
        writer.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        writer
            .execute("INSERT INTO cursorDiskKV VALUES ('k2', 'v2')", [])
            .unwrap();
        assert!(fs::metadata(sidecar(path, "-wal")).unwrap().len() > 0);
        writer
    }

    fn db_err(source: rusqlite::Error) -> Error {
        Error::Database {
            path: PathBuf::new(),
            source,
        }
    }

    fn rows(conn: &Connection) -> Result<Vec<(String, String)>> {
        let mut stmt = conn
            .prepare("SELECT key, value FROM cursorDiskKV ORDER BY key")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    fn copies() -> usize {
        COPIES.with(std::cell::Cell::get)
    }

    #[test]
    fn reading_a_closed_database_leaves_its_directory_untouched() {
        for (journal_mode, empty_wal) in [("delete", false), ("wal", false), ("wal", true)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("state.vscdb");
            create(&path, journal_mode);
            // A clean close removes the WAL sidecars, as when Cursor quits.
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
            if empty_wal {
                // Persistent WAL mode truncates the -wal instead.
                fs::write(sidecar(&path, "-wal"), "").unwrap();
            }
            assert_eq!(is_wal(&path).unwrap(), journal_mode == "wal");

            let before = listing(dir.path());
            let copied = copies();
            assert_eq!(with_readonly(&path, rows).unwrap().len(), 1);
            assert_eq!(copies(), copied, "{journal_mode}");
            assert_eq!(listing(dir.path()), before, "{journal_mode}");
        }
    }

    #[test]
    fn closed_wal_database_is_read_immutable_without_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create(&path, "wal");
        let copied = copies();
        let db = open_readonly(&path, false).unwrap();
        assert!(db.immutable.is_some());
        assert!(db._snapshot.is_none());
        assert_eq!(copies(), copied);
        assert!(db.execute("DELETE FROM cursorDiskKV", []).is_err());
        assert_eq!(rows(&db).unwrap(), [("k".to_string(), "v".to_string())]);
    }

    #[test]
    fn live_wal_database_is_read_in_place_with_its_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create(&path, "wal");
        let _writer = wal_writer(&path);
        // The second row is only in the -wal: the main file alone has one.
        let uri = immutable_uri(&sqlite_path(&path).unwrap()).unwrap();
        let main_only = connect(
            Path::new(&uri),
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap();
        assert_eq!(rows(&main_only).unwrap().len(), 1);

        let copied = copies();
        let db = open_readonly(&path, false).unwrap();
        assert!(db.immutable.is_none());
        assert!(db._snapshot.is_none());
        assert_eq!(copies(), copied);
        assert_eq!(rows(&db).unwrap().len(), 2);
    }

    #[test]
    fn open_wal_database_with_an_emptied_wal_is_read_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create(&path, "wal");
        // Cursor keeps the database open after a checkpoint that empties the -wal.
        let writer = wal_writer(&path);
        writer
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        assert_eq!(fs::metadata(sidecar(&path, "-wal")).unwrap().len(), 0);
        assert!(has_sidecars(&path));

        let names = |dir: &Path| -> Vec<String> {
            listing(dir).into_iter().map(|(name, ..)| name).collect()
        };
        let before = names(dir.path());
        let copied = copies();
        let db = open_readonly(&path, false).unwrap();
        assert!(db.immutable.is_none());
        assert!(db._snapshot.is_none());
        assert_eq!(copies(), copied);
        assert_eq!(rows(&db).unwrap().len(), 2);
        drop(db);
        assert_eq!(names(dir.path()), before);
    }

    #[test]
    fn last_close_leaves_the_wal_of_a_cursor_that_quit_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create(&path, "wal");
        let writer = wal_writer(&path);
        let db = open_readonly(&path, false).unwrap();
        assert!(db.immutable.is_none());
        assert_eq!(rows(&db).unwrap().len(), 2);
        let contents = |suffix| fs::read(sidecar(&path, suffix)).unwrap();
        let (main, wal) = (contents(""), contents("-wal"));

        // Cursor quits after the read started; ours is then the last connection,
        // which must not checkpoint into the database or remove its sidecars.
        drop(writer);
        assert_eq!(rows(&db).unwrap().len(), 2);
        drop(db);
        assert!(has_sidecars(&path));
        assert_eq!(contents(""), main);
        assert_eq!(contents("-wal"), wal);
    }

    #[test]
    fn wal_left_by_a_crash_is_read_from_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.vscdb");
        create(&live, "wal");
        let writer = wal_writer(&live);
        // What a crash leaves behind: the database and its -wal, but no -shm.
        // (Windows' fs::copy cannot open a file another handle is writing.)
        let crashed = dir.path().join("crashed");
        fs::create_dir(&crashed).unwrap();
        let path = crashed.join("state.vscdb");
        for suffix in ["", "-wal"] {
            let data = fs::read(sidecar(&live, suffix)).unwrap();
            fs::write(sidecar(&path, suffix), data).unwrap();
        }
        drop(writer);

        let before = listing(&crashed);
        let copied = copies();
        let db = open_readonly(&path, false).unwrap();
        assert_eq!(copies(), copied + 1);
        let copy = db._snapshot.as_ref().map(|s| s.dir.clone()).unwrap();
        assert!(copy.join("state.vscdb").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&copy).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert_eq!(rows(&db).unwrap().len(), 2);
        drop(db);
        assert!(!copy.exists());
        assert_eq!(listing(&crashed), before);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_wal_database_sees_commits_in_the_wal() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let path = real.join("state.vscdb");
        create(&path, "wal");
        let link = dir.path().join("state.vscdb");
        std::os::unix::fs::symlink(&path, &link).unwrap();

        let _writer = wal_writer(&path);
        let db = open_readonly(&link, false).unwrap();
        assert!(db._snapshot.is_none());
        assert_eq!(rows(&db).unwrap().len(), 2);
    }

    #[test]
    fn reserved_characters_in_paths_reach_sqlite_intact() {
        let dir = tempfile::tempdir().unwrap();
        // Unencoded, `#` and `?` would end the path and `%41` would read as `A`.
        let name = if cfg!(windows) {
            "a b#c%41 ü 名"
        } else {
            "a b#c?d%41 ü 名"
        };
        let parent = dir.path().join(name);
        fs::create_dir(&parent).unwrap();
        let path = parent.join("state #1.vscdb");
        create(&path, "wal");

        let before = listing(&parent);
        let db = open_readonly(&path, false).unwrap();
        assert!(db.immutable.is_some());
        assert_eq!(rows(&db).unwrap().len(), 1);
        drop(db);
        assert_eq!(listing(&parent), before);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_paths_reach_sqlite_intact() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
        fs::create_dir(&parent).unwrap();
        let path = parent.join("state.vscdb");
        create(&path, "wal");
        let db = open_readonly(&path, false).unwrap();
        assert!(db.immutable.is_some());
        assert_eq!(rows(&db).unwrap().len(), 1);
    }

    #[test]
    fn uris_percent_encode_reserved_bytes() {
        assert_eq!(
            unix_uri(b"/a b/#?%/\xff\xc3\xbc/A-z_0.9~/x").as_deref(),
            Some("file:///a%20b/%23%3F%25/%FF%C3%BC/A-z_0.9~/x?immutable=1")
        );
        assert_eq!(unix_uri(b"relative/state.vscdb"), None);
        assert_eq!(
            windows_uri(r"C:\Users\a b\#1%\state.vscdb").as_deref(),
            Some("file:///C:/Users/a%20b/%231%25/state.vscdb?immutable=1")
        );
        assert_eq!(
            windows_uri("d:/Cursor/ü").as_deref(),
            Some("file:///d:/Cursor/%C3%BC?immutable=1")
        );
        for path in [
            r"\\server\share\state.vscdb",
            r"\\?\C:\state.vscdb",
            r"\\.\C:\state.vscdb",
            r"C:state.vscdb",
            r"1:\state.vscdb",
            "C:",
        ] {
            assert_eq!(windows_uri(path), None, "{path}");
        }
    }

    #[test]
    fn immutable_read_runs_again_when_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create(&path, "wal");
        let mut calls = 0;
        let read = with_readonly(&path, |conn| {
            calls += 1;
            let found = rows(conn)?;
            if calls == 1 {
                // Cursor starts, then writes and checkpoints during the read.
                let writer = Connection::open(&path).unwrap();
                writer
                    .execute(
                        "INSERT INTO cursorDiskKV VALUES ('k2', printf('%.*c', 65536, 'x'))",
                        [],
                    )
                    .unwrap();
                writer
                    .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                    .unwrap();
            }
            Ok(found)
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(read.len(), 2);
    }

    #[test]
    fn a_file_that_changes_under_both_reads_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create(&path, "wal");
        let mut calls = 0;
        let read = with_readonly(&path, |conn| {
            calls += 1;
            let found = rows(conn)?;
            // Cursor starts, writes, checkpoints and quits during each read.
            let writer = Connection::open(&path).unwrap();
            writer
                .execute(
                    "INSERT INTO cursorDiskKV VALUES (?1, printf('%.*c', 65536, 'x'))",
                    [format!("k{calls}")],
                )
                .unwrap();
            writer
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                .unwrap();
            Ok(found)
        });
        assert_eq!(calls, 2);
        let err = read.unwrap_err();
        assert!(
            matches!(&err, Error::Changed { path: p } if *p == path),
            "{err}"
        );
    }

    #[test]
    fn only_an_immutable_read_that_looks_corrupt_runs_again() {
        let dir = tempfile::tempdir().unwrap();
        for (journal_mode, expected) in [("wal", 2), ("delete", 1)] {
            let path = dir.path().join(format!("{journal_mode}.vscdb"));
            create(&path, journal_mode);
            let mut calls = 0;
            let result: Result<()> = with_readonly(&path, |_| {
                calls += 1;
                Err(db_err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                    None,
                )))
            });
            assert!(matches!(result, Err(Error::Database { .. })));
            assert_eq!(calls, expected, "{journal_mode}");
        }
    }

    /// Runs `open` as if the database were on a filesystem another machine shares.
    fn shared<T>(open: impl FnOnce() -> T) -> T {
        LOCKLESS.set(Some(true));
        let opened = open();
        LOCKLESS.set(None);
        opened
    }

    #[test]
    fn a_database_on_a_shared_filesystem_is_never_read_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create(&path, "wal");
        // Across the share, the locks and -shm of this writer are not there to see.
        let writer = wal_writer(&path);
        let contents = |suffix| fs::read(sidecar(&path, suffix)).unwrap();
        let (main, wal) = (contents(""), contents("-wal"));

        let copied = copies();
        let db = shared(|| open_readonly(&path, false)).unwrap();
        assert!(db._snapshot.is_some());
        assert_eq!(copies(), copied + 1);
        assert_eq!(rows(&db).unwrap().len(), 2);
        drop(db);
        assert_eq!((contents(""), contents("-wal")), (main, wal));

        // An emptied -wal holds no commit: the main file is read without locks.
        writer
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        let db = shared(|| open_readonly(&path, false)).unwrap();
        assert!(db.immutable.is_some());
        assert_eq!(copies(), copied + 1);
        assert_eq!(rows(&db).unwrap().len(), 2);
    }

    /// What a crash in the middle of a write leaves of a rollback-journal
    /// database: a journal that only a writer may roll back, and a main file
    /// that already holds part of the write.
    fn crashed_write(dir: &Path) -> PathBuf {
        let live = dir.join("live.db");
        create(&live, "delete");
        let writer = Connection::open(&live).unwrap();
        writer
            .execute_batch(
                "PRAGMA cache_size = 10; PRAGMA cache_spill = 10; BEGIN;
                 UPDATE cursorDiskKV SET value = 'changed';",
            )
            .unwrap();
        for n in 0..100 {
            writer
                .execute(
                    "INSERT INTO cursorDiskKV VALUES (?1, randomblob(4000))",
                    [format!("filler{n}")],
                )
                .unwrap();
        }
        let crashed = dir.join("crashed");
        fs::create_dir(&crashed).unwrap();
        let path = crashed.join("store.db");
        for suffix in ["", "-journal"] {
            let data = fs::read(sidecar(&live, suffix)).unwrap();
            fs::write(sidecar(&path, suffix), data).unwrap();
        }
        path
    }

    #[test]
    fn a_journal_left_by_a_crash_is_rolled_back_in_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = crashed_write(dir.path());
        let crashed = path.parent().unwrap();
        let before = listing(crashed);
        let committed = [("k".to_string(), "v".to_string())];

        let copied = copies();
        assert_eq!(with_readonly(&path, rows).unwrap(), committed);
        assert_eq!(copies(), copied + 1);
        assert_eq!(listing(crashed), before);

        // Across a share a journal may belong to a write in progress there.
        let db = shared(|| open_readonly(&path, false)).unwrap();
        assert!(db._snapshot.is_some());
        assert_eq!(rows(&db).unwrap(), committed);
        // Without a journal there is nothing to roll back, nor to copy.
        let clean = dir.path().join("clean.db");
        create(&clean, "delete");
        let db = shared(|| open_readonly(&clean, false)).unwrap();
        assert!(db.immutable.is_some());
        assert_eq!(rows(&db).unwrap(), committed);
    }

    /// The crash-left WAL database of [`wal_left_by_a_crash_is_read_from_a_copy`].
    fn crashed_wal(dir: &Path) -> PathBuf {
        let live = dir.join("live.vscdb");
        create(&live, "wal");
        let writer = wal_writer(&live);
        let crashed = dir.join("crashed");
        fs::create_dir(&crashed).unwrap();
        let path = crashed.join("state.vscdb");
        for suffix in ["", "-wal"] {
            let data = fs::read(sidecar(&live, suffix)).unwrap();
            fs::write(sidecar(&path, suffix), data).unwrap();
        }
        drop(writer);
        path
    }

    fn after_copy(hook: impl FnMut(&Path) + 'static) {
        AFTER_COPY.set(Some(Box::new(hook)));
    }

    fn touch(path: &Path) {
        let later = SystemTime::now() + Duration::from_secs(10);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(later)
            .unwrap();
    }

    #[test]
    fn a_copy_that_cursor_may_have_torn_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = crashed_wal(dir.path());

        // Cursor opens the database during the copy: read it in place with it.
        after_copy(|file| fs::write(sidecar(file, "-shm"), "").unwrap());
        let copied = copies();
        let db = open_readonly(&path, false).unwrap();
        assert!(db._snapshot.is_none() && db.immutable.is_none());
        assert_eq!(copies(), copied + 1);
        assert_eq!(rows(&db).unwrap().len(), 2);
        drop(db);
        fs::remove_file(sidecar(&path, "-shm")).unwrap();

        // The main file changes during the first copy only: copy it again.
        let mut calls = 0;
        after_copy(move |file| {
            calls += 1;
            if calls == 1 {
                touch(file);
            }
        });
        let copied = copies();
        let db = open_readonly(&path, false).unwrap();
        assert!(db._snapshot.is_some());
        assert_eq!(copies(), copied + 2);
        assert_eq!(rows(&db).unwrap().len(), 2);
        drop(db);

        // It keeps changing: across a share there is no reading it in place.
        after_copy(touch);
        let err = shared(|| open_readonly(&path, false)).err().unwrap();
        assert!(
            matches!(&err, Error::Changed { path: p } if *p == path),
            "{err}"
        );
        assert!(open_readonly(&path, false).unwrap()._snapshot.is_none());
        AFTER_COPY.set(None);
    }

    #[cfg(unix)]
    #[test]
    fn a_journal_that_cannot_be_read_is_named_rather_than_the_temporary_directory() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = crashed_wal(dir.path());
        let wal = sidecar(&sqlite_path(&path).unwrap(), "-wal");
        fs::set_permissions(&wal, fs::Permissions::from_mode(0o000)).unwrap();
        if File::open(&wal).is_ok() {
            eprintln!("skipped: permissions are not enforced for this user (root)");
            return;
        }
        let err = open_readonly(&path, false).err().unwrap();
        assert!(
            matches!(&err, Error::Io { path: p, .. } if *p == wal),
            "{err}"
        );
    }

    #[test]
    fn copies_go_to_a_new_directory_only_this_user_can_enter() {
        let temp = tempfile::tempdir().unwrap();
        let name = |n: usize| {
            temp.path()
                .join(format!("{SNAPSHOT_PREFIX}{}-{n}", std::process::id()))
        };
        // Names already taken, as another user could take them in a shared /tmp.
        fs::create_dir(name(0)).unwrap();
        fs::write(name(0).join("planted"), "").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(temp.path().join("elsewhere"), name(1)).unwrap();
        #[cfg(not(unix))]
        fs::write(name(1), "").unwrap();

        let dir = create_private_dir(temp.path(), &AtomicUsize::new(0)).unwrap();
        assert_eq!(dir, name(2));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        assert!(name(0).join("planted").is_file());
        assert!(!temp.path().join("elsewhere").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn network_paths_are_shared() {
        for path in [
            r"\\server\share\Cursor\state.vscdb",
            r"\\wsl$\Ubuntu\home\dana\.cursor\chats",
            r"\\?\UNC\server\share\state.vscdb",
            "//server/share/state.vscdb",
        ] {
            assert!(is_unc(path), "{path}");
        }
        for path in [
            r"C:\Users\dana\AppData\Roaming\Cursor",
            r"\\?\C:\Users\dana",
            r"\\.\C:\state.vscdb",
            "relative",
        ] {
            assert!(!is_unc(path), "{path}");
        }
        assert!(!is_lockless_fs(tempfile::tempdir().unwrap().path()));
        // WSL 2's /mnt/c, an SMB share and a Parallels shared folder are; ext4,
        // btrfs and tmpfs are not.
        for shared in [0x0102_1997, 0xFF53_4D42, 0x7C7C_6673] {
            assert!(is_shared_fs_type(shared));
        }
        for local in [0xEF53, 0x9123_683E, 0x0102_1994] {
            assert!(!is_shared_fs_type(local));
        }
        #[cfg(windows)]
        assert!(lockless_fs(Path::new(r"\\server\share\state.vscdb")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn network_and_read_only_mounts_are_lockless() {
        let mount = |flags: libc::c_int| {
            // SAFETY: statfs is plain data, for which all zeros is valid.
            let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
            stat.f_flags = flags as u32;
            is_lockless(&stat)
        };
        assert!(!mount(libc::MNT_LOCAL));
        assert!(mount(0));
        assert!(mount(libc::MNT_LOCAL | libc::MNT_RDONLY));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn network_mounts_are_lockless() {
        let mount = |fs_type: u32| {
            // SAFETY: statfs is plain data, for which all zeros is valid.
            let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
            stat.f_type = fs_type as _;
            is_lockless(&stat)
        };
        // ext4, and 9p (WSL 2's /mnt/c).
        assert!(!mount(0xEF53));
        assert!(mount(0x0102_1997));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn mounts_are_told_apart_by_what_the_system_reports() {
        let dir = tempfile::tempdir().unwrap();
        let stat = fs_stat(&c_path(dir.path()).unwrap());
        assert!(stat.is_some_and(|stat| !is_lockless(&stat)));
        // macOS mounts its sealed system volume read-only.
        #[cfg(target_os = "macos")]
        assert!(is_lockless_fs(Path::new("/")));
    }

    #[test]
    fn stale_snapshots_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let names = [
            "cursor-session-123-0",
            "cursor-session-123",
            "cursor-session-x-0",
            "other-1-0",
        ];
        for name in names {
            fs::create_dir(dir.path().join(name)).unwrap();
        }
        fs::write(dir.path().join("cursor-session-1-0"), "a file").unwrap();

        remove_stale_snapshots(dir.path(), STALE_SNAPSHOT);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 5);
        remove_stale_snapshots(dir.path(), Duration::ZERO);
        assert!(!dir.path().join(names[0]).exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
    }

    #[test]
    fn writes_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create(&path, "delete");
        let db = open_readonly(&path, false).unwrap();
        assert!(db.execute("DELETE FROM cursorDiskKV", []).is_err());
    }

    #[test]
    fn missing_file_is_storage_not_found() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            open_readonly(&dir.path().join("state.vscdb"), false),
            Err(Error::StorageNotFound { .. })
        ));
    }
}

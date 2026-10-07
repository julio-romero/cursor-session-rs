use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Once;
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
/// connection, which by then usually reads in place.
pub fn with_readonly<T>(path: &Path, mut read: impl FnMut(&Connection) -> Result<T>) -> Result<T> {
    let db = open_readonly(path)?;
    let result = read(&db);
    if !db.torn(&result) {
        return result;
    }
    drop(db);
    read(&*open_readonly(path)?)
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
        let Some((file, opened)) = &self.immutable else {
            return false;
        };
        let corrupt = matches!(
            result,
            Err(Error::Database { source, .. }) if matches!(
                source.sqlite_error_code(),
                Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
            )
        );
        corrupt || stamp(file).ok().as_ref() != Some(opened)
    }
}

/// How [`open_readonly`] reads a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// SQLite's normal read-only open, sharing locks with writers.
    InPlace,
    /// Only the main file, without locks (`immutable=1`).
    Immutable,
    /// A private copy of the main file and its `-wal`.
    Snapshot,
}

impl Mode {
    /// A WAL database with no `-wal`, or an empty one, has every commit in its
    /// main file. A non-empty `-wal` next to a `-shm` belongs to a writer that
    /// has the database open; without a `-shm`, a crash left it behind.
    fn of(path: &Path) -> io::Result<Mode> {
        if !is_wal(path)? {
            return Ok(Mode::InPlace);
        }
        let wal_len = match fs::metadata(sidecar(path, "-wal")) {
            Ok(meta) => meta.len(),
            Err(err) if err.kind() == io::ErrorKind::NotFound => 0,
            Err(err) => return Err(err),
        };
        Ok(if wal_len == 0 {
            Mode::Immutable
        } else if sidecar(path, "-shm").is_file() {
            Mode::InPlace
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
/// one whose path a `file:` URI cannot carry.
fn open_readonly(path: &Path) -> Result<Db> {
    let file = sqlite_path(path).map_err(|source| Error::access(path, source))?;
    for _ in 0..3 {
        let before = stamp(&file).map_err(|source| Error::access(path, source))?;
        let mode = Mode::of(&file).map_err(|source| Error::access(path, source))?;
        let uri = match mode {
            Mode::InPlace => break,
            Mode::Immutable => immutable_uri(&file),
            Mode::Snapshot => None,
        };
        if let Some(uri) = uri {
            let conn = connect(Path::new(&uri), path, OpenFlags::SQLITE_OPEN_URI)?;
            return Ok(Db {
                conn,
                immutable: Some((file, before)),
                _snapshot: None,
            });
        }
        let snapshot = Snapshot::copy(&file).map_err(|source| Error::Snapshot {
            path: path.to_path_buf(),
            source,
        })?;
        // Cursor may have opened the file during the copy, which can tear it.
        // Copy again, or read in place once its -wal and -shm exist.
        if stamp(&file).ok() == Some(before) && Mode::of(&file).ok() == Some(mode) {
            let conn = connect(&snapshot.db, path, OpenFlags::empty())?;
            return Ok(Db {
                conn,
                immutable: None,
                _snapshot: Some(snapshot),
            });
        }
    }
    Ok(Db {
        conn: connect(&file, path, OpenFlags::empty())?,
        immutable: None,
        _snapshot: None,
    })
}

fn connect(file: &Path, path: &Path, flags: OpenFlags) -> Result<Connection> {
    let db_err = |source| Error::Database {
        path: path.to_path_buf(),
        source,
    };
    let conn = Connection::open_with_flags(
        file,
        flags | OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
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
    fn copy(path: &Path) -> io::Result<Self> {
        #[cfg(test)]
        COPIES.with(|copies| copies.set(copies.get() + 1));
        let dir = private_temp_dir()?;
        let snapshot = Snapshot {
            db: dir.join(path.file_name().unwrap_or_else(|| "db".as_ref())),
            dir,
        };
        fs::copy(path, &snapshot.db)?;
        let wal = sidecar(path, "-wal");
        if wal.is_file() {
            fs::copy(&wal, sidecar(&snapshot.db, "-wal"))?;
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
    static SWEEP: Once = Once::new();
    let temp = std::env::temp_dir();
    SWEEP.call_once(|| remove_stale_snapshots(&temp, STALE_SNAPSHOT));
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
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = temp.join(format!("{SNAPSHOT_PREFIX}{}-{n}", std::process::id()));
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && n < 1000 => continue,
            Err(err) => return Err(err),
        }
    }
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
        // `DirEntry` metadata does not follow symlinks.
        let stale = entry.metadata().is_ok_and(|meta| {
            meta.is_dir()
                && meta
                    .modified()
                    .ok()
                    .and_then(|time| time.elapsed().ok())
                    .is_some_and(|age| age >= max_age)
        });
        if ours && stale {
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
        let db = open_readonly(&path).unwrap();
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
        let main_only = connect(Path::new(&uri), &path, OpenFlags::SQLITE_OPEN_URI).unwrap();
        assert_eq!(rows(&main_only).unwrap().len(), 1);

        let copied = copies();
        let db = open_readonly(&path).unwrap();
        assert!(db.immutable.is_none());
        assert!(db._snapshot.is_none());
        assert_eq!(copies(), copied);
        assert_eq!(rows(&db).unwrap().len(), 2);
    }

    #[test]
    fn wal_left_by_a_crash_is_read_from_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.vscdb");
        create(&live, "wal");
        let writer = wal_writer(&live);
        // What a crash leaves behind: the database and its -wal, but no -shm.
        let crashed = dir.path().join("crashed");
        fs::create_dir(&crashed).unwrap();
        let path = crashed.join("state.vscdb");
        fs::copy(&live, &path).unwrap();
        fs::copy(sidecar(&live, "-wal"), sidecar(&path, "-wal")).unwrap();
        drop(writer);

        let before = listing(&crashed);
        let copied = copies();
        let db = open_readonly(&path).unwrap();
        assert_eq!(copies(), copied + 1);
        let copy = db._snapshot.as_ref().map(|s| s.dir.clone()).unwrap();
        assert!(copy.join("state.vscdb").is_file());
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
        let db = open_readonly(&link).unwrap();
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
        let db = open_readonly(&path).unwrap();
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
        let db = open_readonly(&path).unwrap();
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
        let db = open_readonly(&path).unwrap();
        assert!(db.execute("DELETE FROM cursorDiskKV", []).is_err());
    }

    #[test]
    fn missing_file_is_storage_not_found() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            open_readonly(&dir.path().join("state.vscdb")),
            Err(Error::StorageNotFound { .. })
        ));
    }
}

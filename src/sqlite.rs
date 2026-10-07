use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use rusqlite::{Connection, OpenFlags};

use crate::{Error, Result};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SNAPSHOT_PREFIX: &str = "cursor-session-";
const STALE_SNAPSHOT: Duration = Duration::from_secs(60 * 60);

/// A read-only connection. When the database had to be copied first (see
/// [`open_readonly`]), the copy lives as long as the connection.
pub struct Db {
    // Field order matters: the connection must close before the copy is removed.
    conn: Connection,
    _snapshot: Option<Snapshot>,
}

impl Deref for Db {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.conn
    }
}

/// Opens a SQLite database for reading without ever writing next to it.
///
/// The file is opened with `SQLITE_OPEN_READ_ONLY`, a busy timeout and
/// `PRAGMA query_only`, so reads coexist with Cursor writing to it. A WAL-mode
/// database whose `-wal` or `-shm` file is missing is not open anywhere, and
/// SQLite would have to create those files to read it (or fail when the
/// directory is read-only). Such a file is copied to a private temporary
/// directory and the copy is opened instead.
pub fn open_readonly(path: &Path) -> Result<Db> {
    let abs = sqlite_path(path).map_err(|source| Error::access(path, source))?;
    for _ in 0..3 {
        let before = stamp(&abs).map_err(|source| Error::access(path, source))?;
        if !needs_snapshot(&abs).map_err(|source| Error::access(path, source))? {
            break;
        }
        let snapshot = Snapshot::copy(&abs).map_err(|source| Error::Snapshot {
            path: path.to_path_buf(),
            source,
        })?;
        // Cursor may have opened the file during the copy, which can tear it.
        // Copy again, or read in place once its -wal and -shm exist.
        if stamp(&abs).ok() == Some(before) && needs_snapshot(&abs).unwrap_or(false) {
            let conn = connect(&snapshot.db, path)?;
            return Ok(Db {
                conn,
                _snapshot: Some(snapshot),
            });
        }
    }
    Ok(Db {
        conn: connect(&abs, path)?,
        _snapshot: None,
    })
}

fn connect(file: &Path, path: &Path) -> Result<Connection> {
    let db_err = |source| Error::Database {
        path: path.to_path_buf(),
        source,
    };
    let conn = Connection::open_with_flags(
        file,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
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

fn stamp(path: &Path) -> io::Result<(u64, Option<SystemTime>)> {
    let meta = fs::metadata(path)?;
    Ok((meta.len(), meta.modified().ok()))
}

fn needs_snapshot(path: &Path) -> io::Result<bool> {
    Ok(is_wal(path)? && !(sidecar(path, "-wal").is_file() && sidecar(path, "-shm").is_file()))
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

/// A private temporary copy of a database, removed on drop.
struct Snapshot {
    dir: PathBuf,
    db: PathBuf,
}

impl Snapshot {
    fn copy(path: &Path) -> io::Result<Self> {
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

    fn read_all(path: &Path) -> i64 {
        let db = open_readonly(path).unwrap();
        db.query_row("SELECT count(*) FROM cursorDiskKV", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn reading_leaves_the_directory_untouched() {
        for journal_mode in ["delete", "wal"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("state.vscdb");
            create(&path, journal_mode);
            // A clean close removes the WAL sidecars, as when Cursor quits.
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
            assert_eq!(is_wal(&path).unwrap(), journal_mode == "wal");

            let before = listing(dir.path());
            assert_eq!(read_all(&path), 1);
            assert_eq!(listing(dir.path()), before, "{journal_mode}");
        }
    }

    #[test]
    fn wal_without_sidecars_reads_a_copy_that_is_removed_afterwards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create(&path, "wal");
        let db = open_readonly(&path).unwrap();
        let copy = db._snapshot.as_ref().map(|s| s.dir.clone()).unwrap();
        assert!(copy.join("state.vscdb").is_file());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        drop(db);
        assert!(!copy.exists());

        // While another connection has it open, the database is read in place.
        let writer = Connection::open(&path).unwrap();
        writer
            .execute("INSERT INTO cursorDiskKV VALUES ('k2', 'v2')", [])
            .unwrap();
        let db = open_readonly(&path).unwrap();
        assert!(db._snapshot.is_none());
        let count: i64 = db
            .query_row("SELECT count(*) FROM cursorDiskKV", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
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

        let writer = Connection::open(&path).unwrap();
        writer
            .execute("INSERT INTO cursorDiskKV VALUES ('k2', 'v2')", [])
            .unwrap();
        let db = open_readonly(&link).unwrap();
        assert!(db._snapshot.is_none());
        let count: i64 = db
            .query_row("SELECT count(*) FROM cursorDiskKV", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
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

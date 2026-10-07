use std::io;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::{Error, Result};

pub fn open_readonly(path: &Path) -> Result<Connection> {
    if !path.is_file() {
        return Err(Error::StorageNotFound {
            path: path.to_path_buf(),
            source: io::ErrorKind::NotFound.into(),
        });
    }
    let uri = sqlite_uri(path);
    Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|source| Error::Database {
        path: path.to_path_buf(),
        source,
    })
}

fn sqlite_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let encoded: String = raw
        .bytes()
        .map(|b| match b {
            b'/' | b'-' | b'_' | b'.' | b':' | b'~' => (b as char).to_string(),
            b if b.is_ascii_alphanumeric() => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    if encoded.starts_with('/') {
        format!("file://{encoded}?mode=ro&immutable=1")
    } else {
        format!("file:{encoded}?mode=ro&immutable=1")
    }
}

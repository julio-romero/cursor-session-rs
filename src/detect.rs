use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

use crate::{Error, Result};

#[derive(Debug, Clone, Default)]
pub struct StoragePaths {
    pub chats_dir: Option<PathBuf>,
    pub projects_dir: Option<PathBuf>,
    pub global_storage_db: Option<PathBuf>,
}

impl StoragePaths {
    pub fn detect() -> Result<Self> {
        let home = dirs_home()?;
        Ok(Self::from_home(&home))
    }

    pub fn from_home(home: &Path) -> Self {
        let chats_dir = first_existing_dir(&[
            home.join(".cursor/chats"),
            home.join(".config/cursor/chats"),
        ]);
        let projects_dir = existing_dir(home.join(".cursor/projects"));
        let global_storage_db = first_existing_file(&[
            home.join("Library/Application Support/Cursor/User/globalStorage/state.vscdb"),
            home.join(".config/Cursor/User/globalStorage/state.vscdb"),
        ]);
        Self {
            chats_dir,
            projects_dir,
            global_storage_db,
        }
    }

    pub fn from_custom(path: &Path, home: Option<&Path>) -> Result<Self> {
        let path = path
            .canonicalize()
            .map_err(|source| Error::StorageNotFound {
                path: path.to_path_buf(),
                source,
            })?;
        let mut paths = match home {
            Some(home) => Self::from_home(home),
            None => Self::detect().unwrap_or_default(),
        };

        if path.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "state.vscdb" {
                paths.global_storage_db = Some(path);
                return Ok(paths);
            }
            if name == "store.db" {
                let session_dir = path.parent().unwrap_or(&path);
                let chats_root = session_dir
                    .parent()
                    .and_then(|p| p.parent())
                    .unwrap_or(session_dir);
                paths.chats_dir = Some(chats_root.to_path_buf());
                return Ok(paths);
            }
            return Err(Error::UnsupportedStorage { path });
        }

        if path.join("state.vscdb").is_file() {
            paths.global_storage_db = Some(path.join("state.vscdb"));
        }
        if path.join("meta.json").is_file() {
            let chats_root = path.parent().and_then(|p| p.parent()).unwrap_or(&path);
            paths.chats_dir = Some(chats_root.to_path_buf());
        } else if dir_contains_store_or_meta(&path) {
            paths.chats_dir = Some(path);
        }
        Ok(paths)
    }

    pub fn has_agent_storage(&self) -> bool {
        self.chats_dir.as_ref().is_some_and(|p| p.is_dir())
    }

    pub fn has_ide_storage(&self) -> bool {
        self.global_storage_db.as_ref().is_some_and(|p| p.is_file())
    }
}

pub fn workspace_md5(path: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(path.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn dirs_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .ok_or(Error::NoHome)
}

fn existing_dir(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
}

fn first_existing_dir(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_dir()).cloned()
}

fn first_existing_file(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

fn dir_contains_store_or_meta(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join("meta.json").is_file() || path.join("store.db").is_file() {
            return true;
        }
        if path.is_dir() {
            let Ok(inner) = std::fs::read_dir(&path) else {
                continue;
            };
            if inner.flatten().any(|child| {
                let child_path = child.path();
                child_path.join("meta.json").is_file() || child_path.join("store.db").is_file()
            }) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_absolute_workspace_path() {
        assert_eq!(
            workspace_md5("/Users/manuel.romero"),
            "a08c4602b56d190715ccec0647aa6db9"
        );
    }
}

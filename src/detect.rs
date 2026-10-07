use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

use crate::{Error, Result};

#[derive(Debug, Clone, Default)]
pub struct StoragePaths {
    /// Agent CLI chats, at the level `chats_scope` says.
    pub chats_dir: Option<PathBuf>,
    pub chats_scope: ChatsScope,
    pub projects_dir: Option<PathBuf>,
    pub global_storage_db: Option<PathBuf>,
}

/// Which part of the agent chats tree `chats_dir` points at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChatsScope {
    /// The chats root, holding `<workspace hash>/<session id>/` directories.
    #[default]
    All,
    /// One workspace directory.
    Workspace,
    /// One session directory.
    Session,
}

impl ChatsScope {
    /// How many levels below the chats root this scope sits.
    fn depth(self) -> usize {
        match self {
            ChatsScope::All => 0,
            ChatsScope::Workspace => 1,
            ChatsScope::Session => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
    Windows,
}

impl Os {
    const ALL: [Os; 3] = [Os::MacOs, Os::Linux, Os::Windows];

    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// The parts of the process environment that decide where Cursor keeps its data.
#[derive(Debug, Clone)]
pub struct Env {
    pub os: Os,
    pub home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
    pub cursor_config_dir: Option<PathBuf>,
}

/// Possible locations, most preferred first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Candidates {
    pub chats: Vec<PathBuf>,
    pub projects: Vec<PathBuf>,
    pub ide_db: Vec<PathBuf>,
}

impl Env {
    pub fn current() -> Self {
        Self {
            os: Os::current(),
            home: std::env::home_dir().filter(|home| !home.as_os_str().is_empty()),
            xdg_config_home: env_path("XDG_CONFIG_HOME"),
            appdata: env_path("APPDATA"),
            cursor_config_dir: env_path("CURSOR_CONFIG_DIR"),
        }
    }

    pub fn with_home(os: Os, home: &Path) -> Self {
        Self {
            os,
            home: Some(home.to_path_buf()),
            xdg_config_home: None,
            appdata: None,
            cursor_config_dir: None,
        }
    }

    pub fn candidates(&self) -> Result<Candidates> {
        let home = self.home.as_deref().ok_or(Error::NoHome)?;
        // XDG paths must be absolute; anything else is ignored per the spec.
        let xdg = self.xdg_config_home.clone().filter(|dir| dir.is_absolute());

        // Agent CLI: ~/.cursor, or its config dir (`cursor`) where it honours one.
        let mut agent_roots: Vec<PathBuf> = self.cursor_config_dir.iter().cloned().collect();
        agent_roots.push(home.join(".cursor"));
        match self.os {
            Os::Linux => {
                agent_roots.extend(xdg.iter().map(|dir| dir.join("cursor")));
                agent_roots.push(join(home, &[".config", "cursor"]));
            }
            Os::MacOs => agent_roots.push(join(home, &[".config", "cursor"])),
            Os::Windows => {}
        }

        // IDE: Electron's per-user application data directory.
        let app_data: Vec<PathBuf> = match self.os {
            Os::MacOs => vec![join(home, &["Library", "Application Support"])],
            Os::Linux => xdg.into_iter().chain([home.join(".config")]).collect(),
            Os::Windows => self
                .appdata
                .iter()
                .cloned()
                .chain([join(home, &["AppData", "Roaming"])])
                .collect(),
        };

        Ok(Candidates {
            chats: dedup(agent_roots.iter().map(|root| root.join("chats"))),
            projects: dedup(agent_roots.iter().map(|root| root.join("projects"))),
            ide_db: dedup(
                app_data
                    .iter()
                    .map(|dir| join(dir, &["Cursor", "User", "globalStorage", "state.vscdb"])),
            ),
        })
    }
}

impl StoragePaths {
    pub fn detect() -> Result<Self> {
        Self::from_env(&Env::current())
    }

    pub fn from_env(env: &Env) -> Result<Self> {
        Ok(Self::first_existing(&env.candidates()?))
    }

    pub fn from_home(home: &Path) -> Self {
        let candidates = Env::with_home(Os::current(), home)
            .candidates()
            .unwrap_or_default();
        Self::first_existing(&candidates)
    }

    /// Resolves `--storage`. Only the given location is read; nothing is
    /// detected from the environment. `home` expands a leading `~` (the
    /// current home directory when `None`).
    pub fn from_custom(path: &Path, home: Option<&Path>) -> Result<Self> {
        let given = expand_tilde(path, home)?;
        let path = resolve(&given).map_err(|source| Error::access(&given, source))?;
        let given = std::path::absolute(&given).unwrap_or(given);
        let meta = fs::metadata(&path).map_err(|source| Error::access(&path, source))?;
        let paths = if meta.is_file() {
            // A symlink is recognized by its target's name or its own.
            Self::from_storage_file(&path).or_else(|| Self::from_storage_file(&given))
        } else {
            // Inside a directory that cannot be listed, every location would
            // look present but unreadable.
            fs::read_dir(&path).map_err(|source| Error::access(&path, source))?;
            Self::from_storage_dir(&path)
        };
        paths.ok_or(Error::UnsupportedStorage { path: given })
    }

    pub fn has_agent_storage(&self) -> bool {
        self.chats_dir.as_ref().is_some_and(|p| p.is_dir())
    }

    pub fn has_ide_storage(&self) -> bool {
        self.global_storage_db.as_ref().is_some_and(|p| p.is_file())
    }

    fn first_existing(candidates: &Candidates) -> Self {
        Self {
            chats_dir: first_present(&candidates.chats, fs::Metadata::is_dir),
            chats_scope: ChatsScope::All,
            projects_dir: first_present(&candidates.projects, fs::Metadata::is_dir),
            global_storage_db: first_present(&candidates.ide_db, fs::Metadata::is_file),
        }
    }

    fn from_storage_file(file: &Path) -> Option<Self> {
        let name = file.file_name()?.to_str()?;
        if name == "store.db" {
            return Some(Self::from_chats(file.parent()?, ChatsScope::Session));
        }
        if name.ends_with(".vscdb") || name.ends_with(".vscdb.backup") {
            return Some(Self {
                global_storage_db: Some(file.to_path_buf()),
                ..Default::default()
            });
        }
        None
    }

    fn from_storage_dir(dir: &Path) -> Option<Self> {
        // A home directory, or a copy of one (e.g. /mnt/c/Users/<you> under WSL).
        let home = Os::ALL
            .iter()
            .filter_map(|&os| Env::with_home(os, dir).candidates().ok())
            .fold(Candidates::default(), |mut all, more| {
                all.chats.extend(more.chats);
                all.projects.extend(more.projects);
                all.ide_db.extend(more.ide_db);
                all
            });
        let paths = Self::first_existing(&home);
        if !paths.is_empty() {
            return Some(paths);
        }

        let global_storage_db = [
            dir.join("state.vscdb"),
            dir.join("globalStorage").join("state.vscdb"),
            join(dir, &["User", "globalStorage", "state.vscdb"]),
        ]
        .into_iter()
        .find(|p| p.is_file());
        let mut paths = if dir.join("chats").is_dir() || dir.join("projects").is_dir() {
            Self {
                chats_dir: existing_dir(dir.join("chats")),
                projects_dir: existing_dir(dir.join("projects")),
                ..Default::default()
            }
        } else if global_storage_db.is_some() {
            // Extensions keep arbitrary files under globalStorage, so don't
            // look for agent sessions in an IDE directory.
            Self::default()
        } else if dir.file_name().is_some_and(|name| name == "chats") {
            Self::from_chats(dir, ChatsScope::All)
        } else if let Some(scope) = chats_scope(dir) {
            Self::from_chats(dir, scope)
        } else if has_transcripts(dir) {
            Self {
                projects_dir: Some(dir.to_path_buf()),
                ..Default::default()
            }
        } else {
            Self::default()
        };
        paths.global_storage_db = global_storage_db;
        (!paths.is_empty()).then_some(paths)
    }

    /// `dir` is the `scope` part of a chats tree, whose root's sibling
    /// `projects` directory holds the transcripts.
    fn from_chats(dir: &Path, scope: ChatsScope) -> Self {
        let projects_dir = dir
            .ancestors()
            .nth(scope.depth() + 1)
            .and_then(|parent| existing_dir(parent.join("projects")));
        Self {
            chats_dir: Some(dir.to_path_buf()),
            chats_scope: scope,
            projects_dir,
            global_storage_db: None,
        }
    }

    /// Whether no location was found at all.
    pub fn is_empty(&self) -> bool {
        self.chats_dir.is_none() && self.projects_dir.is_none() && self.global_storage_db.is_none()
    }
}

/// Which part of an agent chats tree `dir` looks like, judging by where its
/// session directories are. `None` when none is in reach.
fn chats_scope(dir: &Path) -> Option<ChatsScope> {
    if is_session_dir(dir) {
        return Some(ChatsScope::Session);
    }
    let children: Vec<PathBuf> = subdirs(dir).collect();
    if children.iter().any(|child| is_session_dir(child)) {
        return Some(ChatsScope::Workspace);
    }
    children
        .iter()
        .any(|child| subdirs(child).any(|session| is_session_dir(&session)))
        .then_some(ChatsScope::All)
}

fn is_session_dir(dir: &Path) -> bool {
    dir.join("meta.json").is_file() || dir.join("store.db").is_file()
}

fn has_transcripts(dir: &Path) -> bool {
    subdirs(dir).any(|project| project.join("agent-transcripts").is_dir())
}

fn subdirs(dir: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
}

pub fn workspace_md5(path: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(path.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn join(base: &Path, parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

fn dedup(paths: impl Iterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// The first of `paths` that exists with the wanted type, or that cannot be
/// inspected (e.g. permission denied), so that reading it reports the problem
/// instead of finding no sessions.
fn first_present(paths: &[PathBuf], wanted: fn(&fs::Metadata) -> bool) -> Option<PathBuf> {
    paths
        .iter()
        .find(|path| match fs::metadata(path) {
            Ok(meta) => wanted(&meta),
            Err(err) => !matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ),
        })
        .cloned()
}

fn existing_dir(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
}

fn expand_tilde(path: &Path, home: Option<&Path>) -> Result<PathBuf> {
    let Some(rest) = path.to_str().and_then(|s| s.strip_prefix('~')) else {
        return Ok(path.to_path_buf());
    };
    let rest = if rest.is_empty() {
        rest
    } else if let Some(rest) = rest.strip_prefix(['/', std::path::MAIN_SEPARATOR]) {
        rest
    } else {
        // `~user` is left to the shell.
        return Ok(path.to_path_buf());
    };
    let home = match home {
        Some(home) => home.to_path_buf(),
        None => Env::current().home.ok_or(Error::NoHome)?,
    };
    Ok(if rest.is_empty() {
        home
    } else {
        home.join(rest)
    })
}

/// Absolute form of an existing path. Windows keeps the plain `C:\...` form
/// rather than the `\\?\` one `canonicalize` returns.
fn resolve(path: &Path) -> io::Result<PathBuf> {
    if cfg!(windows) {
        std::path::absolute(path)
    } else {
        path.canonicalize()
    }
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

    /// An absolute path on any host, so XDG_CONFIG_HOME is honoured.
    fn abs(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    fn env(os: Os) -> Env {
        Env::with_home(os, &abs("home"))
    }

    #[test]
    fn macos_locations() {
        let home = abs("home");
        let candidates = env(Os::MacOs).candidates().unwrap();
        assert_eq!(
            candidates.ide_db,
            [home
                .join("Library")
                .join("Application Support")
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb")]
        );
        assert_eq!(
            candidates.chats,
            [
                home.join(".cursor").join("chats"),
                home.join(".config").join("cursor").join("chats"),
            ]
        );
        assert_eq!(
            candidates.projects[0],
            home.join(".cursor").join("projects")
        );
    }

    #[test]
    fn linux_locations_follow_xdg_config_home() {
        let home = abs("home");
        let ide_db = |config: &Path| {
            config
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb")
        };

        let default = env(Os::Linux).candidates().unwrap();
        assert_eq!(default.ide_db, [ide_db(&home.join(".config"))]);
        assert_eq!(
            default.chats,
            [
                home.join(".cursor").join("chats"),
                home.join(".config").join("cursor").join("chats"),
            ]
        );

        let xdg = Env {
            xdg_config_home: Some(abs("xdg")),
            ..env(Os::Linux)
        }
        .candidates()
        .unwrap();
        assert_eq!(
            xdg.ide_db,
            [ide_db(&abs("xdg")), ide_db(&home.join(".config"))]
        );
        assert_eq!(xdg.chats[1], abs("xdg").join("cursor").join("chats"));

        // The XDG spec says to ignore relative paths.
        let relative = Env {
            xdg_config_home: Some(PathBuf::from("relative")),
            ..env(Os::Linux)
        };
        assert_eq!(relative.candidates().unwrap(), default);
    }

    #[test]
    fn windows_locations_use_appdata_and_the_profile() {
        let home = abs("home");
        let roaming = abs("Roaming");
        let candidates = Env {
            appdata: Some(roaming.clone()),
            xdg_config_home: Some(abs("xdg")),
            ..env(Os::Windows)
        }
        .candidates()
        .unwrap();
        let ide_db = |appdata: &Path| {
            appdata
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb")
        };
        assert_eq!(
            candidates.ide_db,
            [
                ide_db(&roaming),
                ide_db(&home.join("AppData").join("Roaming"))
            ]
        );
        assert_eq!(candidates.chats, [home.join(".cursor").join("chats")]);
        assert_eq!(candidates.projects, [home.join(".cursor").join("projects")]);
    }

    #[test]
    fn cursor_config_dir_comes_first_and_home_is_required() {
        for os in Os::ALL {
            let candidates = Env {
                cursor_config_dir: Some(abs("ccd")),
                ..env(os)
            }
            .candidates()
            .unwrap();
            assert_eq!(candidates.chats[0], abs("ccd").join("chats"));
            assert_eq!(candidates.projects[0], abs("ccd").join("projects"));

            let no_home = Env {
                home: None,
                ..env(os)
            };
            assert!(matches!(no_home.candidates(), Err(Error::NoHome)));
        }
    }

    #[test]
    fn detection_picks_the_first_existing_location() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let fallback = home.join(".config").join("cursor").join("chats");
        fs::create_dir_all(&fallback).unwrap();
        let db = home
            .join(".config")
            .join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb");
        touch(&db);

        let paths = StoragePaths::from_env(&Env::with_home(Os::Linux, home)).unwrap();
        assert_eq!(paths.chats_dir, Some(fallback.clone()));
        assert_eq!(paths.projects_dir, None);
        assert_eq!(paths.global_storage_db, Some(db));

        fs::create_dir_all(home.join(".cursor").join("chats")).unwrap();
        let paths = StoragePaths::from_env(&Env::with_home(Os::Linux, home)).unwrap();
        assert_eq!(paths.chats_dir, Some(home.join(".cursor").join("chats")));
        // Windows keeps the IDE database elsewhere.
        let paths = StoragePaths::from_env(&Env::with_home(Os::Windows, home)).unwrap();
        assert_eq!(paths.global_storage_db, None);

        // A file where a directory is expected counts as absent.
        fs::remove_dir_all(home.join(".cursor")).unwrap();
        touch(&home.join(".cursor"));
        let paths = StoragePaths::from_env(&Env::with_home(Os::Linux, home)).unwrap();
        assert_eq!(paths.chats_dir, Some(fallback));
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    /// A `.cursor` directory with one agent session and its transcript.
    fn dot_cursor(root: &Path) -> (PathBuf, PathBuf) {
        let workspace = root.join(".cursor").join("chats").join("0123abcd");
        let session = workspace.join("session-1");
        touch(&session.join("meta.json"));
        touch(&session.join("store.db"));
        let projects = root.join(".cursor").join("projects");
        fs::create_dir_all(projects.join("p").join("agent-transcripts")).unwrap();
        (session, projects)
    }

    fn custom(path: &Path) -> StoragePaths {
        StoragePaths::from_custom(path, None).unwrap()
    }

    fn agent(chats_dir: &Path, projects_dir: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
        (
            Some(resolve(chats_dir).unwrap()),
            Some(resolve(projects_dir).unwrap()),
        )
    }

    #[test]
    fn storage_accepts_agent_locations() {
        let dir = tempfile::tempdir().unwrap();
        let (session, projects) = dot_cursor(dir.path());
        let workspace = session.parent().unwrap();
        let chats = workspace.parent().unwrap();

        for (given, chats_dir, scope) in [
            (session.join("store.db"), &session, ChatsScope::Session),
            (session.clone(), &session, ChatsScope::Session),
            (
                workspace.to_path_buf(),
                &workspace.to_path_buf(),
                ChatsScope::Workspace,
            ),
            (chats.to_path_buf(), &chats.to_path_buf(), ChatsScope::All),
            (
                dir.path().join(".cursor"),
                &chats.to_path_buf(),
                ChatsScope::All,
            ),
        ] {
            let paths = custom(&given);
            assert_eq!(
                (paths.chats_dir, paths.projects_dir),
                agent(chats_dir, &projects),
                "{}",
                given.display()
            );
            assert_eq!(paths.chats_scope, scope, "{}", given.display());
            assert_eq!(paths.global_storage_db, None, "{}", given.display());
        }

        // A stray file in a workspace directory doesn't narrow the chats root.
        touch(&workspace.join("meta.json"));
        for given in [chats.to_path_buf(), dir.path().join(".cursor")] {
            assert_eq!(custom(&given).chats_scope, ChatsScope::All);
        }

        // An empty chats directory is still recognised by name.
        let empty = dir.path().join("other").join("chats");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(custom(&empty).chats_dir, Some(resolve(&empty).unwrap()));
    }

    #[test]
    fn storage_accepts_ide_locations() {
        let dir = tempfile::tempdir().unwrap();
        let cursor = dir.path().join("Cursor");
        let global_storage = cursor.join("User").join("globalStorage");
        let db = global_storage.join("state.vscdb");
        touch(&db);
        touch(&global_storage.join("state.vscdb.backup"));

        for (given, expected) in [
            (db.clone(), db.clone()),
            (global_storage.clone(), db.clone()),
            (cursor.join("User"), db.clone()),
            (cursor.clone(), db.clone()),
            (
                global_storage.join("state.vscdb.backup"),
                global_storage.join("state.vscdb.backup"),
            ),
        ] {
            let paths = custom(&given);
            assert_eq!(paths.global_storage_db, Some(resolve(&expected).unwrap()));
            assert_eq!((paths.chats_dir, paths.projects_dir), (None, None));
        }
    }

    #[test]
    fn ide_directories_are_not_searched_for_agent_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let global_storage = dir.path().join("User").join("globalStorage");
        touch(&global_storage.join("state.vscdb"));
        touch(
            &global_storage
                .join("vendor.extension")
                .join("cache")
                .join("meta.json"),
        );

        for given in [global_storage, dir.path().join("User")] {
            let paths = custom(&given);
            assert_eq!(paths.chats_dir, None, "{}", given.display());
            assert!(paths.global_storage_db.is_some());
        }
    }

    #[test]
    fn storage_is_authoritative() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        dot_cursor(&home);
        let db = dir.path().join("copy").join("state.vscdb");
        touch(&db);

        for given in [db.clone(), Path::new("~").join("..").join("copy")] {
            let paths = StoragePaths::from_custom(&given, Some(&home)).unwrap();
            assert_eq!(paths.global_storage_db, Some(resolve(&db).unwrap()));
            assert_eq!((paths.chats_dir, paths.projects_dir), (None, None));
        }
    }

    #[cfg(unix)]
    #[test]
    fn detection_reports_locations_it_cannot_inspect() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let global_storage = home
            .join("Library")
            .join("Application Support")
            .join("Cursor")
            .join("User")
            .join("globalStorage");
        touch(&global_storage.join("state.vscdb"));
        fs::create_dir_all(home.join(".cursor").join("chats")).unwrap();

        let locked = [global_storage.as_path(), &home.join(".cursor")];
        for dir in locked {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).unwrap();
        }
        let readable = fs::read_dir(&global_storage).is_ok(); // root ignores permissions
        let paths = StoragePaths::from_env(&Env::with_home(Os::MacOs, home)).unwrap();
        for dir in locked {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
        }
        if readable {
            eprintln!("skipped: permissions are not enforced for this user (root)");
            return;
        }

        assert_eq!(
            paths.global_storage_db,
            Some(global_storage.join("state.vscdb"))
        );
        assert_eq!(paths.chats_dir, Some(home.join(".cursor").join("chats")));
        assert_eq!(
            paths.projects_dir,
            Some(home.join(".cursor").join("projects"))
        );
    }

    #[test]
    fn storage_accepts_a_home_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (session, projects) = dot_cursor(dir.path());
        let db = dir
            .path()
            .join("AppData")
            .join("Roaming")
            .join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb");
        touch(&db);

        let paths = custom(dir.path());
        let chats = session.parent().unwrap().parent().unwrap();
        assert_eq!(
            (paths.chats_dir, paths.projects_dir),
            agent(chats, &projects)
        );
        assert_eq!(paths.global_storage_db, Some(resolve(&db).unwrap()));
    }

    #[test]
    fn storage_expands_tilde() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        touch(&db);
        let paths =
            StoragePaths::from_custom(Path::new("~/state.vscdb"), Some(dir.path())).unwrap();
        assert_eq!(paths.global_storage_db, Some(resolve(&db).unwrap()));
    }

    #[test]
    fn storage_rejects_missing_and_unknown_paths() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            StoragePaths::from_custom(&dir.path().join("missing"), None),
            Err(Error::StorageNotFound { .. })
        ));

        let notes = dir.path().join("notes.txt");
        touch(&notes);
        let empty = dir.path().join("empty");
        fs::create_dir(&empty).unwrap();
        for path in [notes, empty] {
            let err = StoragePaths::from_custom(&path, None).unwrap_err();
            assert!(matches!(err, Error::UnsupportedStorage { .. }), "{err}");
            assert!(err.to_string().contains("unrecognized storage location"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn storage_symlinks_count_by_their_own_name_too() {
        let dir = tempfile::tempdir().unwrap();
        let renamed = dir.path().join("renamed.db");
        touch(&renamed);
        let link = dir.path().join("link.vscdb");
        std::os::unix::fs::symlink(&renamed, &link).unwrap();
        let paths = StoragePaths::from_custom(&link, None).unwrap();
        assert_eq!(paths.global_storage_db, Some(link));

        // An unknown file is reported by the name it was given.
        let notes = dir.path().join("notes.txt");
        touch(&notes);
        let named = dir.path().join("named");
        std::os::unix::fs::symlink(&notes, &named).unwrap();
        let err = StoragePaths::from_custom(&named, None).unwrap_err();
        assert!(
            matches!(&err, Error::UnsupportedStorage { path } if *path == named),
            "{err}"
        );
    }
}

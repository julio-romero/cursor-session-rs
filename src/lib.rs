pub mod agent;
pub mod detect;
pub mod export;
pub mod ide;
pub mod model;
mod sqlite;

use anyhow::Result;

use crate::detect::StoragePaths;
use crate::model::Session;

pub fn load_sessions(paths: &StoragePaths) -> Result<Vec<Session>> {
    let mut sessions = Vec::new();
    sessions.extend(agent::load_sessions(paths)?);
    sessions.extend(ide::load_sessions(paths)?);
    Ok(model::merge_sessions(sessions))
}

pub fn find_session<'a>(sessions: &'a [Session], query: &str) -> Option<&'a Session> {
    if let Some(exact) = sessions.iter().find(|s| s.id == query) {
        return Some(exact);
    }
    let matches: Vec<_> = sessions
        .iter()
        .filter(|s| s.id.starts_with(query))
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

pub fn filter_workspace<'a>(sessions: &'a [Session], workspace: &str) -> Vec<&'a Session> {
    sessions
        .iter()
        .filter(|session| {
            session
                .workspace
                .as_deref()
                .is_some_and(|cwd| cwd == workspace || cwd.contains(workspace))
                || session
                    .workspace_hash
                    .as_deref()
                    .is_some_and(|hash| hash == workspace)
        })
        .collect()
}

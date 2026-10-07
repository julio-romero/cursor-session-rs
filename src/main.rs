mod cli;
mod commands;
mod output;

use std::path::Path;

use anyhow::Result;
use clap::Parser;
use cursor_session::detect::StoragePaths;

use crate::cli::Cli;
use crate::output::OutputOpts;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = resolve_paths(cli.storage.as_deref())?;
    if cli.verbose {
        eprintln!("chats: {:?}", paths.chats_dir);
        eprintln!("projects: {:?}", paths.projects_dir);
        eprintln!("ide db: {:?}", paths.global_storage_db);
    }

    let opts = OutputOpts::detect(cli.color);
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    commands::run(cli.command, &paths, &opts, &mut out, &mut err)
}

fn resolve_paths(storage: Option<&Path>) -> Result<StoragePaths> {
    let paths = match storage {
        Some(path) => StoragePaths::from_custom(path, None)?,
        None => StoragePaths::detect()?,
    };
    Ok(paths)
}

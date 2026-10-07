use std::io::{self, Write};
use std::path::Path;

use crate::model::Session;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Jsonl,
    Md,
    Json,
    Yaml,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jsonl => "jsonl",
            Format::Md => "md",
            Format::Json => "json",
            Format::Yaml => "yaml",
        }
    }
}

pub fn export_session(session: &Session, format: Format, writer: &mut impl Write) -> Result<()> {
    match format {
        Format::Jsonl => export_jsonl(session, writer),
        Format::Md => export_markdown(session, writer).map_err(Error::Write),
        Format::Json => export_json(session, writer),
        Format::Yaml => export_yaml(session, writer),
    }
}

pub fn export_path(out_dir: &Path, session: &Session, format: Format) -> std::path::PathBuf {
    out_dir.join(format!("{}.{}", session.id, format.extension()))
}

fn export_jsonl(session: &Session, writer: &mut impl Write) -> Result<()> {
    for message in &session.messages {
        serde_json::to_writer(&mut *writer, message)?;
        writer.write_all(b"\n").map_err(Error::Write)?;
    }
    Ok(())
}

fn export_json(session: &Session, writer: &mut impl Write) -> Result<()> {
    serde_json::to_writer_pretty(&mut *writer, session)?;
    writer.write_all(b"\n").map_err(Error::Write)?;
    Ok(())
}

fn export_yaml(session: &Session, writer: &mut impl Write) -> Result<()> {
    let yaml = serde_yaml::to_string(session)?;
    writer.write_all(yaml.as_bytes()).map_err(Error::Write)?;
    Ok(())
}

fn export_markdown(session: &Session, writer: &mut impl Write) -> io::Result<()> {
    writeln!(writer, "# {}\n", session.title)?;
    writeln!(writer, "- **ID:** `{}`", session.id)?;
    writeln!(writer, "- **Source:** {}", session.source.as_str())?;
    if let Some(workspace) = &session.workspace {
        writeln!(writer, "- **Workspace:** {workspace}")?;
    }
    if let Some(model) = &session.model {
        writeln!(writer, "- **Model:** {model}")?;
    }
    writeln!(writer, "- **Created:** {}", session.created_display())?;
    writeln!(writer, "- **Updated:** {}", session.updated_display())?;
    writeln!(writer, "- **Messages:** {}\n", session.message_count())?;
    writeln!(writer, "---\n")?;
    for (index, message) in session.messages.iter().enumerate() {
        let ts = message
            .timestamp
            .as_deref()
            .map(|t| format!(" ({t})"))
            .unwrap_or_default();
        writeln!(writer, "**{}:**{ts}\n", message.role)?;
        writeln!(writer, "{}\n", message.content)?;
        if index + 1 < session.messages.len() {
            writeln!(writer, "---\n")?;
        }
    }
    Ok(())
}

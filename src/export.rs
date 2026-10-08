use std::borrow::Cow;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

use crate::model::Session;
use crate::{Error, Result};

/// Longest session ID used unchanged as a file name.
const MAX_PLAIN_STEM: usize = 100;
/// Names Windows reserves for devices, with any extension.
const WINDOWS_DEVICES: [&str; 4] = ["con", "prn", "aux", "nul"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Md,
    Json,
    Jsonl,
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

/// `<out_dir>/<session id>.<ext>`, always directly inside `out_dir`.
pub fn export_path(out_dir: &Path, session: &Session, format: Format) -> PathBuf {
    out_dir.join(format!("{}.{}", file_stem(&session.id), format.extension()))
}

/// The export paths of one run, as [`export_path`] gives them, except that a
/// session whose file name differs only in case from one given before gets
/// the name of an ID that is not plain: macOS and Windows would otherwise
/// write both sessions to one file.
#[derive(Debug, Default)]
pub struct ExportPaths {
    taken: HashSet<String>,
}

impl ExportPaths {
    pub fn next(&mut self, out_dir: &Path, session: &Session, format: Format) -> PathBuf {
        let mut stem = file_stem(&session.id);
        if !self.taken.insert(stem.to_lowercase()) {
            stem = Cow::Owned(hashed_stem(&session.id));
            self.taken.insert(stem.to_lowercase());
        }
        out_dir.join(format!("{stem}.{}", format.extension()))
    }
}

/// The session ID when it is a plain file name on every OS: ASCII letters,
/// digits, `-`, `_` and `.`, not starting with `.` and not a Windows device
/// name. In other IDs, which only a crafted or damaged database holds, every
/// character but letters, digits and `-` becomes `_` and a hash of the ID is
/// appended, so they can neither leave the export directory nor share a name.
fn file_stem(id: &str) -> Cow<'_, str> {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if !id.is_empty()
        && id.len() <= MAX_PLAIN_STEM
        && !id.starts_with('.')
        && id.chars().all(plain)
        && !is_windows_device(id)
    {
        return Cow::Borrowed(id);
    }
    Cow::Owned(hashed_stem(id))
}

/// `id` with every character but letters, digits and `-` as `_`, cut to
/// [`MAX_PLAIN_STEM`], and a hash of the whole ID appended.
fn hashed_stem(id: &str) -> String {
    let kept = |c: char| c.is_ascii_alphanumeric() || c == '-';
    let mut stem: String = id
        .chars()
        .take(MAX_PLAIN_STEM)
        .map(|c| if kept(c) { c } else { '_' })
        .collect();
    stem.push('_');
    for byte in &Md5::digest(id.as_bytes())[..6] {
        stem.push_str(&format!("{byte:02x}"));
    }
    stem
}

/// `CON`, `NUL.txt`, `com1` and the like, which Windows opens as devices.
fn is_windows_device(name: &str) -> bool {
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    WINDOWS_DEVICES.contains(&base.as_str())
        || (base.len() == 4
            && (base.starts_with("com") || base.starts_with("lpt"))
            && base.ends_with(|c: char| c.is_ascii_digit()))
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
    writeln!(writer, "- **Created:** {}", session.created_utc())?;
    writeln!(writer, "- **Updated:** {}", session.updated_utc())?;
    writeln!(writer, "- **Messages:** {}\n", session.message_count)?;
    writeln!(writer, "---\n")?;
    for (index, message) in session.messages.iter().enumerate() {
        let ts = message
            .timestamp_display()
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

#[cfg(test)]
mod tests {
    use std::path::Component;

    use super::*;

    #[test]
    fn plain_ids_name_their_file_unchanged() {
        for id in [
            "f4eea6d2-d2d3-41ad-b290-824445295a15",
            "transcript_only.v2",
            "Mixed-Case",
        ] {
            assert_eq!(file_stem(id), id);
        }
    }

    #[test]
    fn ids_that_differ_only_in_case_get_their_own_files() {
        let session = |id: &str| {
            let summary = crate::model::SessionSummary::new(id, "", crate::model::Source::Ide);
            Session::new(summary, Vec::new())
        };
        let out = Path::new("out");
        let mut paths = ExportPaths::default();
        let names: Vec<String> = ["abcdef00-01", "ABCDEF00-01", "Other", "abcdef00-01"]
            .into_iter()
            .map(|id| {
                let path = paths.next(out, &session(id), Format::Md);
                assert_eq!(path.parent(), Some(out));
                path.file_name().unwrap().to_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(names[0], "abcdef00-01.md");
        assert_eq!(names[1], format!("{}.md", hashed_stem("ABCDEF00-01")));
        assert_eq!(names[2], "Other.md");
        let mut lowercase: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
        lowercase.sort();
        lowercase.dedup();
        assert_eq!(lowercase.len(), 4);
    }

    #[test]
    fn other_ids_stay_one_distinct_name_inside_the_directory() {
        let ids = [
            "",
            ".",
            "..",
            ".hidden",
            "../../escaped",
            "/tmp/absolute",
            r"C:\Windows\absolute",
            r"..\escaped",
            "a/b",
            "a_b",
            "a:b",
            "tab\there",
            "nul",
            "NUL.txt",
            "com1",
            "Lpt9.md",
            "ünïcödé",
            &"x".repeat(MAX_PLAIN_STEM + 1),
            &"x".repeat(300),
        ];
        let mut names = Vec::new();
        for id in ids {
            let stem = file_stem(id);
            let name = format!("{stem}.md");
            let components: Vec<_> = Path::new(&name).components().collect();
            assert!(
                matches!(components[..], [Component::Normal(_)]),
                "{id:?}: {name}"
            );
            assert!(
                !name.starts_with('.') && !name.contains(['/', '\\', ':']),
                "{id:?}"
            );
            assert!(!is_windows_device(&name), "{id:?}: {name}");
            assert!(stem.len() <= MAX_PLAIN_STEM + 13, "{id:?}");
            // NAME_MAX is 255 bytes on every OS this runs on.
            assert!(name.len() <= 255, "{id:?}");
            names.push(name);
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), ids.len());
        assert_eq!(file_stem("a_b"), "a_b");
        assert!(file_stem("a/b").starts_with("a_b_"));
        // A plain ID is cut and hashed once it is too long to keep.
        let longest = "x".repeat(MAX_PLAIN_STEM);
        assert_eq!(file_stem(&longest), longest);
        let long = "x".repeat(MAX_PLAIN_STEM + 1);
        assert_eq!(file_stem(&long).len(), MAX_PLAIN_STEM + 13);
        assert!(file_stem(&long).starts_with(&format!("{longest}_")));
    }
}

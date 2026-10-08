//! Shell completion scripts and man pages, made from the command-line
//! definition so that they always match it. Releases ship the copies
//! committed in `completions/` and `man/`, which `tests/generated.rs` keeps
//! current.

use std::io;

use clap::CommandFactory;
use clap_mangen::Man;
use clap_mangen::roff::{Roff, roman};

use crate::cli::{Cli, Shell};

/// The program's name in scripts and pages, also on Windows, where the binary
/// ends in `.exe`.
const NAME: &str = "cursor-session";

/// The completion script for `shell`.
pub fn completions(shell: Shell) -> Vec<u8> {
    let shell = match shell {
        Shell::Bash => clap_complete::Shell::Bash,
        Shell::Zsh => clap_complete::Shell::Zsh,
        Shell::Fish => clap_complete::Shell::Fish,
    };
    // clap_complete panics when its writer fails, which a Vec never does. A
    // closed stdout is then reported like any other output's.
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), NAME, &mut script);
    script
}

/// The man page of `command`, a visible subcommand, or with `None` the page
/// of the program as a whole, in roff. It carries no date, so a version always
/// has the same pages.
pub fn man_page(command: Option<&str>) -> io::Result<Vec<u8>> {
    let mut root = Cli::command().bin_name(NAME).disable_help_subcommand(true);
    // Building names each subcommand's page after its parent's, e.g.
    // cursor-session-list(1), which the overview's list refers to.
    root.build();
    let cmd = match command {
        None => root,
        Some(name) => root
            .get_subcommands()
            .find(|sub| sub.get_name() == name && !sub.is_hide_set())
            .cloned()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("no man page for `{name}`"))
            })?,
    };
    let man = Man::new(cmd.clone())
        .source(format!("{NAME} {}", env!("CARGO_PKG_VERSION")))
        .manual("User Commands");

    let mut page = Page::default();
    page.append(|w| man.render_title(w))?;
    page.append(|w| man.render_name_section(w))?;
    page.append(|w| man.render_synopsis_section(w))?;
    let mut description = Roff::new();
    description.control("SH", ["DESCRIPTION"]);
    if let Some(about) = cmd.get_long_about().or_else(|| cmd.get_about()) {
        help_text(&mut description, &about.to_string());
    }
    page.append(|w| description.to_writer(w))?;
    if cmd.get_arguments().any(|arg| !arg.is_hide_set()) {
        page.append(|w| man.render_options_section(w))?;
    }
    if cmd.get_subcommands().any(|sub| !sub.is_hide_set()) {
        page.append(|w| man.render_subcommands_section(w))?;
    }
    if let Some(tail) = cmd.get_after_long_help() {
        let tail = help_tail(&tail.to_string());
        page.append(|w| tail.to_writer(w))?;
    }
    if cmd.get_version().is_some() {
        page.append(|w| man.render_version_section(w))?;
    }
    Ok(page.0)
}

/// A man page put together from separately rendered parts.
#[derive(Default)]
struct Page(Vec<u8>);

impl Page {
    /// Appends what `render` writes. Every rendering starts with the same
    /// preamble, which the page needs only once.
    fn append(&mut self, render: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> io::Result<()> {
        let mut part = Vec::new();
        render(&mut part)?;
        let mut preamble = Vec::new();
        Roff::new().to_writer(&mut preamble)?;
        let part = match part.strip_prefix(preamble.as_slice()) {
            Some(rest) if !self.0.is_empty() => rest,
            _ => &part,
        };
        self.0.extend_from_slice(part);
        Ok(())
    }
}

/// The sections of `--help`'s tail (examples, exit codes), each headed by
/// its unindented `Name:` line.
fn help_tail(tail: &str) -> Roff {
    let mut roff = Roff::new();
    let mut body = String::new();
    for line in tail.lines() {
        if let Some(name) = line.strip_suffix(':').filter(|_| !line.starts_with(' ')) {
            help_text(&mut roff, &body);
            body.clear();
            let heading = match name {
                "Exit codes" => "EXIT STATUS".to_string(),
                name => name.to_uppercase(),
            };
            roff.control("SH", [heading.as_str()]);
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    help_text(&mut roff, &body);
    roff
}

/// Adds help text to `roff`. Blank lines separate paragraphs, and runs of
/// indented lines (commands, tables) keep their line breaks and spacing,
/// which roff would otherwise fill into one paragraph.
fn help_text(roff: &mut Roff, text: &str) {
    let text = text.trim_matches('\n');
    let mut indented = false;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        if indent > 0 && !indented {
            roff.control("RS", ["4"]).control("nf", []);
        } else if indent == 0 && indented {
            roff.control("fi", []).control("RE", []);
        }
        indented = indent > 0;
        if line.trim().is_empty() {
            roff.control("PP", []);
        } else {
            roff.text([roman(line.trim_start())]);
        }
    }
    if indented {
        roff.control("fi", []).control("RE", []);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::documented_commands;

    fn page(command: Option<&str>) -> String {
        String::from_utf8(man_page(command).unwrap()).unwrap()
    }

    #[test]
    fn every_shell_gets_a_script_for_every_visible_command() {
        // Each script registers for the program's name, also on Windows.
        for (shell, registers) in [
            (Shell::Bash, "-o bashdefault -o default cursor-session\n"),
            (Shell::Zsh, "#compdef cursor-session\n"),
            (Shell::Fish, "\ncomplete -c cursor-session "),
        ] {
            let script = String::from_utf8(completions(shell)).unwrap();
            assert!(script.contains(registers), "{shell:?}: {script}");
            for command in documented_commands() {
                assert!(script.contains(&command), "{shell:?}: {command}");
            }
            assert!(script.contains("storage"), "{shell:?}");
        }
    }

    #[test]
    fn overview_page_lists_each_page_and_the_exit_codes() {
        let page = page(None);
        assert!(page.starts_with(".ie \\n(.g .ds Aq"), "{page}");
        assert_eq!(page.matches(".ds Aq").count(), 2, "one preamble: {page}");
        assert!(page.contains("\n.TH cursor-session 1  \"cursor-session "));
        for command in documented_commands() {
            assert!(
                page.contains(&format!("cursor\\-session\\-{command}(1)")),
                "{command}"
            );
        }
        assert!(!page.contains("cursor\\-session\\-man(1)"));
        assert!(!page.contains("cursor\\-session\\-help(1)"));
        assert!(page.contains("\n.SH EXAMPLES\n.RS 4\n.nf\ncursor\\-session list\n"));
        assert!(page.contains("\n.SH \"EXIT STATUS\"\n.RS 4\n.nf\n0  Success"));
        assert!(page.contains("\n.SH VERSION\n"));
    }

    #[test]
    fn subcommand_pages_document_their_own_and_the_global_options() {
        let page = page(Some("show"));
        assert!(page.contains("\n.TH cursor-session-show 1 "), "{page}");
        assert!(page.contains("\\fB\\-\\-all\\fR"), "{page}");
        assert!(page.contains("\\fB\\-\\-storage\\fR"), "{page}");
        assert!(page.contains("\n.SH EXAMPLES\n"));
        assert!(!page.contains("\n.SH VERSION\n"));

        let error = man_page(Some("man")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn indented_lines_keep_their_breaks() {
        let mut roff = Roff::new();
        help_text(&mut roff, "Intro:\n  a  b\n  c\n\nAfter.\n");
        assert_eq!(
            roff.to_roff(),
            "Intro:\n.RS 4\n.nf\na  b\nc\n.fi\n.RE\n.PP\nAfter.\n"
        );
    }
}

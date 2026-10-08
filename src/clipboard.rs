//! The system clipboard, behind a trait so that the commands can be tested
//! with a fake one: no test touches the real clipboard.
//!
//! On macOS and Windows the system keeps what is copied after the program
//! exits. On X11 the program that copied text must serve it to every program
//! that pastes it, and the text is gone once it exits, so `handoff` starts a
//! copy of itself in the background (`serve-clipboard`, hidden) that holds the
//! text until something else is copied. `handoff` says it copied only once
//! that copy owns the clipboard, and waits for that at most
//! [`HANDOVER_TIMEOUT`].

use std::env;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use cursor_session::handoff::count;

/// Where copied text goes.
pub trait Clipboard {
    /// Puts `text` on the clipboard, where it stays after this program
    /// exits, or says why it could not.
    fn set_text(&mut self, text: &str) -> Result<(), String>;
}

/// Whether the clipboard is X11's, which a process must keep serving: what
/// arboard uses on Unix other than macOS.
const SERVED: bool = cfg!(all(unix, not(target_os = "macos")));

/// How long `handoff` waits for the background copy to own the clipboard.
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(5);

/// The line the background copy prints once it owns the clipboard. Anything
/// else it prints is why it could not.
const READY: &str = "ok";

/// The system clipboard, through arboard.
pub struct System;

impl Clipboard for System {
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        if !SERVED {
            return copy(text);
        }
        if !has_display(
            env::var_os("DISPLAY").as_deref(),
            env::var_os("WAYLAND_DISPLAY").as_deref(),
        ) {
            return Err("no display: neither DISPLAY nor WAYLAND_DISPLAY is set".to_string());
        }
        let exe =
            env::current_exe().map_err(|error| format!("could not find this program: {error}"))?;
        let mut command = Command::new(exe);
        command.arg("serve-clipboard").current_dir("/");
        // Its own process group, so that Ctrl-C in the terminal and the
        // terminal closing do not take the copied text with it.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        hand_over(command, text, HANDOVER_TIMEOUT)
    }
}

/// Puts `text` on the clipboard of this process.
fn copy(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|error| error.to_string())
}

/// Whether a display server is there to hold the clipboard: `DISPLAY` or
/// `WAYLAND_DISPLAY` is set and not empty.
fn has_display(display: Option<&OsStr>, wayland: Option<&OsStr>) -> bool {
    [display, wayland]
        .into_iter()
        .any(|value| value.is_some_and(|value| !value.is_empty()))
}

/// Starts `command`, which must read `text` from its stdin, then print
/// [`READY`] once it owns the clipboard and keep running to serve it. It is
/// left running then, and stopped otherwise; `timeout` bounds the wait.
// The child serving the clipboard outlives this process on purpose.
#[allow(clippy::zombie_processes)]
fn hand_over(mut command: Command, text: &str, timeout: Duration) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start the clipboard process: {error}"))?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("could not talk to the clipboard process".to_string());
    };
    // Writing and reading happen on a thread, so that a child that never
    // reads or answers cannot block this one past the timeout. A child that
    // stops reading has failed, which its answer says.
    let text = text.to_string();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = stdin.write_all(text.as_bytes());
        drop(stdin);
        let mut line = String::new();
        let read = BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(read.map(|_| line));
    });
    let answer = receiver.recv_timeout(timeout);
    if let Ok(Ok(line)) = &answer
        && line.trim_end() == READY
    {
        return Ok(());
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(match answer {
        Ok(Ok(line)) => match line.trim() {
            "" => "the clipboard process exited".to_string(),
            reason => reason.strip_prefix("error: ").unwrap_or(reason).to_string(),
        },
        Ok(Err(error)) => format!("could not talk to the clipboard process: {error}"),
        Err(_) => format!(
            "the clipboard process did not answer within {}",
            count(
                usize::try_from(timeout.as_secs()).unwrap_or(usize::MAX),
                "second"
            )
        ),
    })
}

/// `serve-clipboard`: puts what `input` holds on the clipboard, prints
/// [`READY`] to `out`, then on X11 keeps serving it until something else is
/// copied. A failure is printed as `error: ...` instead.
pub fn serve(input: &mut dyn Read, out: &mut dyn Write) -> Result<()> {
    let result = read_and_copy(input);
    match &result {
        Ok(_) => writeln!(out, "{READY}")?,
        Err(reason) => writeln!(out, "error: {reason}")?,
    }
    out.flush()?;
    let (mut clipboard, text) = result.map_err(|reason| anyhow!(reason))?;
    keep_serving(&mut clipboard, text)
}

fn read_and_copy(input: &mut dyn Read) -> Result<(arboard::Clipboard, String), String> {
    let mut text = String::new();
    input
        .read_to_string(&mut text)
        .map_err(|error| format!("could not read the text to copy: {error}"))?;
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .set_text(text.as_str())
        .map_err(|error| error.to_string())?;
    Ok((clipboard, text))
}

/// Serves `text`, which this process has already put on the clipboard,
/// until another program replaces it.
#[cfg(all(unix, not(target_os = "macos")))]
fn keep_serving(clipboard: &mut arboard::Clipboard, text: String) -> Result<()> {
    use arboard::SetExtLinux;
    clipboard.set().wait().text(text)?;
    Ok(())
}

/// The system keeps what was copied.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn keep_serving(_clipboard: &mut arboard::Clipboard, _text: String) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_is_one_named_by_either_variable() {
        let os = |value| Some(OsStr::new(value));
        assert!(has_display(os(":0"), None));
        assert!(has_display(None, os("wayland-0")));
        assert!(has_display(os(""), os("wayland-0")));
        assert!(!has_display(None, None));
        assert!(!has_display(os(""), os("")));
    }

    /// A stand-in for `serve-clipboard` that runs `script` with the text on
    /// its stdin.
    #[cfg(unix)]
    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[cfg(unix)]
    #[test]
    fn the_handover_succeeds_once_the_process_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let copied = dir.path().join("copied");
        // It returns while the process keeps serving, long before the
        // timeout.
        let script = format!("cat > '{}'; echo ok; exec sleep 10", copied.display());
        let text = "transcript\n".repeat(20_000);
        hand_over(shell(&script), &text, Duration::from_secs(5)).unwrap();
        assert_eq!(std::fs::read_to_string(copied).unwrap(), text);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_handover_says_why() {
        let fails = |script: &str, timeout| hand_over(shell(script), "text", timeout).unwrap_err();
        let long = Duration::from_secs(20);
        assert_eq!(
            fails(
                "cat >/dev/null; echo 'error: X11 server connection timed out'",
                long
            ),
            "X11 server connection timed out"
        );
        assert_eq!(
            fails("cat >/dev/null", long),
            "the clipboard process exited"
        );
        assert_eq!(fails("exit 3", long), "the clipboard process exited");
        let started = std::time::Instant::now();
        assert_eq!(
            fails("exec sleep 30", Duration::from_secs(1)),
            "the clipboard process did not answer within 1 second"
        );
        assert!(started.elapsed() < Duration::from_secs(20));

        let missing = Command::new("/nonexistent/serve-clipboard");
        let reason = hand_over(missing, "text", long).unwrap_err();
        assert!(
            reason.starts_with("could not start the clipboard process: "),
            "{reason}"
        );
    }

    #[test]
    fn serving_reports_a_failure_on_its_output() {
        struct Unreadable;
        impl Read for Unreadable {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::InvalidData.into())
            }
        }
        let mut out = Vec::new();
        assert!(serve(&mut Unreadable, &mut out).is_err());
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.starts_with("error: could not read the text to copy: "),
            "{out}"
        );
        assert_eq!(out.lines().count(), 1);
    }
}

//! The system clipboard, behind a trait so that the commands can be tested
//! with a fake one: no test touches the real clipboard.
//!
//! - macOS: the text is piped into `/usr/bin/pbcopy`, so that the program
//!   links none of AppKit, which every command would pay for at start.
//! - Windows: arboard; the system keeps what is copied.
//! - X11 (Linux and the BSDs): the program that copied text must serve it to
//!   every program that pastes it, and the text is gone once it exits. So
//!   `handoff` starts a copy of itself in the background
//!   (`serve-clipboard`, hidden) that takes the clipboard once and serves it
//!   until something else is copied, the X connection is lost, or
//!   [`MAX_SERVE`] has passed. `handoff` says it copied only once that copy
//!   owns the clipboard, and waits for that at most [`HANDOVER_TIMEOUT`].
//!   arboard is built without Wayland support: without an X11 display the
//!   text is printed, with a warning that names `wl-copy`.
//!
//! The handover to `serve-clipboard` is used on X11 only, and checked for
//! dead code there; elsewhere its tests still run.
#![cfg_attr(
    not(all(
        unix,
        not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
    )),
    allow(dead_code)
)]

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use cursor_session::handoff::count;

/// Where copied text goes.
pub trait Clipboard {
    /// Puts `text` on the clipboard, where it stays after this program
    /// exits, or says why it could not.
    fn set_text(&mut self, text: &str) -> Result<(), String>;
}

/// The system clipboard.
pub struct System;

impl Clipboard for System {
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        copy(text)
    }
}

/// macOS's clipboard tool, by its absolute path: no other `pbcopy` found on
/// `PATH` is run.
#[cfg(target_os = "macos")]
const PBCOPY: &str = "/usr/bin/pbcopy";

#[cfg(target_os = "macos")]
fn copy(text: &str) -> Result<(), String> {
    pipe_into(pbcopy(PBCOPY), text)
}

/// Hands `text` to a `serve-clipboard` process, which keeps serving it.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
))]
fn copy(text: &str) -> Result<(), String> {
    use std::env;
    if let Some(reason) = no_display(
        env::var_os("DISPLAY").as_deref(),
        env::var_os("WAYLAND_DISPLAY").as_deref(),
    ) {
        return Err(reason.to_string());
    }
    let exe =
        env::current_exe().map_err(|error| format!("could not find this program: {error}"))?;
    let mut command = Command::new(exe);
    command.arg("serve-clipboard").current_dir("/");
    // Its own process group, so that Ctrl-C in the terminal and the terminal
    // closing do not take the copied text with it.
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    hand_over(command, text, HANDOVER_TIMEOUT)
}

/// Puts `text` on the clipboard through arboard, where the system keeps it.
#[cfg(not(any(
    target_os = "macos",
    all(unix, not(any(target_os = "android", target_os = "emscripten")))
)))]
fn copy(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|error| error.to_string())
}

/// `program` set up to run as pbcopy: told that its input is UTF-8, which
/// it otherwise takes from the locale, and would garble what is not ASCII
/// under the C locale.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn pbcopy(program: &str) -> Command {
    let mut command = Command::new(program);
    command.env("LC_CTYPE", "UTF-8").env_remove("LC_ALL");
    command
}

/// Runs `command` with `text` on its stdin, and waits for it to exit. It
/// fails if the command cannot be started or exits unsuccessfully.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn pipe_into(mut command: Command, text: &str) -> Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not run {program}: {error}"))?;
    // A program that exits without reading all of it closes the pipe; its
    // exit status then says more than the write error.
    let written = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(text.as_bytes()));
    let status = child
        .wait()
        .map_err(|error| format!("could not run {program}: {error}"))?;
    if !status.success() {
        return Err(format!("{program} failed ({status})"));
    }
    match written {
        Some(Ok(())) => Ok(()),
        Some(Err(error)) => Err(format!("could not write to {program}: {error}")),
        None => Err(format!("could not write to {program}")),
    }
}

/// Why X11's clipboard cannot be used with these values of `DISPLAY` and
/// `WAYLAND_DISPLAY`, if it cannot: arboard is built for X11 only, so a
/// Wayland session without X11 is pointed to `wl-copy`.
fn no_display(display: Option<&OsStr>, wayland: Option<&OsStr>) -> Option<&'static str> {
    let set = |value: Option<&OsStr>| value.is_some_and(|value| !value.is_empty());
    if set(display) {
        None
    } else if set(wayland) {
        Some("no X11 display: DISPLAY is not set; on Wayland, pipe --stdout into wl-copy")
    } else {
        Some("no display: DISPLAY is not set")
    }
}

/// How long `handoff` waits for the background copy to own the clipboard.
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(5);

/// The line the background copy prints once it owns the clipboard. Anything
/// else it prints is why it could not.
const READY: &str = "ok";

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
            seconds(timeout)
        ),
    })
}

/// `duration` in whole seconds, such as `5 seconds`.
fn seconds(duration: Duration) -> String {
    count(
        usize::try_from(duration.as_secs()).unwrap_or(usize::MAX),
        "second",
    )
}

/// `serve-clipboard`: puts what `input` holds on the clipboard, prints
/// [`READY`] to `out`, then on X11 keeps serving it (see [`hold`]). A
/// failure is printed as `error: ...` instead.
pub fn serve(input: &mut dyn Read, out: &mut dyn Write) -> Result<()> {
    let mut text = String::new();
    if let Err(error) = input.read_to_string(&mut text) {
        return report(
            out,
            Err(format!("could not read the text to copy: {error}")),
        );
    }
    serve_text(text, out)
}

/// The longest a `serve-clipboard` process serves what it copied. Past it,
/// the text goes to the clipboard manager if there is one, and is gone
/// otherwise.
const MAX_SERVE: Duration = Duration::from_secs(12 * 60 * 60);

/// How a `serve-clipboard` process waits.
#[derive(Debug, Clone, Copy)]
struct Timings {
    /// How long it waits to own the clipboard; less than
    /// [`HANDOVER_TIMEOUT`], so that it says why it could not.
    confirm: Duration,
    /// How often it looks while it waits to own it.
    poll: Duration,
    /// How often it checks that it still owns it, which finds a lost X
    /// connection, which arboard does not report.
    check: Duration,
}

const TIMINGS: Timings = Timings {
    confirm: Duration::from_secs(3),
    poll: Duration::from_millis(20),
    check: Duration::from_secs(10 * 60),
};

/// Takes the X11 clipboard once, on a thread that then waits until another
/// program takes it or [`MAX_SERVE`] passes, while arboard serves `text`.
/// Another handle checks that this process owns it before [`READY`] is
/// printed, and now and then afterwards.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
))]
fn serve_text(text: String, out: &mut dyn Write) -> Result<()> {
    use arboard::SetExtLinux;
    // Made first, so that arboard's connection outlives the thread's handle.
    let mut probe = match arboard::Clipboard::new() {
        Ok(probe) => probe,
        Err(error) => return report(out, Err(error.to_string())),
    };
    let (sender, done) = mpsc::channel();
    let owned = text.clone();
    let deadline = Instant::now() + MAX_SERVE;
    thread::spawn(move || {
        let result = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set().wait_until(deadline).text(owned))
            .map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    // While this process owns the clipboard, arboard answers from what it
    // holds. Before, it reads what another program holds: if that is
    // already `text`, "copied" is just as true.
    let mut owns = || probe.get_text().is_ok_and(|held| held == text);
    hold(&mut owns, &done, out, &TIMINGS)
}

/// Copies `text` where the system keeps it, and says so.
#[cfg(not(all(
    unix,
    not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
)))]
fn serve_text(text: String, out: &mut dyn Write) -> Result<()> {
    report(out, copy(&text))
}

/// Waits until `owns` says this process holds the text on the clipboard,
/// then prints [`READY`] and waits until `done` hears that the thread
/// holding it stopped (another program took the clipboard, or the deadline
/// passed) or `owns` no longer holds, checked every `timings.check`. If it
/// does not own it within `timings.confirm`, or `done` hears first, it
/// prints why instead.
fn hold(
    owns: &mut dyn FnMut() -> bool,
    done: &Receiver<Result<(), String>>,
    out: &mut dyn Write,
    timings: &Timings,
) -> Result<()> {
    let start = Instant::now();
    loop {
        // The first look comes after the thread had time to take it.
        thread::sleep(timings.poll);
        match done.try_recv() {
            Ok(Err(reason)) => return report(out, Err(reason)),
            Ok(Ok(())) => {
                return report(out, Err("something else was copied at once".to_string()));
            }
            Err(TryRecvError::Disconnected) => {
                return report(out, Err("the clipboard thread stopped".to_string()));
            }
            Err(TryRecvError::Empty) => {}
        }
        if owns() {
            break;
        }
        if start.elapsed() >= timings.confirm {
            return report(
                out,
                Err(format!(
                    "could not take the clipboard within {}",
                    seconds(timings.confirm)
                )),
            );
        }
    }
    report(out, Ok(()))?;
    loop {
        match done.recv_timeout(timings.check) {
            Ok(_) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) if !owns() => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Prints [`READY`] or `error: <reason>` to `out` for `handoff` to read.
fn report(out: &mut dyn Write, result: Result<(), String>) -> Result<()> {
    match &result {
        Ok(()) => writeln!(out, "{READY}")?,
        Err(reason) => writeln!(out, "error: {reason}")?,
    }
    out.flush()?;
    result.map_err(|reason| anyhow!(reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn only_an_x11_display_will_do() {
        let os = |value| Some(OsStr::new(value));
        assert_eq!(no_display(os(":0"), None), None);
        assert_eq!(no_display(os(":0"), os("wayland-0")), None);
        let wayland = no_display(None, os("wayland-0")).unwrap();
        assert!(wayland.contains("wl-copy"), "{wayland}");
        assert_eq!(no_display(os(""), os("wayland-0")), Some(wayland));
        let none = no_display(None, None).unwrap();
        assert_eq!(none, "no display: DISPLAY is not set");
        assert_eq!(no_display(os(""), os("")), Some(none));
    }

    /// A stand-in for `serve-clipboard` or pbcopy that runs `script` with
    /// the text on its stdin.
    #[cfg(unix)]
    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[cfg(unix)]
    #[test]
    fn pbcopy_gets_the_text_as_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let copied = dir.path().join("copied");
        let locale = dir.path().join("locale");
        let mut command = pbcopy("/bin/sh");
        command.args([
            "-c",
            &format!(
                "printf '%s|%s' \"$LC_CTYPE\" \"${{LC_ALL-unset}}\" > '{}'; cat > '{}'",
                locale.display(),
                copied.display()
            ),
        ]);
        let text = format!("héllo — 日本\n{}", "line\n".repeat(50_000));
        pipe_into(command, &text).unwrap();
        assert_eq!(std::fs::read(&copied).unwrap(), text.as_bytes());
        // Whatever locale this process has, pbcopy is told UTF-8.
        assert_eq!(std::fs::read_to_string(locale).unwrap(), "UTF-8|unset");
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_pbcopy_is_an_error() {
        let reason = pipe_into(shell("cat >/dev/null; exit 3"), "text").unwrap_err();
        assert_eq!(reason, "/bin/sh failed (exit status: 3)");
        // One that stops reading at once fails as well.
        let reason = pipe_into(shell("exit 1"), &"x".repeat(1 << 20)).unwrap_err();
        assert_eq!(reason, "/bin/sh failed (exit status: 1)");
        let reason = pipe_into(Command::new("/nonexistent/pbcopy"), "text").unwrap_err();
        assert!(
            reason.starts_with("could not run /nonexistent/pbcopy: "),
            "{reason}"
        );
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
        let started = Instant::now();
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

    const FAST: Timings = Timings {
        confirm: Duration::from_secs(1),
        poll: Duration::from_millis(1),
        check: Duration::from_millis(5),
    };

    /// Runs [`hold`] with `owns` answering in turn, and `done` fed by
    /// `thread`; returns what it printed and whether it succeeded.
    fn held(
        owns: &[bool],
        thread: impl FnOnce(mpsc::Sender<Result<(), String>>) + Send + 'static,
    ) -> (String, bool, usize) {
        let (sender, done) = mpsc::channel();
        let handle = thread::spawn(move || thread(sender));
        let asked = Cell::new(0);
        let mut owns_now = || {
            let i = asked.get();
            asked.set(i + 1);
            owns.get(i).copied().unwrap_or(*owns.last().unwrap())
        };
        let mut out = Vec::new();
        let result = hold(&mut owns_now, &done, &mut out, &FAST);
        drop(done);
        handle.join().unwrap();
        (String::from_utf8(out).unwrap(), result.is_ok(), asked.get())
    }

    #[test]
    fn it_is_ready_once_it_owns_the_clipboard_then_serves_until_replaced() {
        // Owned at the third look; replaced 50 ms later.
        let (out, ok, _) = held(&[false, false, true], |sender| {
            thread::sleep(Duration::from_millis(50));
            sender.send(Ok(())).unwrap();
        });
        assert_eq!(out, "ok\n");
        assert!(ok);
    }

    #[test]
    fn it_stops_serving_once_it_no_longer_owns_the_clipboard() {
        // The thread never hears that it lost it, as when the X connection
        // is gone: the check finds it.
        let (out, ok, asked) = held(&[true, true, false], |sender| {
            thread::sleep(Duration::from_millis(300));
            drop(sender);
        });
        assert_eq!(out, "ok\n");
        assert!(ok);
        assert_eq!(asked, 3);
    }

    #[test]
    fn it_says_why_it_could_not_own_the_clipboard() {
        let (out, ok, _) = held(&[false], |sender| {
            sender.send(Err("clipboard occupied".to_string())).unwrap();
        });
        assert_eq!(out, "error: clipboard occupied\n");
        assert!(!ok);

        let (out, ok, _) = held(&[false], |sender| sender.send(Ok(())).unwrap());
        assert_eq!(out, "error: something else was copied at once\n");
        assert!(!ok);

        let (out, ok, _) = held(&[false], drop);
        assert_eq!(out, "error: the clipboard thread stopped\n");
        assert!(!ok);

        let (out, ok, asked) = held(&[false], |sender| {
            thread::sleep(Duration::from_millis(1500));
            drop(sender);
        });
        assert_eq!(out, "error: could not take the clipboard within 1 second\n");
        assert!(!ok);
        assert!(asked > 1);
    }
}

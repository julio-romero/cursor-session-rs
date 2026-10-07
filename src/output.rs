use cursor_session::ui;

use crate::cli::ColorChoice;

/// Rendering decisions for stdout, resolved once at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputOpts {
    pub tty: bool,
    pub color: bool,
    pub width: usize,
}

impl OutputOpts {
    pub fn detect(_choice: ColorChoice) -> OutputOpts {
        let tty = ui::stdout_is_tty();
        OutputOpts {
            tty,
            color: tty,
            width: ui::terminal_width(),
        }
    }
}

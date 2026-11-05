//! Full-screen views built on ratatui. Only started by interactive commands.

pub mod branches;
mod diffview;
pub mod log;
pub mod rebase;
pub mod stash;
mod stash_new;
pub mod theme;
pub mod widgets;

use std::process::{Command, Stdio};

use ratatui::DefaultTerminal;

/// Full screen with mouse reporting on.
pub(crate) fn enter() -> DefaultTerminal {
    let t = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    t
}

/// Back to the normal screen.
pub(crate) fn leave() {
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();
}

/// Runs git, returning whether it worked and its output (stderr first; that's where git talks).
/// Prompts for credentials still reach the terminal: ssh and git ask on /dev/tty.
pub(crate) fn git(args: &[String]) -> (bool, String) {
    match Command::new("git").args(args).stdin(Stdio::inherit()).output() {
        Ok(out) => {
            let text = format!("{}{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
            (out.status.success(), text.trim().to_string())
        }
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// First two non-empty lines of git's output, for a toast.
pub(crate) fn first_lines(s: &str) -> String {
    s.lines().filter(|l| !l.trim().is_empty()).take(2).collect::<Vec<_>>().join(" · ")
}

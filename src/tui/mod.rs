//! Full-screen views built on ratatui. Only started by interactive commands.

pub mod branches;
mod diffview;
pub mod log;
pub mod rebase;
pub mod stash;
mod stash_new;
pub mod theme;
pub mod widgets;
pub mod worktree;

use std::io::{IsTerminal, Write};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

pub(crate) type Term = Terminal<CrosstermBackend<Box<dyn Write>>>;

/// Where screens draw: stdout, or the terminal itself when stdout is captured,
/// so `$(gg log -i)` and `cd $(gg wt)` show the screen and capture only the answer.
fn screen() -> Box<dyn Write> {
    if !std::io::stdout().is_terminal()
        && let Ok(tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty")
    {
        return Box::new(tty);
    }
    Box::new(std::io::stdout())
}

/// Full screen with mouse reporting on.
pub(crate) fn enter() -> Term {
    // Put the terminal back if we panic, so the message is readable.
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            leave();
            prev(info);
        }));
    });
    let _ = enable_raw_mode();
    let mut out = screen();
    let _ = crossterm::execute!(out, EnterAlternateScreen, EnableMouseCapture);
    Terminal::new(CrosstermBackend::new(out)).expect("terminal size")
}

/// Back to the normal screen.
pub(crate) fn leave() {
    let _ = crossterm::execute!(screen(), DisableMouseCapture, LeaveAlternateScreen, crossterm::cursor::Show);
    let _ = disable_raw_mode();
}

/// Runs git, returning whether it worked and its output (stderr first; that's where git talks).
/// Prompts for credentials still reach the terminal: ssh and git ask on /dev/tty.
pub(crate) fn git(args: &[String]) -> (bool, String) {
    match crate::process::run("git", args, None) {
        Ok(out) => (out.ok, out.text()),
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

/// First two non-empty lines of git's output, for a toast.
pub(crate) fn first_lines(s: &str) -> String {
    s.lines().filter(|l| !l.trim().is_empty()).take(2).collect::<Vec<_>>().join(" · ")
}

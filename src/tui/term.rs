//! Taking over the terminal and giving it back.

use std::io::{IsTerminal, Write};

use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
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

/// Full screen with mouse reporting on. A paste comes as one event (which screens
/// ignore), not as keys: a newline in it would press Enter.
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
    let _ = crossterm::execute!(out, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste);
    Terminal::new(CrosstermBackend::new(out)).expect("terminal size")
}

/// Back to the normal screen.
pub(crate) fn leave() {
    let _ = crossterm::execute!(
        screen(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    let _ = disable_raw_mode();
}

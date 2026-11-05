//! Turns data from `core` into printed text.

pub mod log;
pub mod status;

use std::io::IsTerminal;

use owo_colors::Style;

use crate::cli::ColorMode;

/// Colour styles, all no-ops when colour is off (piped output, NO_COLOR, --color never).
pub struct Paint {
    pub on: bool,
}

impl Paint {
    pub fn detect(mode: ColorMode) -> Self {
        let on = match mode {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        };
        Self { on }
    }

    /// Use for combined styles; never chain onto the result of the helpers below,
    /// or a "colour off" style turns back on.
    pub fn pick(&self, s: Style) -> Style {
        if self.on { s } else { Style::new() }
    }

    pub fn green(&self) -> Style { self.pick(Style::new().green()) }
    pub fn red(&self) -> Style { self.pick(Style::new().red()) }
    pub fn yellow(&self) -> Style { self.pick(Style::new().yellow()) }
    pub fn blue(&self) -> Style { self.pick(Style::new().blue()) }
    pub fn magenta(&self) -> Style { self.pick(Style::new().magenta()) }
    pub fn dim(&self) -> Style { self.pick(Style::new().dimmed()) }
    /// Current branch name.
    pub fn head(&self) -> Style { self.pick(Style::new().bold().cyan()) }
}

/// "3m ago", "2h ago", "5d ago", "4w ago", "7mo ago", "2y ago".
pub fn relative_time(then: i64, now: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{}m ago", s / 60),
        3600..86400 => format!("{}h ago", s / 3600),
        86400..604800 => format!("{}d ago", s / 86400),
        604800..2_592_000 => format!("{}w ago", s / 604800),
        2_592_000..31_536_000 => format!("{}mo ago", s / 2_592_000),
        _ => format!("{}y ago", s / 31_536_000),
    }
}

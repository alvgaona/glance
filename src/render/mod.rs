//! Turns data from `core` into printed text.

pub mod branches;
pub mod log;
pub mod repos;
pub mod stash;
pub mod status;
pub mod worktree;

use std::io::IsTerminal;

use owo_colors::Style;

use crate::cli::ColorMode;

/// Colour styles, all no-ops when colour is off (piped output, NO_COLOR, --color never).
pub struct Paint {
    pub on: bool,
    /// The terminal takes 24-bit colour (COLORTERM=truecolor), so the theme's exact colours work.
    pub rgb: bool,
}

/// The palette of the full-screen views (see `tui::theme`), for printed output.
#[derive(Clone, Copy)]
pub enum Tone {
    Accent,
    Blue,
    Purple,
    Green,
    Red,
    Yellow,
    Text,
    Muted,
    Faint,
}

impl Tone {
    fn rgb(self) -> (u8, u8, u8) {
        match self {
            Tone::Accent => (250, 178, 131),
            Tone::Blue => (92, 156, 245),
            Tone::Purple => (157, 124, 216),
            Tone::Green => (127, 216, 143),
            Tone::Red => (224, 108, 117),
            Tone::Yellow => (229, 192, 123),
            Tone::Text => (238, 238, 238),
            Tone::Muted => (128, 128, 128),
            Tone::Faint => (80, 80, 80),
        }
    }

    /// Nearest of the 16 basic colours, for terminals without 24-bit colour.
    fn basic(self) -> Style {
        match self {
            Tone::Accent => Style::new().yellow(),
            Tone::Blue => Style::new().blue(),
            Tone::Purple => Style::new().magenta(),
            Tone::Green => Style::new().green(),
            Tone::Red => Style::new().red(),
            Tone::Yellow => Style::new().bright_yellow(),
            Tone::Text => Style::new(),
            Tone::Muted => Style::new().dimmed(),
            Tone::Faint => Style::new().bright_black(),
        }
    }
}

impl Paint {
    pub fn detect(mode: ColorMode) -> Self {
        let on = match mode {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        };
        let rgb = std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit");
        Self { on, rgb }
    }

    /// Text in a theme colour.
    pub fn tone(&self, t: Tone) -> Style {
        if !self.on {
            return Style::new();
        }
        if self.rgb {
            let (r, g, b) = t.rgb();
            Style::new().truecolor(r, g, b)
        } else {
            t.basic()
        }
    }

    /// Bold text in a theme colour.
    pub fn tone_bold(&self, t: Tone) -> Style {
        if self.on { self.tone(t).bold() } else { Style::new() }
    }

    /// A tag: theme-coloured text on the raised panel colour. Plain bold without 24-bit colour.
    pub fn tag(&self, t: Tone) -> Style {
        if !self.on {
            Style::new()
        } else if self.rgb {
            self.tone(t).bold().on_truecolor(32, 32, 32)
        } else {
            t.basic().bold().reversed()
        }
    }

    /// Use for combined styles; never chain onto the result of the helpers below,
    /// or a "colour off" style turns back on.
    pub fn pick(&self, s: Style) -> Style {
        if self.on { s } else { Style::new() }
    }

    pub fn green(&self) -> Style {
        self.pick(Style::new().green())
    }
    pub fn red(&self) -> Style {
        self.pick(Style::new().red())
    }
    pub fn yellow(&self) -> Style {
        self.pick(Style::new().yellow())
    }
    pub fn blue(&self) -> Style {
        self.pick(Style::new().blue())
    }
    pub fn magenta(&self) -> Style {
        self.pick(Style::new().magenta())
    }
    pub fn dim(&self) -> Style {
        self.pick(Style::new().dimmed())
    }
    /// Current branch name.
    pub fn head(&self) -> Style {
        self.pick(Style::new().bold().cyan())
    }
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

//! One colour theme for every full-screen view (and the fzf picker).
//! Dark, borderless: panels stand out by background, one warm accent.

use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Rgb(10, 10, 10);
/// Panels (lists, details) sit on this.
pub const PANEL: Color = Color::Rgb(20, 20, 20);
/// Selected rows, inputs, dialogs.
pub const ELEMENT: Color = Color::Rgb(32, 32, 32);
pub const BORDER: Color = Color::Rgb(48, 48, 48);

pub const TEXT: Color = Color::Rgb(238, 238, 238);
pub const MUTED: Color = Color::Rgb(128, 128, 128);
pub const FAINT: Color = Color::Rgb(80, 80, 80);

pub const ACCENT: Color = Color::Rgb(250, 178, 131);
pub const BLUE: Color = Color::Rgb(92, 156, 245);
pub const PURPLE: Color = Color::Rgb(157, 124, 216);
pub const GREEN: Color = Color::Rgb(127, 216, 143);
pub const RED: Color = Color::Rgb(224, 108, 117);
pub const YELLOW: Color = Color::Rgb(229, 192, 123);

pub fn text() -> Style {
    Style::new().fg(TEXT)
}

pub fn muted() -> Style {
    Style::new().fg(MUTED)
}

pub fn faint() -> Style {
    Style::new().fg(FAINT)
}

pub fn bold() -> Style {
    Style::new().fg(TEXT).add_modifier(Modifier::BOLD)
}

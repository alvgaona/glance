//! Where floating boxes go and how panels look.

use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Padding},
};

/// Area of `w`×`h` centred in `area`, clamped to fit.
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let [r] = Layout::horizontal([Constraint::Length(w.min(area.width))]).flex(Flex::Center).areas(area);
    let [r] = Layout::vertical([Constraint::Length(h.min(area.height))]).flex(Flex::Center).areas(r);
    r
}

/// A panel: background colour and inner padding, no border.
pub fn panel(bg: Color) -> Block<'static> {
    Block::new().style(Style::new().bg(bg)).padding(Padding::new(2, 2, 1, 1))
}

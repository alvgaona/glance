//! A short message in the top-right corner that goes away on its own.

use std::time::{Duration, Instant};

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};

use crate::tui::theme;

pub struct Toast {
    text: String,
    color: Color,
    until: Instant,
}

impl Toast {
    pub fn new(text: impl Into<String>, color: Color) -> Self {
        Self { text: text.into(), color, until: Instant::now() + Duration::from_millis(2800) }
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.until
    }

    /// Top-right corner, with a coloured bar on the left.
    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let width = (self.text.chars().count() as u16 + 6).min(area.width.saturating_sub(4)).max(10);
        let lines = self.text.chars().count() as u16 / width.saturating_sub(6).max(1) + 1;
        let r = Rect { x: area.right().saturating_sub(width + 2), y: area.y + 1, width, height: lines + 2 }
            .intersection(area);
        f.render_widget(Clear, r);
        let bar = Block::new()
            .style(Style::new().bg(theme::ELEMENT))
            .borders(Borders::LEFT)
            .border_set(ratatui::symbols::border::THICK)
            .border_style(Style::new().fg(self.color).bg(theme::ELEMENT))
            .padding(Padding::new(1, 1, 1, 1));
        f.render_widget(
            Paragraph::new(Span::styled(self.text.clone(), theme::text())).wrap(Wrap { trim: true }).block(bar),
            r,
        );
    }
}

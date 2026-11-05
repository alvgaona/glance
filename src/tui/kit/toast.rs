//! A short message in the top-right corner that goes away on its own.

use std::time::{Duration, Instant};

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Text,
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

    /// Green `ok` when the write worked, else red with the first lines of what went wrong.
    pub fn ran(ran: &Result<String, String>, ok: impl Into<String>) -> Self {
        match ran {
            Ok(_) => Self::new(ok, theme::GREEN),
            Err(e) => Self::new(crate::tui::first_lines(e), theme::RED),
        }
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.until
    }

    /// Top-right corner, with a coloured bar on the left.
    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let longest = self.text.lines().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
        let width = (longest + 5).min(area.width.saturating_sub(4)).max(10);
        let bar = Block::new()
            .style(Style::new().bg(theme::ELEMENT))
            .borders(Borders::LEFT)
            .border_set(ratatui::symbols::border::THICK)
            .border_style(Style::new().fg(self.color).bg(theme::ELEMENT))
            .padding(Padding::new(1, 1, 1, 1));
        // `Text`, not `Span`: a message of several lines (git's errors) keeps its breaks.
        let p = Paragraph::new(Text::styled(self.text.clone(), theme::text())).wrap(Wrap { trim: true }).block(bar);
        // The bar and the side padding take 3 columns.
        let height = p.line_count(width.saturating_sub(3)) as u16;
        let r = Rect { x: area.right().saturating_sub(width + 2), y: area.y + 1, width, height }.intersection(area);
        f.render_widget(Clear, r);
        f.render_widget(p, r);
    }
}

//! A centred yes/no question.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use super::layout::{centered, panel};
use super::text::pill;
use crate::tui::theme;

/// Enter confirms, Esc cancels.
pub struct Confirm {
    pub title: String,
    pub body: String,
    pub yes: String,
    pub no: String,
    pub danger: bool,
}

impl Confirm {
    /// `body` may have several lines; lines starting with `$ ` are shown as commands.
    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let w: u16 = 64;
        let inner_w = (w - 4) as usize;
        let body_rows: u16 = self.body.lines().map(|l| (l.chars().count().max(1)).div_ceil(inner_w) as u16).sum();
        let r = centered(area, w, body_rows + 7);
        f.render_widget(Clear, r);
        let yes_color = if self.danger { theme::RED } else { theme::ACCENT };
        let mut text = vec![Line::styled(self.title.clone(), theme::bold()), Line::raw("")];
        for l in self.body.lines() {
            text.push(match l.strip_prefix("$ ") {
                Some(cmd) => Line::styled(format!("  {cmd}"), Style::new().fg(theme::ACCENT)),
                None => Line::styled(l.to_string(), theme::muted()),
            });
        }
        text.push(Line::raw(""));
        text.push(Line::from(vec![
            pill(&format!("enter  {}", self.yes), yes_color, theme::BG),
            Span::raw("  "),
            pill(&format!("esc  {}", self.no), theme::BORDER, theme::TEXT),
        ]));
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }).block(panel(theme::ELEMENT)), r);
    }
}

//! The bottom line of every screen: a status on the left, key hints on the right.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::text::truncate;
use crate::tui::theme;

/// "key label" pairs, key bright and label muted: `ctrl+p commands   w apply`.
pub fn hints(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(key.to_string(), theme::text()));
        spans.push(Span::styled(format!(" {label}"), theme::muted()));
    }
    Line::from(spans)
}

/// The search being typed (or kept): `/ query▏   3 of 40`.
pub fn search(query: &str, typing: bool, found: usize, total: usize) -> Line<'static> {
    let mut spans =
        vec![Span::styled("/ ", Style::new().fg(theme::ACCENT)), Span::styled(query.to_string(), theme::text())];
    if typing {
        spans.push(Span::styled("▏", Style::new().fg(theme::ACCENT)));
    }
    spans.push(Span::styled(format!("   {found} of {total}"), theme::muted()));
    Line::from(spans)
}

/// `left`, then the hints on the right. List hints most important first: when the
/// line is too narrow the ones before the last go first, so "q quit" always stays.
pub fn draw(f: &mut Frame, area: Rect, left: Line<'static>, mut pairs: Vec<(&str, &str)>) {
    let room = (area.width as usize).saturating_sub(if left.width() > 0 { left.width() + 3 } else { 0 });
    while pairs.len() > 2 && hints(&pairs).width() > room {
        pairs.remove(pairs.len() - 2);
    }
    let right = hints(&pairs);
    let [l, r] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(truncate(left, l.width)), l);
    f.render_widget(Paragraph::new(right), r);
}

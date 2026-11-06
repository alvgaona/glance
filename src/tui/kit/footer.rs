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
    let pairs: Vec<[Span<'static>; 2]> = pairs
        .iter()
        .map(|(key, label)| {
            [Span::styled(key.to_string(), theme::text()), Span::styled(format!(" {label}"), theme::muted())]
        })
        .collect();
    Line::from(pairs.join(&Span::raw("   ")))
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

/// Adds `i sidebar` before the last hint, on screens wide enough for one.
pub fn sidebar(pairs: &mut Vec<(&str, &str)>, width: u16) {
    if width >= super::layout::SIDE_FROM {
        pairs.insert(pairs.len().saturating_sub(1), ("i", "sidebar"));
    }
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

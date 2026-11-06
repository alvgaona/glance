//! The sidebar on wide screens: titled sections, one under the other. Where it goes
//! and when it shows is `layout::panes`; `i` turns it on and off on every screen.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::theme;

pub struct Section {
    /// Empty: no title row.
    pub title: &'static str,
    /// Shown muted after the title, like `hunks   3`.
    pub count: Option<usize>,
    pub lines: Vec<Line<'static>>,
}

/// Draws the sections in `area`; gives where each one's lines ended up, for clicks.
pub fn draw(f: &mut Frame, area: Rect, sections: Vec<Section>) -> Vec<Rect> {
    let block = super::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines: Vec<Line> = Vec::new();
    let mut at = Vec::new();
    for (i, s) in sections.into_iter().enumerate() {
        if i > 0 {
            lines.push(Line::raw(""));
        }
        if !s.title.is_empty() {
            let mut title = vec![Span::styled(s.title, theme::bold())];
            title.extend(s.count.map(|n| Span::styled(format!("   {n}"), theme::muted())));
            lines.push(Line::from(title));
            lines.push(Line::raw(""));
        }
        let top = lines.len() as u16;
        let shown = (s.lines.len() as u16).min(inner.height.saturating_sub(top));
        at.push(Rect { y: inner.y + top.min(inner.height), height: shown, ..inner });
        lines.extend(s.lines);
    }
    let lines: Vec<Line> = lines.into_iter().map(|l| super::truncate(l, inner.width)).collect();
    f.render_widget(Paragraph::new(lines), inner);
    at
}

/// One hunk in an outline: `┃    39  impl Store {`, the bar on the current one.
pub fn outline_row(line: u32, context: &str, on: bool) -> Line<'static> {
    let bar = if on { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };
    let context = if context.is_empty() { "top of the file".to_string() } else { context.to_string() };
    Line::from(vec![
        bar,
        Span::styled(format!("{line:>5}  "), theme::faint()),
        Span::styled(context, if on { theme::text() } else { theme::muted() }),
    ])
}

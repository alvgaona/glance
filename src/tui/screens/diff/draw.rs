//! Drawing the diff screen: header, file list, the diff, footer.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Modifier,
    text::Span,
    widgets::{Block, Padding, Paragraph},
};

use super::*;
use crate::git::diff::Size;
use crate::tui::kit::{self, footer};
use crate::tui::panels::changes::{self, Item};

pub(super) fn draw(f: &mut Frame, app: &mut App) {
    let full = f.area();
    f.render_widget(Block::new().style(Style::new().bg(theme::BG)), full);
    let area = Rect { x: full.x + 1, width: full.width.saturating_sub(2), ..full };
    let [header, _, body, _, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, app, header);
    if app.hide_files || app.files.is_empty() {
        app.list_area = Rect::default();
        draw_diff(f, app, body);
    } else if body.width >= 100 {
        // As wide as the longest row, within a third or so of the screen.
        let longest = app.files.iter().map(|f| Item::stat(f).row().width() as u16 + 4).max().unwrap_or(0);
        let w = longest.min(body.width * 2 / 5).max(28);
        let [l, _, r] =
            Layout::horizontal([Constraint::Length(w), Constraint::Length(1), Constraint::Min(40)]).areas(body);
        draw_list(f, app, l);
        draw_diff(f, app, r);
    } else {
        let h = (app.visible.len() as u16 + 2).min(body.height / 3).max(4);
        let [t, _, b] =
            Layout::vertical([Constraint::Length(h), Constraint::Length(1), Constraint::Min(3)]).areas(body);
        draw_list(f, app, t);
        draw_diff(f, app, b);
    }
    draw_footer(f, app, foot);

    if let Some(p) = &app.palette {
        p.draw(f, full);
    }
    if let Some(t) = &app.toast {
        t.draw(f, full);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let (added, removed) = app.files.iter().fold((0, 0), |(a, r), f| match f.size {
        Some(Size::Lines { added, removed }) => (a + added, r + removed),
        _ => (a, r),
    });
    let n = app.files.len();
    let mut spans = vec![
        Span::styled("diff", Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.repo_name.clone(), theme::bold()),
        Span::raw("  "),
        Span::styled(app.target.label(), Style::new().fg(theme::ACCENT)),
        Span::styled(format!(" · {n} file{}  ", if n == 1 { "" } else { "s" }), theme::muted()),
    ];
    spans.extend(changes::counts(Some(Size::Lines { added, removed })));
    f.render_widget(Paragraph::new(kit::truncate(Line::from(spans), area.width)), area);
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(0, 1, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;
    let height = inner.height as usize;
    app.scroll = list::follow(app.scroll, app.selected, app.selected, height);
    if app.visible.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("  No files match", theme::muted())), inner);
        return;
    }
    for (row, &i) in app.visible.iter().enumerate().skip(app.scroll).take(height) {
        let r = Rect { y: inner.y + (row - app.scroll) as u16, height: 1, ..inner };
        let selected = row == app.selected;
        let bar = if selected { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };
        let mut line = Item::stat(&app.files[i]).row();
        line.spans.insert(0, bar);
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };
        f.render_widget(Paragraph::new(kit::truncate(line, r.width)).style(Style::new().bg(bg)), r);
    }
}

fn draw_diff(f: &mut Frame, app: &mut App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.detail_area = area;
    app.width = inner.width;
    let Some(file) = app.current() else {
        let why = if app.files.is_empty() { format!("No {} changes", app.target.label()) } else { String::new() };
        f.render_widget(Paragraph::new(Span::styled(why, theme::muted())), inner);
        return;
    };

    // The file's name on top: delta is told not to print it.
    let [title, _, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)]).areas(inner);
    let mut name = Item::stat(file).row();
    if let Some(s) = name.spans.get_mut(1) {
        s.style = theme::bold();
    }
    f.render_widget(Paragraph::new(kit::truncate(name, title.width)), title);

    // Another file starts at the top.
    let path = Some(file.path.clone());
    if path != app.detail_for {
        app.detail_for = path;
        app.detail_scroll.reset();
    }
    let Some(lines) = app.key_for(inner.width).and_then(|k| app.cache.get(&k)) else {
        f.render_widget(Paragraph::new(Span::styled("loading…", theme::faint())), body);
        return;
    };
    app.detail_scroll.fit(lines.len(), body.height);
    let shown: Vec<Line> = lines
        .iter()
        .skip(app.detail_scroll.top() as usize)
        .take(body.height as usize)
        .map(|l| kit::truncate(l.clone(), body.width))
        .collect();
    f.render_widget(Paragraph::new(shown), body);
    app.detail_scroll.draw_more(f, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let left = if app.searching || !app.query.is_empty() {
        footer::search(&app.query, app.searching, app.visible.len(), app.files.len())
    } else {
        // What's on, so the toggles are visible.
        let mut on = Vec::new();
        if app.delta {
            on.push(if app.split_at(app.width) { "side by side" } else { "unified" });
            if app.numbers {
                on.push("line numbers");
            }
        }
        if app.cut.whitespace {
            on.push("ignoring whitespace");
        }
        let context = format!("{} lines of context", app.cut.context);
        on.push(if app.cut.full { "whole file" } else { &context });
        Line::styled(on.join(" · "), theme::faint())
    };
    let mut pairs = vec![("1 2 3", "compare")];
    if app.delta {
        pairs.extend([("s", "split"), ("l", "numbers")]);
    }
    pairs.extend([
        ("w", "whitespace"),
        ("+ -", "context"),
        ("f", "whole file"),
        ("z", "files"),
        ("e", "edit"),
        ("ctrl+p", "commands"),
        ("q", "quit"),
    ]);
    if app.searching {
        pairs = vec![("enter", "keep filter"), ("esc", "clear")];
    }
    footer::draw(f, area, left, pairs);
}

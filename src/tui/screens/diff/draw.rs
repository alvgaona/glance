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
    // The list as wide as its longest row; the sidebar on wide screens.
    let want = (!app.hide_files && !app.files.is_empty())
        .then(|| app.files.iter().map(|f| Item::stat(f).row().width() as u16 + 4).max().unwrap_or(0));
    let p = kit::panes(body, want, app.visible.len() as u16 + 2, app.side && !app.files.is_empty());
    app.side_open = p.side.is_some();
    app.outline_area = Rect::default();
    if let Some(side) = p.side {
        draw_side(f, app, side);
    }
    app.list_area = Rect::default();
    if want.is_some() {
        draw_list(f, app, p.list);
    }
    draw_diff(f, app, p.main);
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
    // `git diff` leaves new files out; say so, or they look lost.
    if app.untracked > 0 {
        spans.push(Span::styled(format!(" · {} untracked, not shown", app.untracked), theme::faint()));
    }
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
    // Until a new option's render is in, the last one of this file stays up.
    let want = app.key_for(inner.width).filter(|k| app.cache.contains_key(k));
    let key = want.or_else(|| app.shown.clone().filter(|k| app.detail_for.as_ref() == Some(&k.path)));
    let Some(lines) = key.as_ref().and_then(|k| app.cache.get(k)).map(|r| &r.lines) else {
        f.render_widget(Paragraph::new(Span::styled("loading…", theme::faint())), body);
        return;
    };
    app.shown = key;
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

/// The commit (for `glance show`), the file's hunks, and the commits that changed it.
fn draw_side(f: &mut Frame, app: &mut App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;
    let title = |t: String| Line::styled(t, theme::bold());
    let now = crate::fmt::now();
    let mut lines: Vec<Line> = Vec::new();

    if let Some(d) = &app.details {
        lines.extend(kit::wrap(&d.subject, w).into_iter().map(|l| Line::styled(l, theme::bold())));
        lines.push(Line::styled(format!("{} · {}", d.author, crate::fmt::relative_time(d.time, now)), theme::muted()));
        if !d.body.is_empty() {
            lines.push(Line::raw(""));
            let body = d.body.lines().flat_map(|l| kit::wrap(l, w)).take(12);
            lines.extend(body.map(|l| Line::styled(l, theme::muted())));
        }
        lines.push(Line::raw(""));
    }

    let hunks = app.on_screen().map(|r| r.hunks.clone()).unwrap_or_default();
    let here = app.hunk_here();
    lines.push(Line::from(vec![
        Span::styled("hunks", theme::bold()),
        Span::styled(format!("   {}", hunks.len()), theme::muted()),
    ]));
    lines.push(Line::raw(""));
    let first = lines.len() as u16;
    for (i, h) in hunks.iter().enumerate() {
        let on = here == Some(i);
        let bar = if on { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };
        let context = if h.context.is_empty() { "top of the file".to_string() } else { h.context.clone() };
        lines.push(Line::from(vec![
            bar,
            Span::styled(format!("{:>5}  ", h.line), theme::faint()),
            Span::styled(context, if on { theme::text() } else { theme::muted() }),
        ]));
    }
    let shown = (hunks.len() as u16).min(inner.height.saturating_sub(first));
    app.outline_area = Rect { y: inner.y + first, height: shown, ..inner };

    lines.push(Line::raw(""));
    lines.push(title("history".into()));
    lines.push(Line::raw(""));
    match app.current().and_then(|c| app.history.get(&c.path)) {
        None => lines.push(Line::styled("reading…", theme::faint())),
        Some(h) if h.is_empty() => lines.push(Line::styled("Not committed yet", theme::faint())),
        Some(h) => {
            for t in h {
                lines.push(Line::from(vec![
                    Span::styled(format!("{}  ", t.id), Style::new().fg(theme::ACCENT)),
                    Span::styled(t.subject.clone(), theme::text()),
                ]));
                let when = crate::fmt::relative_time(t.time, now);
                lines
                    .push(Line::styled(format!("{}{} · {when}", " ".repeat(t.id.len() + 2), t.author), theme::faint()));
            }
        }
    }
    let lines: Vec<Line> = lines.into_iter().map(|l| kit::truncate(l, inner.width)).collect();
    f.render_widget(Paragraph::new(lines), inner);
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
        ("n N", "hunks"),
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

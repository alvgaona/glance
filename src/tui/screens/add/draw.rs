//! Drawing the staging screen: header, file groups, diff, footer.

use super::*;

pub(super) fn draw(f: &mut Frame, app: &mut App) {
    let full = f.area();
    f.render_widget(Block::new().style(Style::new().bg(theme::BG)), full);
    let area = Rect { x: full.x + 1, width: full.width.saturating_sub(2), ..full };
    let [header, _, body, _, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, app, header);
    let (list, details) = if body.width >= 110 {
        let [l, _, r] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Length(1), Constraint::Min(40)]).areas(body);
        (l, r)
    } else {
        // Not `clamp`: it panics when a short terminal makes the upper bound the smaller one.
        let rows = (display(app).len() as u16 + 2).min(body.height / 2).max(4);
        let [t, _, b] =
            Layout::vertical([Constraint::Length(rows), Constraint::Length(1), Constraint::Min(5)]).areas(body);
        (t, b)
    };
    draw_list(f, app, list);
    draw_diff(f, app, details);
    draw_footer(f, app, footer);

    if let Some(p) = &app.palette {
        p.draw(f, full);
    }
    if let Some((c, _)) = &app.confirm {
        c.draw(f, full);
    }
    if let Some(t) = &app.toast {
        t.draw(f, full);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let count = |g: Group| app.entries.iter().filter(|e| e.group == g).count();
    let mut spans = vec![
        Span::styled("add", Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.repo_name.clone(), theme::bold()),
        Span::raw("  "),
    ];
    spans.push(match &app.st.branch {
        Some(b) => Span::styled(format!("on {b}"), theme::muted()),
        None => Span::styled("detached HEAD", Style::new().fg(theme::YELLOW)),
    });
    if let Some(op) = &app.st.in_progress {
        spans.push(Span::styled(format!(" · {op}"), Style::new().fg(theme::YELLOW)));
    }
    for (g, color, what) in [
        (Group::Conflicts, theme::RED, "conflicted"),
        (Group::Staged, theme::GREEN, "staged"),
        (Group::Changes, theme::YELLOW, "changed"),
        (Group::Untracked, theme::MUTED, "untracked"),
    ] {
        let n = count(g);
        if n > 0 {
            spans.push(Span::styled(" · ", theme::faint()));
            spans.push(Span::styled(format!("{n} {what}"), Style::new().fg(color)));
        }
    }
    f.render_widget(Paragraph::new(kit::truncate(Line::from(spans), area.width)), area);
}

/// The list as drawn: a header per group, its files, a gap. `Some(i)` rows are entries.
fn display(app: &App) -> Vec<Option<usize>> {
    let mut out = Vec::new();
    let mut last = None;
    for (i, e) in app.entries.iter().enumerate() {
        if last != Some(e.group) {
            if last.is_some() {
                out.push(None);
            }
            out.push(None);
            last = Some(e.group);
        }
        out.push(Some(i));
    }
    out
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(2, 1, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;

    if app.entries.is_empty() {
        app.shown.clear();
        let text = vec![
            Line::styled("Nothing to commit", theme::bold()),
            Line::styled("The working tree matches the last commit.", theme::muted()),
        ];
        f.render_widget(Paragraph::new(text), inner);
        return;
    }

    let rows = display(app);
    let height = inner.height as usize;
    let at = rows.iter().position(|r| *r == Some(app.selected)).unwrap_or(0);
    // The group's header comes along when its first file is selected.
    let first = if at >= 1 && rows[at - 1].is_none() { at - 1 } else { at };
    app.scroll = list::follow(app.scroll.min(rows.len().saturating_sub(1)), first, at, height);

    let mut lines = Vec::new();
    app.shown.clear();
    for (n, row) in rows.iter().enumerate().skip(app.scroll).take(height) {
        app.shown.push(*row);
        let line = match *row {
            Some(i) => {
                let e = &app.entries[i];
                let selected = i == app.selected;
                let mut spans =
                    vec![if selected { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") }];
                spans.extend(changes::row(&e.item()).spans);
                let line = Line::from(spans);
                if selected { line.style(Style::new().bg(theme::ELEMENT)) } else { line }
            }
            // A header is the row before a group's first file.
            None => match rows.get(n + 1) {
                Some(Some(i)) => {
                    let g = app.entries[*i].group;
                    let items: Vec<changes::Item> =
                        app.entries.iter().filter(|e| e.group == g).map(|e| e.item()).collect();
                    let mut h = changes::header(format!("{} {}", g.title(), items.len()), &items);
                    if g == Group::Conflicts {
                        h = h.style(Style::new().fg(theme::RED));
                    }
                    let mut spans = vec![Span::raw("  ")];
                    spans.extend(h.spans);
                    Line::from(spans)
                }
                _ => Line::raw(""),
            },
        };
        lines.push(kit::truncate(line, inner.width));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_diff(f: &mut Frame, app: &mut App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.detail_area = area;
    if app.entries.is_empty() {
        return;
    }
    let len = app.diff_lines().len();
    app.detail_scroll.fit(len, inner.height);
    let top = (app.detail_scroll.top() as usize).min(len);
    let shown: Vec<Line> = app.diff_lines()[top..]
        .iter()
        .take(inner.height as usize)
        .map(|l| kit::truncate(l.clone(), inner.width))
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
    app.detail_scroll.draw_more(f, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let toggle = match app.current().map(|e| e.group) {
        Some(Group::Staged) => "unstage",
        Some(Group::Conflicts) => "resolved",
        _ => "stage",
    };
    let pairs = if app.entries.is_empty() {
        vec![("ctrl+r", "reload"), ("q", "quit")]
    } else {
        vec![
            ("space", toggle),
            ("a", "stage all"),
            ("u", "unstage all"),
            ("d", "discard"),
            ("ctrl+p", "commands"),
            ("q", "quit"),
        ]
    };
    footer::draw(f, area, Line::default(), pairs);
}

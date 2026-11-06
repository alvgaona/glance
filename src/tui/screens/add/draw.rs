//! Drawing the staging screen: header, file groups, diff, footer.

use super::*;
use crate::git::diff::Size;
use crate::tui::kit::sidebar::{self, Section};

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
    let p = kit::panes(body, Some(list_width(app)), display(app).len() as u16 + 2, app.side && !app.entries.is_empty());
    let (list, details) = (p.list, p.main);
    app.outline_area = Rect::default();
    if let Some(side) = p.side {
        draw_side(f, app, side);
    }
    draw_list(f, app, list);
    draw_diff(f, app, details);
    draw_footer(f, app, footer);

    if let Some(p) = &app.palette {
        p.draw(f, full);
    }
    if let Some((c, _)) = &app.confirm {
        c.draw(f, full);
    }
    app.draw_commit(f, full);
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

/// The sidebar: the file's hunks, then what the next commit would be.
fn draw_side(f: &mut Frame, app: &mut App, area: Rect) {
    let view = app.view();
    let outline = view.outline();
    // The hunk worked on, or the one at the top of the diff.
    let top = app.detail_scroll.top() as usize;
    let here = if app.focus == Focus::Hunks { Some(app.hunk) } else { view.ranges.iter().position(|r| r.end > top) };
    let hunks = outline
        .iter()
        .enumerate()
        .map(|(i, (line, context))| sidebar::outline_row(*line, context, here == Some(i)))
        .collect();

    let mut next = Vec::new();
    next.push(match (&app.st.branch, &app.st.upstream) {
        (None, _) => Line::styled("detached HEAD", Style::new().fg(theme::YELLOW)),
        (Some(b), None) => Line::from(vec![
            Span::styled(format!("on {b}"), theme::text()),
            Span::styled("  no upstream", theme::faint()),
        ]),
        (Some(b), Some(up)) => {
            let mut spans = vec![Span::styled(format!("on {b}  "), theme::text())];
            spans.push(match (up.ahead, up.behind) {
                (0, 0) => Span::styled("=", theme::faint()),
                (a, 0) => Span::styled(format!("↑{a}"), Style::new().fg(theme::GREEN)),
                (0, b) => Span::styled(format!("↓{b}"), Style::new().fg(theme::RED)),
                (a, b) => Span::styled(format!("↑{a} ↓{b}"), Style::new().fg(theme::YELLOW)),
            });
            spans.push(Span::styled(format!(" {}", up.name), theme::faint()));
            Line::from(spans)
        }
    });
    if let Some(op) = &app.st.in_progress {
        next.push(Line::styled(format!("{op} in progress"), Style::new().fg(theme::YELLOW)));
    }
    let staged: Vec<changes::Item> =
        app.entries.iter().filter(|e| e.group == Group::Staged).map(|e| e.item()).collect();
    next.push(if staged.is_empty() {
        Line::styled("nothing staged yet", theme::muted())
    } else {
        let mut spans = vec![Span::styled(
            format!("{} file{}  ", staged.len(), if staged.len() == 1 { "" } else { "s" }),
            theme::text(),
        )];
        let (a, r) = staged.iter().fold((0, 0), |(a, r), i| match i.size {
            Some(Size::Lines { added, removed }) => (a + added, r + removed),
            _ => (a, r),
        });
        spans.extend(changes::counts(Some(Size::Lines { added: a, removed: r })));
        Line::from(spans)
    });
    if let Some(last) = &app.last_commit {
        next.push(Line::raw(""));
        next.push(Line::styled("last commit, C amends it", theme::faint()));
        next.push(Line::styled(last.clone(), theme::muted()));
    }

    let sections = vec![
        Section { title: "hunks", count: Some(outline.len()), lines: hunks },
        Section { title: "next commit", count: None, lines: next },
    ];
    app.outline_area = sidebar::draw(f, area, sections)[0];
}

/// The list's width with its padding: its widest file row or group header.
fn list_width(app: &App) -> u16 {
    let file = |e: &Entry| {
        let note = if e.group == Group::Conflicts { super::conflict::what(e.conflict).len() + 2 } else { 0 };
        e.item().row().width() + note
    };
    let header = |g: Group| {
        let items: Vec<changes::Item> = app.entries.iter().filter(|e| e.group == g).map(|e| e.item()).collect();
        changes::header(format!("{} {}", g.title(), items.len()), &items).width()
    };
    // Entries come grouped, so each group's header is measured once.
    let mut groups: Vec<Group> = app.entries.iter().map(|e| e.group).collect();
    groups.dedup();
    let widest = app.entries.iter().map(file).chain(groups.into_iter().map(header)).max().unwrap_or(0);
    // The selection bar, then the panel's padding: 2 left, 1 right.
    (widest + 2 + 3).min(u16::MAX as usize) as u16
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
                // The bar dims while the diff has the keys.
                let bar = if app.focus == Focus::Files { theme::ACCENT } else { theme::FAINT };
                let mut spans =
                    vec![if selected { Span::styled("┃ ", Style::new().fg(bar)) } else { Span::raw("  ") }];
                spans.extend(e.item().row().spans);
                if e.group == Group::Conflicts {
                    spans.push(Span::styled(format!("  {}", super::conflict::what(e.conflict)), theme::faint()));
                }
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
    // Two columns for the bar that marks the hunk.
    app.diff_width = inner.width.saturating_sub(2);
    if app.entries.is_empty() {
        return;
    }
    let focused = app.focus == Focus::Hunks;
    let current = app.hunk;
    let len = app.view().lines.len();
    app.detail_scroll.fit(len, inner.height);
    let top = (app.detail_scroll.top() as usize).min(len);
    let picked = app.picked_rows();
    let view = app.view();
    // While the diff has the keys, a bar marks the hunk they work on; while picking
    // lines, it marks the picked ones and `›` the cursor.
    let bar = focused.then(|| view.ranges.get(current).cloned()).flatten();
    let accent = Style::new().fg(theme::ACCENT);
    let shown: Vec<Line> = view.lines[top..]
        .iter()
        .enumerate()
        .take(inner.height as usize)
        .map(|(n, l)| {
            let row = top + n;
            let in_hunk = bar.as_ref().is_some_and(|r| r.contains(&row));
            let mark = match &picked {
                Some((cursor, _)) if *cursor == row => Span::styled("› ", accent),
                Some((_, range)) if range.contains(&row) => Span::styled("┃ ", accent),
                Some(_) if in_hunk => Span::styled("│ ", theme::faint()),
                None if in_hunk => Span::styled("┃ ", accent),
                _ => Span::raw("  "),
            };
            let mut spans = vec![mark];
            spans.extend(l.spans.iter().cloned());
            kit::truncate(Line::from(spans).style(l.style), inner.width)
        })
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
    app.detail_scroll.draw_more(f, area);
}

fn draw_footer(f: &mut Frame, app: &mut App, area: Rect) {
    if app.focus == Focus::Hunks {
        let staged = app.current().is_some_and(|e| e.group == Group::Staged);
        if app.pick().is_some() {
            let left = Line::styled("picking lines", Style::new().fg(theme::ACCENT));
            let n = app.picked_changes();
            let s = if n == 1 { "" } else { "s" };
            let space = if staged { format!("unstage {n} line{s}") } else { format!("stage {n} line{s}") };
            let mut pairs = vec![("space", space.as_str())];
            if !staged {
                pairs.push(("d", "discard"));
            }
            pairs.extend([("j/k", "move"), ("J/K", "extend"), ("esc", "cancel")]);
            footer::draw(f, area, left, pairs);
            return;
        }
        let n = app.view().ranges.len();
        let left = Line::styled(format!("hunk {} of {n}", app.hunk + 1), theme::muted());
        let mut pairs = vec![("space", if staged { "unstage hunk" } else { "stage hunk" })];
        if !staged {
            pairs.push(("d", "discard hunk"));
        }
        pairs.extend([("v", "pick lines"), ("j/k", "hunks"), ("esc", "files"), ("q", "quit")]);
        footer::sidebar(&mut pairs, area.width);
        footer::draw(f, area, left, pairs);
        return;
    }
    if let Some(c) = app.current().filter(|e| e.group == Group::Conflicts).map(|e| e.conflict) {
        let mut pairs = match c {
            Some(c) if !c.ours && !c.theirs => vec![("space", "remove")],
            Some(c) if !c.in_content() => vec![("space", "keep"), ("d", "delete")],
            _ => vec![("space", "resolved")],
        };
        pairs.extend([("e", "edit"), ("j/k", "files"), ("ctrl+p", "commands"), ("q", "quit")]);
        footer::draw(f, area, Line::default(), pairs);
        return;
    }
    let toggle = if app.current().is_some_and(|e| e.group == Group::Staged) { "unstage" } else { "stage" };
    let mut pairs = if app.entries.is_empty() {
        vec![("ctrl+r", "reload"), ("q", "quit")]
    } else {
        vec![
            ("space", toggle),
            ("enter", "hunks"),
            ("c", "commit"),
            ("a", "stage all"),
            ("u", "unstage all"),
            ("d", "discard"),
            ("ctrl+p", "commands"),
            ("q", "quit"),
        ]
    };
    footer::sidebar(&mut pairs, area.width);
    footer::draw(f, area, Line::default(), pairs);
}

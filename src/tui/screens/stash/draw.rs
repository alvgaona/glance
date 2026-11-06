//! Drawing the stash screen: header, changes pane, stash list, details, footer.

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
    // Wide: changes on the left, stashes and the selected one's details on the right.
    // Narrow: all three stacked.
    // Not `clamp`: it panics when a short terminal makes the upper bound the smaller one.
    let list_rows = (app.visible.len() as u16 + 4).min(body.height / 3).max(5);
    let p = kit::panes(body, Some(app.changes.width()), app.changes.height(body.width), false);
    let (changes, right) = (p.list, p.main);
    let [list, _, details] =
        Layout::vertical([Constraint::Length(list_rows), Constraint::Length(1), Constraint::Min(3)]).areas(right);
    app.changes_area = changes;
    let focused = app.focus == Pane::Changes;
    app.changes.draw(f, changes, focused);
    draw_list(f, app, list);
    draw_details(f, app, details);
    draw_footer(f, app, footer);

    if let Some(p) = &app.palette {
        p.draw(f, full);
    }
    if let Some((c, _)) = &app.confirm {
        c.draw(f, full);
    }
    if let Some((input, _)) = &app.input {
        input.draw(f, full);
    }
    if let Some(t) = &app.toast {
        t.draw(f, full);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let n = app.list.len();
    let mut spans = vec![
        Span::styled("stashes", Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.repo_name.clone(), theme::bold()),
        Span::raw("  "),
        Span::styled(format!("{n} stash{}", if n == 1 { "" } else { "es" }), theme::muted()),
    ];
    if let Some(b) = &app.branch {
        spans.push(Span::styled(format!(" · on {b}"), theme::muted()));
    }
    // Applying onto a dirty tree can fail or conflict, so say what's there.
    spans.push(match app.dirty {
        0 => Span::styled(" · working tree clean", theme::faint()),
        1 => Span::styled(" · 1 changed file", Style::new().fg(theme::YELLOW)),
        d => Span::styled(format!(" · {d} changed files"), Style::new().fg(theme::YELLOW)),
    });
    f.render_widget(Paragraph::new(kit::truncate(Line::from(spans), area.width)), area);
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(0, 0, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let focused = app.focus == Pane::Stashes;

    // Title row, a gap, then the stashes.
    let title = if focused { Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD) } else { theme::bold() };
    let n = app.list.len();
    let count = if n == 0 { "none yet".to_string() } else { n.to_string() };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled("stashes", title),
            Span::styled(format!("   {count}"), theme::muted()),
        ])),
        inner,
    );
    let inner = Rect { y: inner.y + 2, height: inner.height.saturating_sub(2), ..inner };
    app.list_area = inner;

    let height = inner.height as usize;
    app.scroll = list::follow(app.scroll, app.selected, app.selected, height);
    if app.list.is_empty() {
        let hint = if app.changes.is_empty() { "  Nothing stashed" } else { "  Pick changes on the left, then enter" };
        f.render_widget(Paragraph::new(Span::styled(hint, theme::muted())), inner);
        return;
    }
    if app.visible.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("  No stashes match", theme::muted())), inner);
        return;
    }

    let now = now();
    let index_w = app.list.iter().map(|s| s.index.to_string().len()).max().unwrap_or(1);
    // Fixed columns on the right: branch, "+untracked", age.
    let branch_w =
        app.list.iter().map(|s| s.branch.as_deref().unwrap_or("detached").chars().count()).max().unwrap_or(0);
    let any_untracked = app.list.iter().any(|s| s.untracked.is_some());
    for (row, &i) in app.visible.iter().enumerate().skip(app.scroll).take(height) {
        let s = &app.list[i];
        let r = Rect { y: inner.y + (row - app.scroll) as u16, height: 1, ..inner };
        let selected = row == app.selected;
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };

        let bar =
            if selected && focused { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };
        let mark = if app.marked.contains(&s.oid) {
            Span::styled("✓ ", Style::new().fg(theme::ACCENT))
        } else {
            Span::raw("  ")
        };
        let mut left = vec![
            bar,
            mark,
            Span::styled(format!("{:>index_w$}  ", s.index), theme::faint()),
            // Without -m the "title" is just the last commit's subject: show it quieter.
            Span::styled(s.title().to_string(), if s.is_wip() { theme::muted() } else { theme::text() }),
        ];
        if s.is_wip() {
            left.push(Span::styled("  wip", theme::faint()));
        }

        let branch = s.branch.clone().unwrap_or_else(|| "detached".into());
        let here = s.branch.is_some() && s.branch == app.branch;
        let mut right = vec![Span::styled(
            format!("{branch:<branch_w$}"),
            if here { Style::new().fg(theme::ACCENT) } else { theme::muted() },
        )];
        if any_untracked {
            let tag = if s.untracked.is_some() { "+untracked" } else { "" };
            right.push(Span::styled(format!("  {tag:<10}"), theme::faint()));
        }
        right.push(Span::styled(format!("{:>9}", relative_time(s.time, now)), theme::faint()));
        right.push(Span::raw("  "));
        let right = Line::from(right);

        let [l, rr] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16 + 2)]).areas(r);
        f.render_widget(Paragraph::new(kit::truncate(Line::from(left), l.width)).style(Style::new().bg(bg)), l);
        f.render_widget(
            Paragraph::new(right).alignment(ratatui::layout::Alignment::Right).style(Style::new().bg(bg)),
            rr,
        );
    }
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.detail_area = area;
    app.detail_width = inner.width;
    let Some(lines) = app.detail(inner.width) else {
        // Not read yet (it is before the next draw), or no stash at all.
        if app.current().is_none() {
            f.render_widget(Paragraph::new(Span::styled("A stash's files and diff show here.", theme::faint())), inner);
        }
        return;
    };

    // Another stash starts at the top.
    let id = app.current().map(|s| s.oid);
    if id != app.detail_for {
        app.detail_for = id;
        app.detail_scroll.reset();
    }
    app.detail_scroll.fit(lines.len(), inner.height);

    let shown: Vec<Line> = lines
        .iter()
        .skip(app.detail_scroll.top() as usize)
        .take(inner.height as usize)
        .map(|l| kit::truncate(l.clone(), inner.width))
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
    app.detail_scroll.draw_more(f, area);
}

pub(super) fn build_detail(
    repo: &gix::Repository,
    s: &Stash,
    branch: Option<&str>,
    width: usize,
) -> Vec<Line<'static>> {
    let now = now();
    let label = |t: &str| Span::styled(format!("{t:<10}"), theme::faint());
    let mut out: Vec<Line<'static>> =
        kit::wrap(s.title(), width).into_iter().map(|l| Line::styled(l, theme::bold())).collect();
    out.push(Line::from(vec![
        Span::styled(s.name(), Style::new().fg(theme::ACCENT)),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(s.id.clone(), theme::muted()),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(relative_time(s.time, now), theme::muted()),
    ]));
    out.push(Line::raw(""));

    let on = s.branch.clone().unwrap_or_else(|| "detached HEAD".into());
    let here = s.branch.is_some() && s.branch.as_deref() == branch;
    let mut branch_row =
        vec![label("branch"), Span::styled(on, Style::new().fg(if here { theme::ACCENT } else { theme::TEXT }))];
    if here {
        branch_row.push(Span::styled("  (current)", theme::faint()));
    }
    out.push(Line::from(branch_row));
    out.push(Line::from(vec![
        label("made on"),
        Span::styled(format!("{}  ", s.base_id), theme::muted()),
        Span::styled(s.base_subject.clone(), theme::text()),
    ]));
    // Has HEAD moved since? Applying then may conflict.
    if let Ok(head) = repo.head_id() {
        let head = head.detach();
        if head != s.base {
            let (ahead, behind) = crate::git::branches::ahead_behind(repo, head, s.base);
            let note = match (ahead, behind) {
                (a, 0) => format!("HEAD is {a} commit{} past it", if a == 1 { "" } else { "s" }),
                _ => "HEAD is on a different line of history".to_string(),
            };
            out.push(Line::from(vec![label(""), Span::styled(note, Style::new().fg(theme::YELLOW))]));
        }
    }

    match stash::files(repo, s) {
        Ok((tracked, untracked)) => {
            out.push(Line::raw(""));
            out.extend(panels::changes::files(&tracked, None));
            if !untracked.is_empty() {
                out.push(Line::raw(""));
                out.extend(panels::changes::files(&untracked, Some("untracked")));
            }
            out.extend(diff::patches(repo, &tracked, width as u16));
            out.extend(diff::patches(repo, &untracked, width as u16));
        }
        Err(e) => out.push(Line::styled(format!("couldn't read the stash: {e}"), Style::new().fg(theme::RED))),
    }
    out
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let left = if app.searching || !app.query.is_empty() {
        footer::search(&app.query, app.searching, app.visible.len(), app.list.len())
    } else if !app.marked.is_empty() {
        Line::styled(format!("✓ {} selected", app.marked.len()), Style::new().fg(theme::ACCENT))
    } else {
        Line::raw("")
    };
    let pairs: Vec<(&str, &str)> = if app.searching {
        vec![("enter", "keep filter"), ("esc", "clear")]
    } else if app.focus == Pane::Changes {
        let mut p = if app.changes.is_empty() { Vec::new() } else { app.changes.hints() };
        p.extend([("ctrl+p", "commands"), ("tab", "stashes"), ("q", "quit")]);
        p
    } else if !app.marked.is_empty() {
        vec![("d", "drop"), ("space", "select"), ("esc", "clear")]
    } else if app.list.is_empty() {
        vec![("tab", "changes"), ("q", "quit")]
    } else {
        vec![
            ("enter", "apply"),
            ("p", "pop"),
            ("d", "drop"),
            ("ctrl+p", "commands"),
            ("b", "branch"),
            ("tab", "changes"),
            ("q", "quit"),
        ]
    };
    footer::draw(f, area, left, pairs);
}

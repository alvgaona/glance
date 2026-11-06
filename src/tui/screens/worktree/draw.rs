//! Drawing the worktree screen: list, details, footer.

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
    let p = kit::panes(body, Some(list_width(app)), app.rows.len() as u16 + 2, false);
    let (list, details) = (p.list, p.main);
    draw_list(f, app, list);
    draw_details(f, app, details);
    draw_footer(f, app, footer);

    if let Some(p) = &app.palette {
        p.draw(f, full);
    }
    if let Some((c, _)) = &app.confirm {
        c.draw(f, full);
    }
    if let Some(input) = &app.input {
        input.draw(f, full);
    }
    if let Some(t) = &app.toast {
        t.draw(f, full);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let n = app.rows.len();
    let mut spans = vec![
        Span::styled("worktrees", Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.repo_name.clone(), theme::bold()),
        Span::raw("  "),
        Span::styled(format!("{n} worktree{}", if n == 1 { "" } else { "s" }), theme::muted()),
    ];
    let gone = app.rows.iter().filter(|r| r.wt.prunable.is_some()).count();
    if gone > 0 {
        spans.push(Span::styled(format!(" · {gone} prunable"), Style::new().fg(theme::YELLOW)));
    }
    f.render_widget(Paragraph::new(kit::truncate(Line::from(spans), area.width)), area);
}

/// `3 changed` / `clean` / `…`, and its colour.
fn state_text(r: &Row) -> Span<'static> {
    if r.wt.bare {
        return Span::styled("bare", theme::faint());
    }
    match &r.load {
        Load::Loading => Span::styled("…", theme::faint()),
        Load::None => Span::styled("folder gone", Style::new().fg(theme::RED)),
        Load::Failed(_) => Span::styled("unreadable", Style::new().fg(theme::RED)),
        Load::Ready(st) => {
            if !st.conflicted.is_empty() {
                return Span::styled(format!("{} conflicts", st.conflicted.len()), Style::new().fg(theme::RED));
            }
            if let Some(op) = &st.in_progress {
                return Span::styled(op.clone(), Style::new().fg(theme::YELLOW));
            }
            match r.dirty().unwrap_or(0) {
                0 => Span::styled("clean", theme::faint()),
                n => Span::styled(format!("{n} changed"), Style::new().fg(theme::YELLOW)),
            }
        }
    }
}

/// `↑2 ↓1` against the upstream, empty when in sync or unknown.
fn sync_text(r: &Row) -> String {
    let Load::Ready(st) = &r.load else { return String::new() };
    let Some(u) = &st.upstream else { return String::new() };
    crate::fmt::sync::arrows(u.ahead, u.behind)
}

/// The list's width that fits its columns as `draw_list` lays them out: bar, dot,
/// path, branch and lock on the left; state, sync and age on the right.
fn list_width(app: &App) -> u16 {
    let w = |f: &dyn Fn(&Row) -> usize| app.rows.iter().map(f).max().unwrap_or(0);
    let path = w(&|r| crate::fmt::width(&app.name(&r.wt.path)));
    let branch = w(&|r| crate::fmt::width(&r.label()));
    let locked = if app.rows.iter().any(|r| r.wt.locked.is_some()) { 8 } else { 0 };
    let state = w(&|r| crate::fmt::width(&state_text(r).content));
    let sync = match w(&|r| crate::fmt::width(&sync_text(r))) {
        0 => 0,
        s => 2 + s,
    };
    (4 + path + 2 + branch + locked + state + sync + 11 + 2).min(u16::MAX as usize) as u16
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(0, 0, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;

    let height = inner.height as usize;
    app.scroll = list::follow(app.scroll, app.selected, app.selected, height);

    let now = now();
    let paths: Vec<String> = app.rows.iter().map(|r| app.name(&r.wt.path)).collect();
    let path_w = paths.iter().map(|p| p.chars().count()).max().unwrap_or(0).min(inner.width as usize / 2);
    let branch_w = app.rows.iter().map(|r| r.label().chars().count()).max().unwrap_or(0);
    let state_w = app.rows.iter().map(|r| state_text(r).content.chars().count()).max().unwrap_or(0);
    let sync_w = app.rows.iter().map(|r| sync_text(r).chars().count()).max().unwrap_or(0);
    for (row, r) in app.rows.iter().enumerate().skip(app.scroll).take(height) {
        let area = Rect { y: inner.y + (row - app.scroll) as u16, height: 1, ..inner };
        let selected = row == app.selected;
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };

        let path = &paths[row];
        let pad = path_w.saturating_sub(path.chars().count());
        let mut left = vec![
            if selected { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") },
            // ● marks where you are now.
            if r.here { Span::styled("● ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") },
            Span::styled(format!("{path}{}  ", " ".repeat(pad)), if r.here { theme::bold() } else { theme::text() }),
            Span::styled(
                format!("{:<branch_w$}", r.label()),
                Style::new().fg(if r.wt.branch.is_some() { theme::ACCENT } else { theme::MUTED }),
            ),
        ];
        if r.wt.locked.is_some() {
            left.push(Span::styled("  locked", Style::new().fg(theme::YELLOW)));
        }

        let state = state_text(r);
        let state = Span::styled(format!("{:<state_w$}", state.content), state.style);
        let mut right = vec![state];
        if sync_w > 0 {
            right.push(Span::styled(format!("  {:<sync_w$}", sync_text(r)), Style::new().fg(theme::BLUE)));
        }
        let age = if r.time > 0 { relative_time(r.time, now) } else { String::new() };
        right.push(Span::styled(format!("{age:>9}  "), theme::faint()));
        let right = Line::from(right);

        let [l, rr] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16 + 2)]).areas(area);
        f.render_widget(Paragraph::new(kit::truncate(Line::from(left), l.width)).style(Style::new().bg(bg)), l);
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right).style(Style::new().bg(bg)), rr);
    }
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.detail_area = area;
    let Some(r) = app.current() else { return };
    let lines = build_detail(r, inner.width as usize);

    app.detail_scroll.fit(lines.len(), inner.height);
    let shown: Vec<Line> = lines
        .into_iter()
        .skip(app.detail_scroll.top() as usize)
        .take(inner.height as usize)
        .map(|l| kit::truncate(l, inner.width))
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
    app.detail_scroll.draw_more(f, area);
}

fn build_detail(r: &Row, width: usize) -> Vec<Line<'static>> {
    let now = now();
    let label = |t: &str| Span::styled(format!("{t:<10}"), theme::faint());
    let title = match &r.wt.branch {
        Some(b) => b.clone(),
        None if r.wt.bare => "bare repository".into(),
        None => "detached HEAD".into(),
    };
    let mut out: Vec<Line<'static>> = vec![Line::styled(title, theme::bold())];
    let mut where_ = vec![Span::styled(tilde(&r.wt.path), theme::muted())];
    if r.here {
        where_.push(Span::styled("  · you're here", Style::new().fg(theme::ACCENT)));
    }
    if r.wt.main {
        where_.push(Span::styled("  · main worktree", theme::faint()));
    }
    out.push(Line::from(where_));
    out.push(Line::raw(""));

    if !r.short.is_empty() {
        let mut commit = vec![label("commit"), Span::styled(format!("{}  ", r.short), theme::muted())];
        commit.extend(
            kit::wrap(&r.subject, width.saturating_sub(20)).into_iter().take(1).map(|s| Span::styled(s, theme::text())),
        );
        out.push(Line::from(commit));
        out.push(Line::from(vec![label(""), Span::styled(relative_time(r.time, now), theme::faint())]));
    }
    if let Load::Ready(st) = &r.load {
        match &st.upstream {
            Some(u) => {
                let sync = sync_text(r);
                let sync = if sync.is_empty() { "in sync".to_string() } else { sync };
                out.push(Line::from(vec![
                    label("upstream"),
                    Span::styled(format!("{}  ", u.name), theme::muted()),
                    Span::styled(sync, Style::new().fg(theme::BLUE)),
                ]));
            }
            None if r.wt.branch.is_some() => {
                out.push(Line::from(vec![label("upstream"), Span::styled("none: not pushed", theme::faint())]))
            }
            None => {}
        }
        if let Some(op) = &st.in_progress {
            out.push(Line::from(vec![label("state"), Span::styled(op.clone(), Style::new().fg(theme::YELLOW))]));
        }
    }
    if let Some(why) = &r.wt.locked {
        let why = if why.is_empty() { "yes".to_string() } else { why.clone() };
        out.push(Line::from(vec![label("locked"), Span::styled(why, Style::new().fg(theme::YELLOW))]));
    }
    if let Some(why) = &r.wt.prunable {
        out.push(Line::from(vec![label("prunable"), Span::styled(why.clone(), Style::new().fg(theme::RED))]));
    }
    out.push(Line::raw(""));

    match &r.load {
        Load::Loading => out.push(Line::styled("Checking for changes…", theme::faint())),
        Load::Failed(e) => out.push(Line::styled(format!("Couldn't read it: {e}"), Style::new().fg(theme::RED))),
        Load::None if r.wt.bare => {}
        Load::None => out.push(Line::styled(
            "The folder is gone, so git can't use this worktree. p prunes it.",
            Style::new().fg(theme::YELLOW),
        )),
        Load::Ready(st) => {
            if r.dirty() == Some(0) {
                out.push(Line::styled("Clean: nothing uncommitted.", theme::muted()));
            }
            out.extend(changes::status(st));
        }
    }
    out
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let go = if app.captured { "go there" } else { "copy path" };
    let pairs = vec![
        ("enter", go),
        ("n", "new"),
        ("d", "remove"),
        ("ctrl+p", "commands"),
        ("p", "prune"),
        ("l", "lock"),
        ("q", "quit"),
    ];
    footer::draw(f, area, Line::default(), pairs);
}

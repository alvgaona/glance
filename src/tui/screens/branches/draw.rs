//! Drawing the branch screen: list, details with PR status, footer.

use super::*;

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.detail_area = area;
    app.detail_width = inner.width;
    // Not read yet: it is before the next draw.
    let Some(lines) = app.detail(inner.width) else { return };

    // A new branch starts at the top.
    let name = app.current().map(|b| b.name.clone()).unwrap_or_default();
    if name != app.detail_for {
        app.detail_for = name;
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
    let (list, details) = if !app.details {
        (body, None)
    } else if body.width >= 110 {
        let [l, _, r] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Length(1), Constraint::Min(30)]).areas(body);
        (l, Some(r))
    } else {
        let [t, _, b] =
            Layout::vertical([Constraint::Percentage(50), Constraint::Length(1), Constraint::Min(5)]).areas(body);
        (t, Some(b))
    };
    draw_list(f, app, list);
    if let Some(d) = details {
        draw_details(f, app, d);
    } else {
        app.detail_area = Rect::default();
    }
    draw_footer(f, app, footer);

    if let Some(p) = &app.palette {
        p.draw(f, full);
    }
    if let Some((c, _)) = &app.confirm {
        c.draw(f, full);
    }
    if let Some(p) = &app.pick {
        p.draw(f, full);
    }
    if let Some((input, _)) = &app.rename {
        input.draw(f, full);
    }
    if let Some(t) = &app.toast {
        t.draw(f, full);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let local: Vec<&Branch> = app.data.list.iter().filter(|b| !b.remote).collect();
    let merged = local.iter().filter(|b| b.merged).count();
    let gone = local.iter().filter(|b| b.gone()).count();
    let mut spans = vec![
        Span::styled("branches", Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.repo_name.clone(), theme::bold()),
        Span::raw("  "),
        Span::styled(format!("{} local", local.len()), theme::muted()),
    ];
    if merged > 0 {
        spans.push(Span::styled(format!(" · {merged} merged"), theme::muted()));
    }
    if gone > 0 {
        spans.push(Span::styled(format!(" · {gone} gone"), Style::new().fg(theme::RED)));
    }
    if app.remotes {
        spans.push(Span::styled("  · with remotes", theme::faint()));
    }
    if app.fetching.is_some() {
        spans.push(Span::styled("  · fetching…", Style::new().fg(theme::ACCENT)));
    }
    match &app.prs {
        Prs::Loading(_) => spans.push(Span::styled("  · loading pull requests…", theme::faint())),
        Prs::Failed => spans.push(Span::styled("  · no pull requests (R retries)", theme::faint())),
        Prs::Ready(_) | Prs::Off => {}
    }
    f.render_widget(Paragraph::new(kit::truncate(Line::from(spans), area.width)), area);
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(0, 0, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;

    let height = inner.height as usize;
    app.scroll = list::follow(app.scroll, app.selected, app.selected, height);
    if app.visible.is_empty() {
        let msg = if app.query.is_empty() { "No branches" } else { "No branches match" };
        f.render_widget(Paragraph::new(Span::styled(format!("  {msg}"), theme::muted())), inner);
        return;
    }

    let now = now();
    for (row, &i) in app.visible.iter().enumerate().skip(app.scroll).take(height) {
        let b = &app.data.list[i];
        let r = Rect { y: inner.y + (row - app.scroll) as u16, height: 1, ..inner };
        let selected = row == app.selected;
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };

        let bar = if selected { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };
        let mark = if app.marked.contains(&b.name) {
            Span::styled("✓ ", Style::new().fg(theme::ACCENT))
        } else {
            Span::raw("  ")
        };
        // ● checked out here, ◆ checked out in another worktree.
        let dot = if b.current {
            Span::styled("● ", Style::new().fg(theme::ACCENT))
        } else if b.worktree.is_some() {
            Span::styled("◆ ", Style::new().fg(theme::BLUE))
        } else {
            Span::raw("  ")
        };
        let name_style = if b.current {
            Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
        } else if b.remote || b.merged || b.gone() {
            theme::muted()
        } else {
            theme::text()
        };
        let left = Line::from(vec![bar, mark, dot, Span::styled(b.name.clone(), name_style)]);

        let mut track: Vec<Span> = Vec::new();
        match &b.upstream {
            _ if b.gone() => track.push(Span::styled("gone", Style::new().fg(theme::RED))),
            Some(up) if up.ahead == 0 && up.behind == 0 => track.push(Span::styled("=", theme::faint())),
            Some(up) => {
                if up.ahead > 0 {
                    track.push(Span::styled(format!("↑{}", up.ahead), Style::new().fg(theme::GREEN)));
                }
                if up.behind > 0 {
                    if !track.is_empty() {
                        track.push(Span::raw(" "));
                    }
                    track.push(Span::styled(format!("↓{}", up.behind), Style::new().fg(theme::RED)));
                }
            }
            None if b.remote => track.push(Span::styled("remote", theme::faint())),
            None => {}
        }
        let track_w = Line::from(track.clone()).width();
        // On several remotes, or on one it doesn't track: "2r".
        let on = b.copies.len();
        let show = on > 1 || (on == 1 && (b.upstream.is_none() || b.gone()));
        let mut right = Vec::new();
        // PR column, only once some branch has one: "#1042 ✓".
        if !app.pr_of.is_empty() {
            let mut pr_spans = match app.pr(b) {
                Some(pr) => {
                    let mut s = vec![Span::styled(format!("#{}", pr.number), Style::new().fg(pr_color(pr.state)))];
                    if let Some((glyph, color)) = pr.checks.as_ref().and_then(checks_glyph) {
                        s.push(Span::styled(format!(" {glyph}"), Style::new().fg(color)));
                    }
                    s
                }
                None => Vec::new(),
            };
            let w = Line::from(pr_spans.clone()).width();
            right.push(Span::raw(" ".repeat(10usize.saturating_sub(w))));
            right.append(&mut pr_spans);
        }
        right.push(Span::styled(if show { format!("  {on}r") } else { "    ".into() }, theme::faint()));
        right.push(Span::raw(" ".repeat(9usize.saturating_sub(track_w))));
        right.extend(track);
        right.push(Span::styled(if b.merged { "   merged" } else { "         " }, theme::faint()));
        right.push(Span::styled(format!("{:>9}", relative_time(b.time, now)), theme::faint()));
        right.push(Span::raw("  "));
        let right = Line::from(right);

        let [l, rr] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(r);
        f.render_widget(Paragraph::new(kit::truncate(left, l.width)).style(Style::new().bg(bg)), l);
        f.render_widget(Paragraph::new(right).style(Style::new().bg(bg)), rr);
    }
}

fn pr_color(s: github::State) -> ratatui::style::Color {
    match s {
        github::State::Open => theme::GREEN,
        github::State::Draft => theme::MUTED,
        github::State::Merged => theme::PURPLE,
        github::State::Closed => theme::RED,
    }
}

/// ✗ if any check failed, • while some run, ✓ when all passed.
fn checks_glyph(c: &github::Checks) -> Option<(&'static str, ratatui::style::Color)> {
    if c.failed > 0 {
        Some(("✗", theme::RED))
    } else if c.pending > 0 {
        Some(("•", theme::YELLOW))
    } else if c.passed > 0 {
        Some(("✓", theme::GREEN))
    } else {
        None
    }
}

/// The "pr" rows of the details panel.
fn pr_lines(pr: &Pr, b: &Branch, label: &dyn Fn(&str) -> Span<'static>, width: usize) -> Vec<Line<'static>> {
    let state = match pr.state {
        github::State::Open => "open",
        github::State::Draft => "draft",
        github::State::Merged => "merged",
        github::State::Closed => "closed",
    };
    let mut first = vec![
        label("pr"),
        Span::styled(format!("#{}", pr.number), Style::new().fg(pr_color(pr.state)).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(format!(" {state} "), Style::new().bg(theme::ELEMENT).fg(pr_color(pr.state))),
    ];
    if matches!(pr.state, github::State::Open | github::State::Draft) {
        let review = match pr.review.as_deref() {
            Some("APPROVED") => Some(("approved", theme::GREEN)),
            Some("CHANGES_REQUESTED") => Some(("changes requested", theme::RED)),
            Some("REVIEW_REQUIRED") => Some(("review required", theme::MUTED)),
            _ => None,
        };
        if let Some((text, color)) = review {
            first.push(Span::styled("  ·  ", theme::faint()));
            first.push(Span::styled(text, Style::new().fg(color)));
        }
        if let Some(c) = &pr.checks {
            let mut parts = Vec::new();
            if c.failed > 0 {
                parts.push(Span::styled(format!("✗ {} failed", c.failed), Style::new().fg(theme::RED)));
            }
            if c.pending > 0 {
                parts.push(Span::styled(format!("• {} running", c.pending), Style::new().fg(theme::YELLOW)));
            }
            if c.failed == 0 && c.pending == 0 && c.passed > 0 {
                parts.push(Span::styled(format!("✓ {} checks passed", c.passed), Style::new().fg(theme::GREEN)));
            }
            for p in parts {
                first.push(Span::styled("  ·  ", theme::faint()));
                first.push(p);
            }
        }
    }
    let mut out = vec![Line::from(first)];
    // Title wrapped under the label column (10 wide).
    for part in kit::wrap(&pr.title, width.saturating_sub(10)) {
        out.push(Line::from(vec![label(""), Span::styled(part, theme::text())]));
    }
    // Local work the PR doesn't have (pushed later, or never pushed).
    if pr.head.is_some_and(|h| h != b.oid) {
        let note = match pr.state {
            github::State::Merged => "branch has changed since the PR was merged",
            github::State::Closed => "branch has changed since the PR was closed",
            _ => "local branch differs from the PR",
        };
        out.push(Line::from(vec![label(""), Span::styled(note, Style::new().fg(theme::YELLOW))]));
    }
    out.push(Line::from(vec![
        label(""),
        Span::styled(format!("{}  ", pr.url), theme::faint()),
        Span::styled("o open", theme::muted()),
    ]));
    out
}

pub(super) fn build_detail(
    repo: &gix::Repository,
    b: &Branch,
    default: Option<&(String, gix::ObjectId)>,
    protected: bool,
    pr: Option<&Pr>,
    width: usize,
) -> Vec<Line<'static>> {
    let now = now();
    let label = |s: &str| Span::styled(format!("{s:<10}"), theme::faint());
    let mut badges = vec![Span::styled(
        b.name.clone(),
        if b.current { Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD) } else { theme::bold() },
    )];
    for (on, text, color) in [
        (b.current, "checked out", theme::ACCENT),
        (b.remote, "remote", theme::MUTED),
        (b.merged, "merged", theme::GREEN),
        (b.gone(), "gone", theme::RED),
        (b.worktree.is_some(), "in another worktree", theme::BLUE),
    ] {
        if on {
            badges.push(Span::raw("  "));
            badges.push(Span::styled(format!(" {text} "), Style::new().bg(theme::ELEMENT).fg(color)));
        }
    }
    let mut out = vec![
        Line::from(badges),
        Line::raw(""),
        Line::from(vec![
            Span::styled(b.id.clone(), Style::new().fg(theme::ACCENT)),
            Span::styled("  ·  ", theme::faint()),
            Span::styled(b.author.clone(), theme::muted()),
            Span::styled("  ·  ", theme::faint()),
            Span::styled(relative_time(b.time, now), theme::muted()),
        ]),
    ];
    for part in kit::wrap(&b.summary, width) {
        out.push(Line::styled(part, theme::text()));
    }
    out.push(Line::raw(""));

    if let Some(pr) = pr {
        out.extend(pr_lines(pr, b, &label, width));
        out.push(Line::raw(""));
    }

    // Upstream.
    if !b.remote {
        let up: Vec<Span> = match &b.upstream {
            None if b.copies.is_empty() => vec![Span::styled("none (never pushed)", theme::muted())],
            None => vec![Span::styled("none (not tracking a remote branch)", theme::muted())],
            Some(u) if b.gone() => vec![
                Span::styled(u.name.trim_end_matches(" (gone)").to_string(), theme::text()),
                Span::styled("  deleted on the remote", Style::new().fg(theme::RED)),
            ],
            Some(u) => {
                let mut s = vec![Span::styled(u.name.clone(), theme::text()), Span::raw("  ")];
                match (u.ahead, u.behind) {
                    (0, 0) => s.push(Span::styled("up to date", theme::muted())),
                    (a, d) => {
                        if a > 0 {
                            s.push(Span::styled(format!("{a} to push "), Style::new().fg(theme::GREEN)));
                        }
                        if d > 0 {
                            s.push(Span::styled(format!("{d} to pull"), Style::new().fg(theme::RED)));
                        }
                    }
                }
                s
            }
        };
        let mut line = vec![label("upstream")];
        line.extend(up);
        out.push(Line::from(line));

        if let Some(path) = &b.worktree {
            out.push(Line::from(vec![
                label("worktree"),
                Span::styled(short_path(path), Style::new().fg(theme::BLUE)),
                Span::styled("  can't delete or check out here", theme::muted()),
            ]));
        }

        // Every remote that has it, when that says more than the upstream row.
        if b.copies.len() > 1 || (b.copies.len() == 1 && (b.upstream.is_none() || b.gone())) {
            let w = b.copies.iter().map(|c| c.remote.chars().count() + c.branch.chars().count() + 1).max().unwrap_or(0);
            for (i, c) in b.copies.iter().enumerate() {
                let full = format!("{}/{}", c.remote, c.branch);
                let pad = " ".repeat(w - full.chars().count() + 2);
                let (ahead, behind) = branches::ahead_behind(repo, b.oid, c.oid);
                let color = match (ahead, behind) {
                    (0, 0) => theme::MUTED,
                    (_, 0) => theme::GREEN,
                    (0, _) => theme::RED,
                    _ => theme::YELLOW,
                };
                out.push(Line::from(vec![
                    if i == 0 { label("remotes") } else { label("") },
                    Span::styled(full + &pad, theme::text()),
                    Span::styled(sync_note((ahead, behind)), Style::new().fg(color)),
                ]));
            }
        }
    }

    // Relation to main.
    let Some((main, main_id)) = default else {
        out.push(Line::from(vec![label("main"), Span::styled("no main branch found", theme::muted())]));
        return out;
    };
    if !b.remote && b.name == *main {
        out.push(Line::from(vec![label("main"), Span::styled("this is the main branch", theme::muted())]));
        return out;
    }
    let d = branches::detail(repo, b, Some(*main_id)).ok();
    let state = match &d {
        Some(d) if d.unique_total == 0 => Span::styled(
            if protected { "merged".to_string() } else { format!("merged into {main}, safe to delete") },
            Style::new().fg(theme::GREEN),
        ),
        // Squash or rebase merges: git can't see it, GitHub can.
        Some(_) if pr.is_some_and(|p| p.state == github::State::Merged && p.head == Some(b.oid)) => Span::styled(
            if protected { "merged via PR".to_string() } else { "merged via PR, safe to delete".to_string() },
            Style::new().fg(theme::GREEN),
        ),
        Some(d) => Span::styled(
            format!("{} commit{} not in {main}", d.unique_total, if d.unique_total == 1 { "" } else { "s" }),
            Style::new().fg(theme::YELLOW),
        ),
        None => Span::styled("couldn't compare", theme::muted()),
    };
    out.push(Line::from(vec![label(main), state]));
    if let Some(d) = &d {
        if let Some((id, time)) = &d.fork {
            out.push(Line::from(vec![
                label("forked"),
                Span::styled(format!("from {main} {}", relative_time(*time, now)), theme::text()),
                Span::styled(format!("  {id}"), theme::faint()),
            ]));
        }
        if d.unique_total > 0 {
            out.push(Line::from(vec![
                label("changes"),
                Span::styled(format!("+{} ", d.added), Style::new().fg(theme::GREEN)),
                Span::styled(format!("−{}", d.removed), Style::new().fg(theme::RED)),
                Span::styled(format!("  in {} file{}", d.files, if d.files == 1 { "" } else { "s" }), theme::muted()),
            ]));
            out.push(Line::raw(""));
            out.push(Line::styled(format!("Commits not in {main}"), theme::faint()));
            for (id, subject) in &d.unique {
                out.push(Line::from(vec![
                    Span::styled(format!("{id}  "), theme::faint()),
                    Span::styled(subject.clone(), theme::text()),
                ]));
            }
            if d.unique_total > d.unique.len() {
                out.push(Line::styled(format!("…and {} more", d.unique_total - d.unique.len()), theme::faint()));
            }
        }
    }
    out
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let left = if app.searching || !app.query.is_empty() {
        footer::search(&app.query, app.searching, app.visible.len(), app.data.list.len())
    } else if !app.marked.is_empty() {
        Line::styled(format!("✓ {} selected", app.marked.len()), Style::new().fg(theme::ACCENT))
    } else {
        Line::styled(
            format!("{} branch{}", app.visible.len(), if app.visible.len() == 1 { "" } else { "es" }),
            theme::muted(),
        )
    };
    let pairs = if app.searching {
        vec![("enter", "keep filter"), ("esc", "clear")]
    } else if !app.marked.is_empty() {
        vec![("d", "delete"), ("D", "delete on remote"), ("space", "select"), ("esc", "clear")]
    } else {
        vec![
            ("enter", "checkout"),
            ("ctrl+p", "commands"),
            ("d", "delete"),
            ("f", "fetch"),
            ("r", "rename"),
            ("q", "quit"),
        ]
    };
    footer::draw(f, area, left, pairs);
}

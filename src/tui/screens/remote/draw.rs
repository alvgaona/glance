//! Drawing the remote screen: list, details, footer.

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
    let (list, details) = if body.width >= 120 {
        let [l, _, r] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Length(1), Constraint::Min(40)]).areas(body);
        (l, r)
    } else {
        // Not `clamp`: it panics when a short terminal makes the upper bound the smaller one.
        let rows = (app.remotes.len() as u16 + 2).min(body.height / 2).max(4);
        let [t, _, b] =
            Layout::vertical([Constraint::Length(rows), Constraint::Length(1), Constraint::Min(5)]).areas(body);
        (t, b)
    };
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
    let mut spans = vec![
        Span::styled("remotes", Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.repo_name.clone(), theme::bold()),
        Span::raw("  "),
        Span::styled(plural(app.remotes.len(), "remote"), theme::muted()),
    ];
    if let Some(j) = &app.job {
        // Dots that move while git works.
        let dots = ".".repeat((now() as usize % 3) + 1);
        spans.push(Span::styled(format!(" · {}{dots}", j.what.to_lowercase()), Style::new().fg(theme::YELLOW)));
    }
    f.render_widget(Paragraph::new(kit::truncate(Line::from(spans), area.width)), area);
}

fn fetched_text(r: &Remote, now: i64) -> String {
    match r.fetched {
        Some(t) => relative_time(t, now),
        None => "never".into(),
    }
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(0, 0, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;

    if app.remotes.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("  No remotes yet: a adds one", theme::muted())), inner);
        return;
    }
    let height = inner.height as usize;
    app.scroll = list::follow(app.scroll, app.selected, app.selected, height);

    let now = now();
    let name_w = app.remotes.iter().map(|r| r.name.chars().count()).max().unwrap_or(0);
    let urls: Vec<String> = app.remotes.iter().map(|r| r.url.as_deref().map(url::short).unwrap_or_default()).collect();
    let count_w = app.remotes.iter().map(|r| plural(r.branches.len(), "branch").len()).max().unwrap_or(0);
    // Whatever the name and the right-hand columns leave, so long URLs end in "…".
    let room = (inner.width as usize).saturating_sub(4 + name_w + 2 + count_w + 11 + 2);
    let url_w = urls.iter().map(|u| u.chars().count()).max().unwrap_or(0).min(room);
    let urls: Vec<String> = urls
        .into_iter()
        .map(|u| {
            if u.chars().count() > url_w {
                u.chars().take(url_w.saturating_sub(1)).collect::<String>() + "…"
            } else {
                u
            }
        })
        .collect();
    for (row, r) in app.remotes.iter().enumerate().skip(app.scroll).take(height) {
        let area = Rect { y: inner.y + (row - app.scroll) as u16, height: 1, ..inner };
        let selected = row == app.selected;
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };
        let here = app.tracked_here(r);

        let mut left = vec![
            if selected { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") },
            // ● marks the remote the checked-out branch tracks.
            if here { Span::styled("● ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") },
            Span::styled(format!("{:<name_w$}  ", r.name), Style::new().fg(theme::ACCENT)),
            Span::styled(format!("{:<url_w$}", urls[row]), theme::text()),
        ];
        if r.push_url.is_some() {
            left.push(Span::styled("  push elsewhere", Style::new().fg(theme::YELLOW)));
        }

        let right = Line::from(vec![
            Span::styled(format!("{:<count_w$}", plural(r.branches.len(), "branch")), theme::muted()),
            Span::styled(format!("{:>9}  ", fetched_text(r, now)), theme::faint()),
        ]);
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
    let Some(r) = app.current() else {
        let hint = "A remote is another copy of this repository, usually on a server.\na adds one.";
        f.render_widget(Paragraph::new(Text::styled(hint, theme::faint())), inner);
        return;
    };
    let lines = build_detail(r, app.tracked_here(r), inner.width as usize);

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

fn build_detail(r: &Remote, here: bool, width: usize) -> Vec<Line<'static>> {
    let now = now();
    let label = |t: &str| Span::styled(format!("{t:<10}"), theme::faint());
    let mut out: Vec<Line<'static>> = vec![Line::styled(r.name.clone(), theme::bold())];
    let mut sub = vec![Span::styled(r.url.as_deref().map(url::short).unwrap_or_default(), theme::muted())];
    if here {
        sub.push(Span::styled("  · this branch tracks it", Style::new().fg(theme::ACCENT)));
    }
    out.push(Line::from(sub));
    out.push(Line::raw(""));

    let url = |u: &Option<String>| match u {
        Some(u) => Span::styled(u.clone(), theme::text()),
        None => Span::styled("none", Style::new().fg(theme::RED)),
    };
    out.push(Line::from(vec![label("fetch"), url(&r.url)]));
    out.push(Line::from(match &r.push_url {
        Some(p) => vec![label("push"), Span::styled(p.clone(), Style::new().fg(theme::YELLOW))],
        None => vec![label("push"), Span::styled("same as fetch", theme::faint())],
    }));
    out.push(Line::from(match &r.head {
        Some(h) => vec![label("default"), Span::styled(h.clone(), Style::new().fg(theme::ACCENT))],
        None => vec![label("default"), Span::styled("unknown: h asks the remote", theme::faint())],
    }));
    out.push(Line::from(match r.fetched {
        Some(t) => vec![label("fetched"), Span::styled(relative_time(t, now), theme::muted())],
        None => vec![label("fetched"), Span::styled("never: f fetches it", Style::new().fg(theme::YELLOW))],
    }));
    if !r.tracking.is_empty() {
        let names = r.tracking.join(", ");
        let mut rows = kit::wrap(&names, width.saturating_sub(10)).into_iter();
        out.push(Line::from(vec![label("tracking"), Span::styled(rows.next().unwrap_or_default(), theme::text())]));
        out.extend(rows.map(|l| Line::from(vec![label(""), Span::styled(l, theme::text())])));
    }
    out.push(Line::raw(""));

    if r.branches.is_empty() {
        out.push(Line::styled("No branches fetched from it yet.", theme::muted()));
        return out;
    }
    out.push(Line::styled(plural(r.branches.len(), "branch"), theme::faint()));
    let name_w = r.branches.iter().map(|b| b.name.chars().count()).max().unwrap_or(0).min(width / 3);
    for b in &r.branches {
        let pad = name_w.saturating_sub(b.name.chars().count());
        let color = if r.head.as_ref() == Some(&b.name) { theme::ACCENT } else { theme::TEXT };
        out.push(Line::from(vec![
            Span::styled(format!("{}{}  ", b.name, " ".repeat(pad)), Style::new().fg(color)),
            Span::styled(format!("{}  ", b.id), theme::faint()),
            Span::styled(format!("{:>8}  ", relative_time(b.time, now)), theme::faint()),
            Span::styled(b.summary.clone(), theme::muted()),
        ]));
    }
    out
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let pairs = if app.remotes.is_empty() {
        vec![("a", "add"), ("ctrl+p", "commands"), ("q", "quit")]
    } else {
        vec![
            ("f", "fetch"),
            ("a", "add"),
            ("r", "rename"),
            ("u", "url"),
            ("d", "remove"),
            ("ctrl+p", "commands"),
            ("q", "quit"),
        ]
    };
    footer::draw(f, area, Line::default(), pairs);
}

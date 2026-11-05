//! `gg log -i` (and the commit picker of `gg rebase -p`): commit list, details and diff, full screen.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};

use super::theme;
use super::widgets::{self, Command, Palette, PaletteEvent, Toast};
use crate::core::diff::{PatchLine, Size};
use crate::core::log::{Commit, RefLabel};
use crate::core::show::{self, Shown};
use crate::core::status::ChangeKind;
use crate::render::relative_time;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Look around; Enter returns the commit id (the caller copies it).
    Browse,
    /// Choose a commit for `gg rebase -p`.
    Pick,
}

pub struct Options {
    pub mode: Mode,
    pub all: bool,
    pub count: usize,
}

#[derive(Clone)]
enum Cmd {
    Search,
    Copy,
    ToggleAll,
    Details,
    Choose,
    Quit,
}

/// Rendered details of one commit, cached by id.
struct Detail {
    lines: Vec<Line<'static>>,
}

struct App<'r> {
    repo: &'r gix::Repository,
    opts: Options,
    commits: Vec<Commit>,
    /// Indexes into `commits` that match the search, in order.
    visible: Vec<usize>,
    selected: usize,
    scroll: usize,
    query: String,
    searching: bool,
    details: bool,
    detail_scroll: u16,
    cache: HashMap<gix::ObjectId, Rc<Detail>>,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    list_area: Rect,
    detail_area: Rect,
    repo_name: String,
    branch: String,
}

enum Outcome {
    Done(Option<String>),
}

/// Returns the chosen commit's short id, or `None` if the user quit.
pub fn run(repo: &gix::Repository, opts: Options) -> Result<Option<String>> {
    let mut app = App {
        repo,
        commits: Vec::new(),
        visible: Vec::new(),
        selected: 0,
        scroll: 0,
        query: String::new(),
        searching: false,
        details: true,
        detail_scroll: 0,
        cache: HashMap::new(),
        toast: None,
        palette: None,
        list_area: Rect::default(),
        detail_area: Rect::default(),
        repo_name: crate::core::repo_name(repo),
        branch: repo.head_name().ok().flatten().map(|n| n.shorten().to_string()).unwrap_or_else(|| "detached".into()),
        opts,
    };
    app.load()?;

    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    let result = (|| -> Result<Option<String>> {
        loop {
            if app.toast.as_ref().is_some_and(Toast::expired) {
                app.toast = None;
            }
            terminal.draw(|f| draw(f, &mut app))?;
            if !event::poll(Duration::from_millis(200))? {
                continue;
            }
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if let Some(Outcome::Done(id)) = app.key(k)? {
                        return Ok(id);
                    }
                }
                Event::Mouse(m) => app.mouse(m),
                _ => {}
            }
        }
    })();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();
    result
}

impl App<'_> {
    fn load(&mut self) -> Result<()> {
        self.commits = crate::core::log::read(self.repo, self.opts.count, self.opts.all)?;
        self.rebuild();
        Ok(())
    }

    /// Recomputes the visible rows after loading or a search change, keeping the selected commit.
    fn rebuild(&mut self) {
        let keep = self.selected_commit().map(|c| c.oid);
        let q = self.query.to_lowercase();
        self.visible = (0..self.commits.len()).filter(|&i| q.is_empty() || matches(&self.commits[i], &q)).collect();
        self.selected = keep
            .and_then(|oid| self.visible.iter().position(|&i| self.commits[i].oid == oid))
            .unwrap_or(0);
        self.detail_scroll = 0;
    }

    fn selected_commit(&self) -> Option<&Commit> {
        self.visible.get(self.selected).map(|&i| &self.commits[i])
    }

    /// Moves `steps` commits up (negative) or down.
    fn step(&mut self, steps: isize) {
        let last = self.visible.len().saturating_sub(1) as isize;
        let i = (self.selected as isize + steps).clamp(0, last) as usize;
        if i != self.selected {
            self.selected = i;
            self.detail_scroll = 0;
        }
    }

    fn commands(&self) -> Vec<Command<Cmd>> {
        let mut c = vec![Command { label: "Search commits", keys: "/", action: Cmd::Search }];
        match self.opts.mode {
            Mode::Browse => {
                c.push(Command { label: "Copy commit id", keys: "y", action: Cmd::Copy });
                c.push(Command { label: "Copy id and quit", keys: "enter", action: Cmd::Choose });
                let all = if self.opts.all { "Show only this branch" } else { "Show all branches" };
                c.push(Command { label: all, keys: "a", action: Cmd::ToggleAll });
            }
            Mode::Pick => c.push(Command { label: "Rebase from this commit", keys: "enter", action: Cmd::Choose }),
        }
        c.push(Command { label: "Show or hide details", keys: "tab", action: Cmd::Details });
        c.push(Command { label: "Quit", keys: "esc", action: Cmd::Quit });
        c
    }

    fn key(&mut self, k: KeyEvent) -> Result<Option<Outcome>> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if ctrl && k.code == KeyCode::Char('c') {
            return Ok(Some(Outcome::Done(None)));
        }
        if let Some(p) = &mut self.palette {
            match p.key(k) {
                Some(PaletteEvent::Close) => self.palette = None,
                Some(PaletteEvent::Run(cmd)) => {
                    self.palette = None;
                    return self.run(cmd);
                }
                None => {}
            }
            return Ok(None);
        }
        let half = (self.detail_area.height / 2).max(1);

        if self.searching {
            match k.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                    self.rebuild();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.rebuild();
                }
                KeyCode::Up => self.step(-1),
                KeyCode::Down => self.step(1),
                KeyCode::Char(c) if !ctrl => {
                    self.query.push(c);
                    self.rebuild();
                }
                _ => {}
            }
            return Ok(None);
        }

        let cmd = match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(self.commands()));
                return Ok(None);
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(self.commands()));
                return Ok(None);
            }
            KeyCode::Char('d') if ctrl => {
                self.detail_scroll = self.detail_scroll.saturating_add(half);
                return Ok(None);
            }
            KeyCode::Char('u') if ctrl => {
                self.detail_scroll = self.detail_scroll.saturating_sub(half);
                return Ok(None);
            }
            KeyCode::Down if shift => {
                self.detail_scroll = self.detail_scroll.saturating_add(3);
                return Ok(None);
            }
            KeyCode::Up if shift => {
                self.detail_scroll = self.detail_scroll.saturating_sub(3);
                return Ok(None);
            }
            KeyCode::Char('J') => {
                self.detail_scroll = self.detail_scroll.saturating_add(3);
                return Ok(None);
            }
            KeyCode::Char('K') => {
                self.detail_scroll = self.detail_scroll.saturating_sub(3);
                return Ok(None);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(1);
                return Ok(None);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(-1);
                return Ok(None);
            }
            KeyCode::PageDown => {
                self.step(15);
                return Ok(None);
            }
            KeyCode::PageUp => {
                self.step(-15);
                return Ok(None);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.step(-(self.visible.len() as isize));
                return Ok(None);
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.step(self.visible.len() as isize);
                return Ok(None);
            }
            KeyCode::Char('/') => Cmd::Search,
            KeyCode::Char('y') if self.opts.mode == Mode::Browse => Cmd::Copy,
            KeyCode::Char('a') if self.opts.mode == Mode::Browse => Cmd::ToggleAll,
            KeyCode::Tab => Cmd::Details,
            KeyCode::Enter => Cmd::Choose,
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.rebuild();
                return Ok(None);
            }
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Ok(None),
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Result<Option<Outcome>> {
        match cmd {
            Cmd::Search => self.searching = true,
            Cmd::Copy => {
                if let Some(c) = self.selected_commit() {
                    let id = c.id.clone();
                    crate::fzf::copy(&id);
                    self.toast = Some(Toast::new(format!("Copied {id}"), theme::ACCENT));
                }
            }
            Cmd::ToggleAll => {
                self.opts.all = !self.opts.all;
                self.load()?;
                let what = if self.opts.all { "All branches".to_string() } else { format!("Only {}", self.branch) };
                self.toast = Some(Toast::new(what, theme::ACCENT));
            }
            Cmd::Details => self.details = !self.details,
            Cmd::Choose => return Ok(Some(Outcome::Done(self.selected_commit().map(|c| c.id.clone())))),
            Cmd::Quit => return Ok(Some(Outcome::Done(None))),
        }
        Ok(None)
    }

    fn mouse(&mut self, m: crossterm::event::MouseEvent) {
        if self.palette.is_some() {
            return;
        }
        let inside = |a: Rect| m.column >= a.x && m.column < a.right() && m.row >= a.y && m.row < a.bottom();
        match m.kind {
            MouseEventKind::ScrollDown if inside(self.detail_area) => self.detail_scroll = self.detail_scroll.saturating_add(3),
            MouseEventKind::ScrollUp if inside(self.detail_area) => self.detail_scroll = self.detail_scroll.saturating_sub(3),
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::ScrollUp => self.step(-1),
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) => {
                let i = self.scroll + (m.row - self.list_area.y) as usize;
                if i < self.visible.len() && i != self.selected {
                    self.selected = i;
                    self.detail_scroll = 0;
                }
            }
            _ => {}
        }
    }

    fn detail(&mut self) -> Option<Rc<Detail>> {
        let c = self.selected_commit()?;
        if let Some(d) = self.cache.get(&c.oid) {
            return Some(d.clone());
        }
        let oid = c.oid;
        let lines = match show::read(self.repo, &oid.to_string()) {
            Ok(s) => detail_lines(self.repo, &s),
            Err(e) => vec![Line::styled(format!("Couldn't read this commit: {e}"), theme::muted())],
        };
        let d = Rc::new(Detail { lines });
        self.cache.insert(oid, d.clone());
        Some(d)
    }
}

fn matches(c: &Commit, q: &str) -> bool {
    let refs = c.refs.iter().any(|r| label_name(r).to_lowercase().contains(q));
    refs || c.summary.to_lowercase().contains(q) || c.author.to_lowercase().contains(q) || c.id.starts_with(q)
}

fn label_name(r: &RefLabel) -> &str {
    match r {
        RefLabel::Head(n) | RefLabel::Branch(n) | RefLabel::Remote(n) | RefLabel::Tag(n) => n,
    }
}

// ---------------------------------------------------------------- drawing

fn draw(f: &mut Frame, app: &mut App) {
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
        let [l, _, r] = Layout::horizontal([Constraint::Percentage(50), Constraint::Length(1), Constraint::Min(30)]).areas(body);
        (l, Some(r))
    } else {
        let [t, _, b] = Layout::vertical([Constraint::Percentage(45), Constraint::Length(1), Constraint::Min(5)]).areas(body);
        (t, Some(b))
    };
    draw_list(f, app, list);
    app.detail_area = details.unwrap_or_default();
    if let Some(d) = details {
        draw_details(f, app, d);
    }
    draw_footer(f, app, footer);

    if let Some(p) = &app.palette {
        p.draw(f, full);
    }
    if let Some(t) = &app.toast {
        t.draw(f, full);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let accent = Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD);
    let left = match app.opts.mode {
        Mode::Browse => Line::from(vec![
            Span::styled("log", accent),
            Span::raw("  "),
            Span::styled(app.repo_name.clone(), theme::bold()),
            Span::raw("  "),
            Span::styled(if app.opts.all { "all branches".to_string() } else { app.branch.clone() }, theme::muted()),
        ]),
        Mode::Pick => Line::from(vec![
            Span::styled("rebase", accent),
            Span::raw("  "),
            Span::styled(app.repo_name.clone(), theme::bold()),
            Span::raw("  "),
            Span::styled("choose the oldest commit to edit", theme::muted()),
        ]),
    };
    f.render_widget(Paragraph::new(left), area);
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(0, 0, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;

    let height = inner.height as usize;
    if app.selected < app.scroll {
        app.scroll = app.selected;
    } else if height > 0 && app.selected >= app.scroll + height {
        app.scroll = app.selected + 1 - height;
    }

    if app.visible.is_empty() {
        let msg = if app.query.is_empty() { "No commits yet" } else { "No commits match" };
        f.render_widget(Paragraph::new(Span::styled(format!("  {msg}"), theme::muted())), inner);
        return;
    }

    let now = crate::core::now();
    let rows: Vec<usize> = app.visible.iter().copied().skip(app.scroll).take(height).collect();
    // Branch column: only as wide as needed, and small next to the messages
    // (a sixth of the list, 10 to 30 columns).
    let refs_w = rows
        .iter()
        .map(|&i| Line::from(compact_refs(&app.commits[i].refs, usize::MAX)).width())
        .max()
        .unwrap_or(0)
        .min((inner.width / 6).clamp(10, 30) as usize);

    for (n, &index) in rows.iter().enumerate() {
        let row = app.scroll + n;
        let r = Rect { y: inner.y + n as u16, height: 1, ..inner };
        let selected = row == app.selected;
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };
        let bar = if selected { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };

        let c = &app.commits[index];
        let subject_style = if c.is_merge { theme::muted() } else { theme::text() };
        let left = Line::from(vec![
            bar,
            Span::styled(c.id.clone(), theme::faint()),
            Span::raw("  "),
            Span::styled(c.summary.clone(), subject_style),
        ]);

        // Right side: branches (right-aligned in their column), author, age.
        let refs = Line::from(compact_refs(&c.refs, refs_w));
        let mut right = vec![Span::raw(" ".repeat(refs_w.saturating_sub(refs.width()) + 2))];
        right.extend(refs.spans);
        let author = c.author.split_whitespace().next().unwrap_or(&c.author).to_string();
        right.push(Span::raw("  "));
        right.push(Span::styled(format!("{author:<10}"), theme::muted()));
        right.push(Span::styled(format!("{:>8}", relative_time(c.time, now)), theme::faint()));
        right.push(Span::raw("  "));
        let right = Line::from(right);

        let [l, rr] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(r);
        f.render_widget(Paragraph::new(widgets::truncate(left, l.width)).style(Style::new().bg(bg)), l);
        f.render_widget(Paragraph::new(right).style(Style::new().bg(bg)), rr);
    }
}

/// Branch tags that fit in `width` columns: HEAD's branch first, then local branches, tags,
/// and remotes that aren't just a copy of a local branch; the rest become "+N".
fn compact_refs(refs: &[RefLabel], width: usize) -> Vec<Span<'static>> {
    let locals: Vec<&str> = refs
        .iter()
        .filter_map(|r| match r {
            RefLabel::Head(n) | RefLabel::Branch(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    let mut ordered: Vec<&RefLabel> = refs
        .iter()
        .filter(|r| match r {
            // origin/feat duplicates feat: skip it here (the details panel lists everything).
            RefLabel::Remote(n) => !n.split_once('/').is_some_and(|(_, b)| locals.contains(&b)),
            _ => true,
        })
        .collect();
    ordered.sort_by_key(|r| match r {
        RefLabel::Head(_) => 0,
        RefLabel::Branch(_) => 1,
        RefLabel::Tag(_) => 2,
        RefLabel::Remote(_) => 3,
    });

    let mut out: Vec<Span> = Vec::new();
    let mut used = 0;
    for (i, r) in ordered.iter().enumerate() {
        let left_after = ordered.len() - i - 1;
        // Room for this pill, a gap, and a "+N" if more would follow.
        let more_w = if left_after > 0 { format!(" +{left_after}").len() } else { 0 };
        let gap = if out.is_empty() { 0 } else { 1 };
        // Names longer than 20 are shortened; the first one is also shortened to fit the column.
        let pad = if matches!(r, RefLabel::Tag(_)) { 3 } else { 2 };
        let mut max_name = 20;
        if out.is_empty() {
            max_name = max_name.min(width.saturating_sub(pad + more_w)).max(4);
        }
        let name = label_name(r);
        let short = if name.chars().count() > max_name {
            format!("{}…", name.chars().take(max_name - 1).collect::<String>())
        } else {
            name.to_string()
        };
        let pill = ref_pill_named(r, &short);
        if !out.is_empty() && used + gap + pill.width() + more_w > width {
            let rest = ordered.len() - i;
            out.push(Span::styled(format!("{}+{rest}", if out.is_empty() { "" } else { " " }), theme::faint()));
            break;
        }
        if gap == 1 {
            out.push(Span::raw(" "));
        }
        used += gap + pill.width();
        out.push(pill);
    }
    out
}

fn ref_pill(r: &RefLabel) -> Span<'static> {
    ref_pill_named(r, label_name(r))
}

/// A tag for `r`, showing `name` (possibly shortened).
fn ref_pill_named(r: &RefLabel, name: &str) -> Span<'static> {
    match r {
        RefLabel::Head(_) => widgets::pill(name, theme::ACCENT, theme::BG),
        RefLabel::Branch(_) => Span::styled(format!(" {name} "), Style::new().bg(theme::BORDER).fg(theme::GREEN)),
        RefLabel::Remote(_) => Span::styled(format!(" {name} "), Style::new().bg(theme::ELEMENT).fg(theme::MUTED)),
        RefLabel::Tag(_) => Span::styled(format!(" #{name} "), Style::new().bg(theme::ELEMENT).fg(theme::PURPLE)),
    }
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let block = widgets::panel(theme::PANEL);
    let inner_h = block.inner(area).height;
    let Some(d) = app.detail() else {
        f.render_widget(block, area);
        return;
    };
    // Don't scroll past the end.
    let max = (d.lines.len() as u16).saturating_sub(inner_h);
    app.detail_scroll = app.detail_scroll.min(max);
    f.render_widget(Paragraph::new(d.lines.clone()).scroll((app.detail_scroll, 0)).block(block), area);
}

/// Most diff lines shown for one commit, so huge commits stay quick.
const MAX_DIFF_LINES: usize = 4000;

fn detail_lines(repo: &gix::Repository, s: &Shown) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(vec![
        Span::styled(s.id.clone(), Style::new().fg(theme::ACCENT)),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(s.author.clone(), theme::muted()),
        Span::styled(format!(" <{}>", s.email), theme::faint()),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(relative_time(s.time, crate::core::now()), theme::muted()),
    ])];
    if !s.refs.is_empty() {
        let mut spans = Vec::new();
        for r in &s.refs {
            spans.push(ref_pill(r));
            spans.push(Span::raw(" "));
        }
        out.push(Line::raw(""));
        out.push(Line::from(spans));
    }
    if s.parents.len() > 1 {
        out.push(Line::styled(format!("merge of {}", s.parents.join(" + ")), theme::faint()));
    }
    out.push(Line::raw(""));
    out.push(Line::styled(s.subject.clone(), theme::bold()));
    if !s.body.is_empty() {
        out.push(Line::raw(""));
        out.extend(s.body.lines().map(|l| Line::styled(l.to_string(), theme::muted())));
    }

    // File list with totals.
    let (mut ta, mut tr) = (0, 0);
    for f in &s.files {
        if let Some(Size::Lines { added, removed }) = f.size {
            ta += added;
            tr += removed;
        }
    }
    let n = s.files.len();
    out.push(Line::raw(""));
    out.push(Line::from(vec![
        Span::styled(format!("{n} file{}   ", if n == 1 { "" } else { "s" }), theme::faint()),
        Span::styled(format!("+{ta} "), Style::new().fg(theme::GREEN)),
        Span::styled(format!("−{tr}"), Style::new().fg(theme::RED)),
    ]));
    for f in &s.files {
        let (mark, color) = kind_mark(f.kind);
        let mut spans = vec![Span::styled(format!("{mark}  "), Style::new().fg(color)), Span::styled(f.path.clone(), theme::text())];
        if let Some(Size::Lines { added, removed }) = f.size {
            spans.push(Span::styled(format!("  +{added} "), Style::new().fg(theme::GREEN)));
            spans.push(Span::styled(format!("−{removed}"), Style::new().fg(theme::RED)));
        }
        out.push(Line::from(spans));
    }

    // The diff itself.
    let added_bg = Color::Rgb(22, 44, 30);
    let removed_bg = Color::Rgb(52, 24, 27);
    let mut shown = 0;
    for f in &s.files {
        out.push(Line::raw(""));
        let (mark, color) = kind_mark(f.kind);
        let name = match &f.from {
            Some(from) => format!("{from} → {}", f.path),
            None => f.path.clone(),
        };
        out.push(
            Line::from(vec![Span::styled(format!(" {mark} "), Style::new().fg(color)), Span::styled(name, theme::bold())])
                .style(Style::new().bg(theme::ELEMENT)),
        );
        if shown >= MAX_DIFF_LINES {
            out.push(Line::styled("   diff not shown: this commit is large", theme::faint()));
            continue;
        }
        let Some(lines) = show::file_patch(repo, f) else {
            out.push(Line::styled("   binary or very large file", theme::faint()));
            continue;
        };
        for l in lines {
            shown += 1;
            out.push(match l {
                PatchLine::Hunk { new_start, .. } => Line::styled(format!("   ⋯ line {new_start}"), theme::faint()),
                PatchLine::Context { new, text, .. } => {
                    Line::from(vec![Span::styled(format!("{new:>5}  "), theme::faint()), Span::styled(text, theme::muted())])
                }
                PatchLine::Added { new, text } => Line::from(vec![
                    Span::styled(format!("{new:>5} "), Style::new().fg(theme::GREEN)),
                    Span::styled("+", Style::new().fg(theme::GREEN)),
                    Span::styled(text, theme::text()),
                ])
                .style(Style::new().bg(added_bg)),
                PatchLine::Removed { old, text } => Line::from(vec![
                    Span::styled(format!("{old:>5} "), Style::new().fg(theme::RED)),
                    Span::styled("-", Style::new().fg(theme::RED)),
                    Span::styled(text, theme::text()),
                ])
                .style(Style::new().bg(removed_bg)),
            });
        }
    }
    out
}

fn kind_mark(k: ChangeKind) -> (&'static str, Color) {
    match k {
        ChangeKind::Added => ("A", theme::GREEN),
        ChangeKind::Deleted => ("D", theme::RED),
        ChangeKind::Renamed | ChangeKind::Copied => ("R", theme::BLUE),
        ChangeKind::Modified | ChangeKind::TypeChange => ("M", theme::YELLOW),
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let total = app.commits.len();
    let left = if app.searching || !app.query.is_empty() {
        let found = app.visible.len();
        let mut spans = vec![
            Span::styled("/ ", Style::new().fg(theme::ACCENT)),
            Span::styled(app.query.clone(), theme::text()),
        ];
        if app.searching {
            spans.push(Span::styled("▏", Style::new().fg(theme::ACCENT)));
        }
        spans.push(Span::styled(format!("   {found} of {total}"), theme::muted()));
        Line::from(spans)
    } else {
        Line::styled(format!("{total} commit{}", if total == 1 { "" } else { "s" }), theme::muted())
    };
    let right = match (app.opts.mode, app.searching) {
        (_, true) => widgets::hints(&[("enter", "keep filter"), ("esc", "clear")]),
        (Mode::Browse, false) => widgets::hints(&[("/", "search"), ("enter", "copy"), ("ctrl+p", "commands"), ("esc", "quit")]),
        (Mode::Pick, false) => widgets::hints(&[("/", "search"), ("enter", "rebase from here"), ("esc", "cancel")]),
    };
    let [l, r] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right), r);
}

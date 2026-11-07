//! `gg log` (and the commit picker of `gg rebase -p`): commit list, details and diff, full screen.

use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};

use crate::fmt::{now, relative_time};
use crate::git::commit::{self, Details};
use crate::git::log::{Commit, RefLabel};
use crate::tui::app::{self, Flow, Screen};
use crate::tui::kit::sidebar::{self, Section};
use crate::tui::kit::{self, Command, Palette, PaletteEvent, Scroll, Toast, footer, list};
use crate::tui::panels::{changes, diff};
use crate::tui::theme;

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
    Side,
    Choose,
    Quit,
}

/// A row of the list. Commits that branches point at get a label row above them.
#[derive(Clone, Copy)]
enum Item {
    Label(usize),
    Commit(usize),
}

struct App<'r> {
    repo: &'r gix::Repository,
    opts: Options,
    commits: Vec<Commit>,
    /// Rows of the list for the commits that match the search, in order.
    visible: Vec<Item>,
    /// Always the row of a commit, never of a label.
    selected: usize,
    scroll: usize,
    query: String,
    searching: bool,
    details: bool,
    detail_scroll: Scroll,
    /// Each commit read so far, for the details and the sidebar.
    meta: HashMap<gix::ObjectId, Rc<Details>>,
    /// The details panel by (commit, width, sidebar open), filled in `tick` once the
    /// panel is measured; with the sidebar open the panel is only the diff.
    cache: HashMap<(gix::ObjectId, u16, bool), Rc<diff::Patches>>,
    /// The details panel's inner width at the last draw.
    detail_width: u16,
    /// The sidebar, on wide screens, and whether the last draw had room for it.
    side: bool,
    side_open: bool,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    list_area: Rect,
    detail_area: Rect,
    /// The sidebar's file rows, one per file, for clicks.
    files_area: Rect,
    repo_name: String,
    branch: String,
}

enum Effect {
    Copy(String),
    ToggleAll,
}

/// Returns the chosen commit's short id, or `None` if the user quit.
pub fn run(repo: &gix::Repository, opts: Options) -> Result<Option<String>> {
    let mut app = start(repo, opts)?;
    app.side = crate::tui::state::sidebar();
    let out = app::run(&mut app)?;
    crate::tui::state::set_sidebar(app.side);
    Ok(out)
}

/// The screen as it opens, before the first draw.
fn start(repo: &gix::Repository, opts: Options) -> Result<App<'_>> {
    let mut app = App {
        repo,
        commits: Vec::new(),
        visible: Vec::new(),
        selected: 0,
        scroll: 0,
        query: String::new(),
        searching: false,
        details: true,
        detail_scroll: Scroll::default(),
        meta: HashMap::new(),
        cache: HashMap::new(),
        detail_width: 0,
        side: true,
        side_open: false,
        toast: None,
        palette: None,
        list_area: Rect::default(),
        detail_area: Rect::default(),
        files_area: Rect::default(),
        repo_name: crate::git::repo_name(repo),
        branch: repo.head_name().ok().flatten().map(|n| n.shorten().to_string()).unwrap_or_else(|| "detached".into()),
        opts,
    };
    app.load()?;
    Ok(app)
}

impl Screen for App<'_> {
    type Out = Option<String>;
    type Effect = Effect;

    fn draw(&mut self, f: &mut Frame) {
        draw(f, self)
    }
    fn apply(&mut self, e: Effect) -> Result<Flow<Self::Out, Effect>> {
        match e {
            Effect::Copy(id) => {
                crate::clipboard::copy(&id);
                self.toast = Some(Toast::new(format!("Copied {id}"), theme::ACCENT));
            }
            Effect::ToggleAll => {
                self.opts.all = !self.opts.all;
                self.load()?;
                let what = if self.opts.all { "All branches".to_string() } else { format!("Only {}", self.branch) };
                self.toast = Some(Toast::new(what, theme::ACCENT));
            }
        }
        Ok(Flow::Stay)
    }
    fn key(&mut self, k: KeyEvent) -> Flow<Self::Out, Effect> {
        self.on_key(k)
    }
    fn tick(&mut self) -> Result<()> {
        if self.details {
            self.load_detail();
        }
        Ok(())
    }
    fn mouse(&mut self, m: MouseEvent) {
        self.on_mouse(m)
    }
    fn watched(&self) -> Option<(&gix::Repository, bool)> {
        Some((self.repo, false))
    }
    fn refresh(&mut self) -> Result<()> {
        self.load()
    }
    fn toast(&mut self) -> &mut Option<Toast> {
        &mut self.toast
    }
    fn typing(&self) -> bool {
        self.searching
    }
}

impl App<'_> {
    fn load(&mut self) -> Result<()> {
        self.commits = crate::git::log::read(self.repo, self.opts.count, self.opts.all)?;
        self.rebuild();
        Ok(())
    }

    /// Recomputes the visible rows after loading or a search change, keeping the selected commit.
    fn rebuild(&mut self) {
        let keep = self.selected_commit().map(|c| c.oid);
        let q = self.query.to_lowercase();
        self.visible = (self.commits.iter().enumerate())
            .filter(|(_, c)| q.is_empty() || matches(c, &q))
            .flat_map(|(i, c)| (!c.refs.is_empty()).then_some(Item::Label(i)).into_iter().chain([Item::Commit(i)]))
            .collect();
        let first = self.visible.iter().position(|it| matches!(it, Item::Commit(_)));
        self.selected = keep
            .and_then(|oid| {
                self.visible.iter().position(|it| matches!(it, Item::Commit(i) if self.commits[*i].oid == oid))
            })
            .or(first)
            .unwrap_or(0);
        self.detail_scroll.reset();
    }

    fn selected_commit(&self) -> Option<&Commit> {
        match self.visible.get(self.selected)? {
            Item::Commit(i) => Some(&self.commits[*i]),
            Item::Label(_) => None,
        }
    }

    fn commit_count(&self) -> usize {
        self.visible.iter().filter(|it| matches!(it, Item::Commit(_))).count()
    }

    /// Moves `steps` commits up (negative) or down, skipping label rows.
    fn step(&mut self, steps: isize) {
        let mut i = self.selected;
        let mut left = steps.unsigned_abs();
        while left > 0 {
            let next = if steps < 0 {
                self.visible[..i].iter().rposition(|it| matches!(it, Item::Commit(_)))
            } else {
                self.visible.iter().skip(i + 1).position(|it| matches!(it, Item::Commit(_))).map(|p| p + i + 1)
            };
            match next {
                Some(n) => i = n,
                None => break,
            }
            left -= 1;
        }
        if i != self.selected {
            self.selected = i;
            self.detail_scroll.reset();
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
        c.push(Command { label: "Sidebar: show or hide (wide screens)", keys: "i", action: Cmd::Side });
        c.push(Command { label: "Quit", keys: "esc", action: Cmd::Quit });
        c
    }

    fn on_key(&mut self, k: KeyEvent) -> Flow<Option<String>, Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Flow::Done(None);
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
            return Flow::Stay;
        }

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
            return Flow::Stay;
        }

        if self.detail_scroll.key(k, self.detail_area.height) {
            return Flow::Stay;
        }
        let cmd = match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(self.commands()));
                return Flow::Stay;
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(self.commands()));
                return Flow::Stay;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(1);
                return Flow::Stay;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(-1);
                return Flow::Stay;
            }
            KeyCode::PageDown => {
                self.step(15);
                return Flow::Stay;
            }
            KeyCode::PageUp => {
                self.step(-15);
                return Flow::Stay;
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.step(-(self.visible.len() as isize));
                return Flow::Stay;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.step(self.visible.len() as isize);
                return Flow::Stay;
            }
            KeyCode::Char('n') if self.details => {
                self.jump(true);
                return Flow::Stay;
            }
            KeyCode::Char('N') if self.details => {
                self.jump(false);
                return Flow::Stay;
            }
            KeyCode::Char('/') => Cmd::Search,
            KeyCode::Char('y') if self.opts.mode == Mode::Browse => Cmd::Copy,
            KeyCode::Char('a') if self.opts.mode == Mode::Browse => Cmd::ToggleAll,
            KeyCode::Tab => Cmd::Details,
            KeyCode::Char('i') => Cmd::Side,
            KeyCode::Enter => Cmd::Choose,
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.rebuild();
                return Flow::Stay;
            }
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Flow::Stay,
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Flow<Option<String>, Effect> {
        match cmd {
            Cmd::Search => self.searching = true,
            Cmd::Copy => {
                if let Some(c) = self.selected_commit() {
                    return Flow::Run(Effect::Copy(c.id.clone()));
                }
            }
            Cmd::ToggleAll => return Flow::Run(Effect::ToggleAll),
            Cmd::Details => self.details = !self.details,
            Cmd::Side => self.side = !self.side,
            Cmd::Choose => return Flow::Done(self.selected_commit().map(|c| c.id.clone())),
            Cmd::Quit => return Flow::Done(None),
        }
        Flow::Stay
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if self.palette.is_some() {
            return;
        }
        let in_details = list::inside(self.detail_area, &m);
        match m.kind {
            MouseEventKind::ScrollDown if in_details => self.detail_scroll.wheel(true),
            MouseEventKind::ScrollUp if in_details => self.detail_scroll.wheel(false),
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::ScrollUp => self.step(-1),
            MouseEventKind::Down(MouseButton::Left) if self.side_open && list::inside(self.files_area, &m) => {
                let file = (m.row - self.files_area.y) as usize;
                if let Some(at) = self.detail().and_then(|d| d.files.get(file).copied()) {
                    self.detail_scroll.to(at);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(mut i) = list::row_at(self.list_area, self.scroll, &m) else { return };
                // A click on a label selects the commit below it.
                if matches!(self.visible.get(i), Some(Item::Label(_))) {
                    i += 1;
                }
                if i < self.visible.len() && i != self.selected {
                    self.selected = i;
                    self.detail_scroll.reset();
                }
            }
            _ => {}
        }
    }

    /// Reads the selected commit's details into the cache, unless they're there.
    fn load_detail(&mut self) {
        let Some(oid) = self.selected_commit().map(|c| c.oid) else { return };
        // Zero until the first draw has measured the panel.
        let key = (oid, self.detail_width, self.side_open);
        if key.1 == 0 || self.cache.contains_key(&key) {
            return;
        }
        if !self.meta.contains_key(&oid) {
            match commit::read(self.repo, &oid.to_string()) {
                Ok(d) => {
                    self.meta.insert(oid, Rc::new(d));
                }
                Err(e) => {
                    let lines = vec![Line::styled(format!("Couldn't read this commit: {e}"), theme::muted())];
                    self.cache.insert(key, Rc::new(diff::Patches { lines, ..Default::default() }));
                    return;
                }
            }
        }
        let d = &self.meta[&oid];
        // The sidebar has the message and the files; the panel keeps the diff.
        let p = if key.2 { diff::patches(self.repo, &d.files) } else { detail_lines(self.repo, d) };
        self.cache.insert(key, Rc::new(p));
    }

    /// Scrolls the details to the next hunk (or the one before), if there is one.
    fn jump(&mut self, forward: bool) {
        let Some(d) = self.detail() else { return };
        let top = self.detail_scroll.top() as usize;
        let to = if forward { d.hunks.iter().find(|&&h| h > top) } else { d.hunks.iter().rev().find(|&&h| h < top) };
        match to {
            Some(&h) => self.detail_scroll.to(h),
            // Before the first hunk: back to the top, where the message is.
            None if !forward => self.detail_scroll.reset(),
            None => {}
        }
    }

    fn detail(&self) -> Option<Rc<diff::Patches>> {
        self.cache.get(&(self.selected_commit()?.oid, self.detail_width, self.side_open)).cloned()
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
    app.side_open = false;
    let (list, details) = if !app.details {
        (body, None)
    } else {
        let p = kit::panes(body, Some(list_width(app)), app.visible.len() as u16 + 2, app.side);
        if let Some(side) = p.side {
            app.side_open = true;
            app.files_area = draw_side(f, app, side);
        }
        (p.list, Some(p.main))
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

/// The list's width that fits its widest commit row: bar, id, subject, then author
/// and age as `draw_list` lays them out.
fn list_width(app: &App) -> u16 {
    let widest = app.commits.iter().map(|c| c.id.len() + crate::fmt::width(&c.summary)).max().unwrap_or(0);
    (2 + 2 + widest + 22).min(u16::MAX as usize) as u16
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(0, 0, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;

    let height = inner.height as usize;
    // Keep the selected commit on screen, together with its label row if it has one.
    let first = match app.selected.checked_sub(1).map(|i| app.visible[i]) {
        Some(Item::Label(_)) => app.selected - 1,
        _ => app.selected,
    };
    app.scroll = list::follow(app.scroll, first, app.selected, height);

    if app.visible.is_empty() {
        let msg = if app.query.is_empty() { "No commits yet" } else { "No commits match" };
        f.render_widget(Paragraph::new(Span::styled(format!("  {msg}"), theme::muted())), inner);
        return;
    }

    let now = now();
    for (n, item) in app.visible.iter().enumerate().skip(app.scroll).take(height) {
        let r = Rect { y: inner.y + (n - app.scroll) as u16, height: 1, ..inner };
        // Only the commit row is highlighted, never its label.
        let bg = if n == app.selected { theme::ELEMENT } else { theme::PANEL };

        match *item {
            Item::Label(index) => {
                let mut spans = vec![Span::styled("  ╭ ", theme::faint())];
                for (i, label) in label_order(&app.commits[index].refs).into_iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::raw(" "));
                    }
                    spans.push(ref_pill(label));
                }
                f.render_widget(
                    Paragraph::new(kit::truncate(Line::from(spans), r.width)).style(Style::new().bg(bg)),
                    r,
                );
            }
            Item::Commit(index) => {
                let c = &app.commits[index];
                let bar = if n == app.selected {
                    Span::styled("┃ ", Style::new().fg(theme::ACCENT))
                } else {
                    Span::raw("  ")
                };
                let subject_style = if c.is_merge { theme::muted() } else { theme::text() };
                let left = Line::from(vec![
                    bar,
                    Span::styled(c.id.clone(), theme::faint()),
                    Span::raw("  "),
                    Span::styled(c.summary.clone(), subject_style),
                ]);
                let author = c.author.split_whitespace().next().unwrap_or(&c.author).to_string();
                let right = Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{author:<10}"), theme::muted()),
                    Span::styled(format!("{:>8}", relative_time(c.time, now)), theme::faint()),
                    Span::raw("  "),
                ]);
                let [l, rr] =
                    Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(r);
                f.render_widget(Paragraph::new(kit::truncate(left, l.width)).style(Style::new().bg(bg)), l);
                f.render_widget(Paragraph::new(right).style(Style::new().bg(bg)), rr);
            }
        }
    }
}

/// HEAD's branch first, then local branches, tags and remotes.
fn label_order(refs: &[RefLabel]) -> Vec<&RefLabel> {
    let mut ordered: Vec<&RefLabel> = refs.iter().collect();
    ordered.sort_by_key(|r| match r {
        RefLabel::Head(_) => 0,
        RefLabel::Branch(_) => 1,
        RefLabel::Tag(_) => 2,
        RefLabel::Remote(_) => 3,
    });
    ordered
}

fn ref_pill(r: &RefLabel) -> Span<'static> {
    match r {
        RefLabel::Head(n) => kit::pill(n, theme::ACCENT, theme::BG),
        RefLabel::Branch(n) => Span::styled(format!(" {n} "), Style::new().bg(theme::BORDER).fg(theme::GREEN)),
        RefLabel::Remote(n) => Span::styled(format!(" {n} "), Style::new().bg(theme::ELEMENT).fg(theme::MUTED)),
        RefLabel::Tag(n) => Span::styled(format!(" #{n} "), Style::new().bg(theme::ELEMENT).fg(theme::PURPLE)),
    }
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let inner = block.inner(area);
    app.detail_width = inner.width;
    let Some(d) = app.detail() else {
        f.render_widget(block, area);
        return;
    };
    f.render_widget(Paragraph::new(app.detail_scroll.view(&d.lines, &d.pins, inner)).block(block), area);
    app.detail_scroll.draw_more(f, area);
}

/// The sidebar: the commit's id, author, refs and message, then its files. Gives where
/// the file rows are.
fn draw_side(f: &mut Frame, app: &App, area: Rect) -> Rect {
    let w = (area.width as usize).saturating_sub(4);
    let Some(d) = app.selected_commit().and_then(|c| app.meta.get(&c.oid)) else {
        let reading = Section { title: "", count: None, lines: vec![Line::styled("reading…", theme::faint())] };
        sidebar::draw(f, area, vec![reading]);
        return Rect::default();
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(d.id.clone(), Style::new().fg(theme::ACCENT)),
            Span::styled(format!("  {}", d.author), theme::muted()),
        ]),
        Line::styled(format!("{} · {}", d.email, relative_time(d.time, now())), theme::faint()),
    ];
    if !d.refs.is_empty() {
        lines.push(Line::from(d.refs.iter().flat_map(|r| [ref_pill(r), Span::raw(" ")]).collect::<Vec<_>>()));
    }
    if d.parents.len() > 1 {
        lines.push(Line::styled(format!("merge of {}", d.parents.join(" + ")), theme::faint()));
    }
    lines.push(Line::raw(""));
    lines.extend(kit::wrap(&d.subject, w).into_iter().map(|l| Line::styled(l, theme::bold())));
    if !d.body.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(kit::reflow(&d.body, w).into_iter().map(|l| Line::styled(l, theme::muted())));
    }
    let files = d.files.iter().map(|f| changes::Item::stat(f).row()).collect();
    let sections = vec![
        Section { title: "", count: None, lines },
        Section { title: "files", count: Some(d.files.len()), lines: files },
    ];
    sidebar::draw(f, area, sections)[1]
}

fn detail_lines(repo: &gix::Repository, s: &Details) -> diff::Patches {
    let mut out = vec![Line::from(vec![
        Span::styled(s.id.clone(), Style::new().fg(theme::ACCENT)),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(s.author.clone(), theme::muted()),
        Span::styled(format!(" <{}>", s.email), theme::faint()),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(relative_time(s.time, now()), theme::muted()),
    ])];
    if !s.refs.is_empty() {
        out.push(Line::raw(""));
        out.push(Line::from(s.refs.iter().flat_map(|r| [ref_pill(r), Span::raw(" ")]).collect::<Vec<_>>()));
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

    out.push(Line::raw(""));
    out.extend(changes::files(&s.files, None));
    diff::patches(repo, &s.files).below(out)
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let total = app.commits.len();
    let left = if app.searching || !app.query.is_empty() {
        footer::search(&app.query, app.searching, app.commit_count(), total)
    } else {
        Line::styled(format!("{total} commit{}", if total == 1 { "" } else { "s" }), theme::muted())
    };
    let mut pairs = match (app.opts.mode, app.searching) {
        (_, true) => vec![("enter", "keep filter"), ("esc", "clear")],
        (Mode::Browse, false) => vec![("/", "search"), ("enter", "copy"), ("ctrl+p", "commands"), ("esc", "quit")],
        (Mode::Pick, false) => vec![("/", "search"), ("enter", "rebase from here"), ("esc", "cancel")],
    };
    if !app.searching && app.details {
        if app.detail_scroll.wide() {
            pairs.insert(pairs.len() - 1, ("← →", "long lines"));
        }
        footer::sidebar(&mut pairs, area.width);
    }
    footer::draw(f, area, left, pairs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::press;

    #[test]
    fn copies_the_commit_under_the_cursor() {
        let s = crate::git::run::Scratch::new("log-copy");
        s.commit("a.txt", "one\n");
        let repo = s.repo();
        let mut app = start(&repo, Options { mode: Mode::Browse, all: false, count: 10 }).unwrap();
        let head = repo.head_id().unwrap().to_string();
        let Flow::Run(Effect::Copy(id)) = app.key(press(KeyCode::Char('y'))) else { panic!("y should copy") };
        assert!(!id.is_empty() && head.starts_with(&id));
    }

    #[test]
    fn the_sidebar_takes_the_message_and_files_on_wide_screens() {
        let s = crate::git::run::Scratch::new("log-side");
        s.commit("a.txt", "one\n");
        let repo = s.repo();
        let mut app = start(&repo, Options { mode: Mode::Browse, all: false, count: 10 }).unwrap();
        // Draw, read the details for that width, draw again: the event loop's order.
        let screen = |app: &mut App, w: u16| {
            let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, 24)).unwrap();
            t.draw(|f| app.draw(f)).unwrap();
            app.tick().unwrap();
            t.draw(|f| app.draw(f)).unwrap();
            let cells: Vec<&str> = t.backend().buffer().content().iter().map(|c| c.symbol()).collect();
            cells.concat()
        };
        let wide = screen(&mut app, 200);
        assert!(app.side_open && wide.contains("files   1"), "{wide}");
        // The panel is the diff only: the subject `a.txt` shows in the list and the sidebar.
        let panel = app.detail().unwrap();
        assert!(!panel.lines.iter().any(|l| l.to_string() == "a.txt"), "{:?}", panel.lines);
        app.key(press(KeyCode::Char('i')));
        screen(&mut app, 200);
        assert!(!app.side_open && app.detail().unwrap().lines.iter().any(|l| l.to_string() == "a.txt"));
    }

    #[test]
    fn n_jumps_to_hunks_and_a_click_on_a_file_to_its_diff() {
        let s = crate::git::run::Scratch::new("log-jump");
        let old: String = (1..=30).map(|i| format!("{i}\n")).collect();
        s.commit("a.txt", &old);
        s.write("a.txt", &old.replace("\n2\n", "\ntwo\n").replace("25\n", "twenty-five\n"));
        s.write("b.txt", "b\n");
        s.git(&["add", "."]);
        s.git(&["commit", "-q", "-m", "two files"]);
        let repo = s.repo();
        let mut app = start(&repo, Options { mode: Mode::Browse, all: false, count: 10 }).unwrap();
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 30)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        app.tick().unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        let d = app.detail().unwrap();
        assert_eq!((d.files.len(), d.hunks.len()), (2, 3), "{:?}", d.lines);

        app.key(press(KeyCode::Char('n')));
        assert_eq!(app.detail_scroll.top() as usize, d.hunks[0]);
        app.key(press(KeyCode::Char('n')));
        assert_eq!(app.detail_scroll.top() as usize, d.hunks[1]);
        app.key(press(KeyCode::Char('N')));
        assert_eq!(app.detail_scroll.top() as usize, d.hunks[0]);

        // The second file's row in the sidebar.
        let at = app.files_area;
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: at.x + 2,
            row: at.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(click);
        assert_eq!(app.detail_scroll.top() as usize, d.files[1]);
    }
}

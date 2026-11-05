//! `gg wt`: every worktree of the repo, full screen. See what each holds, add, remove,
//! prune, lock, or pick one: enter prints its folder, so `cd "$(gg wt)"` jumps there.
//! Writes go through `git worktree`.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use gix::bstr::ByteSlice;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};

use super::app::{self, Flow, Screen};
use super::kit::{self, Command, Confirm, Input, InputEvent, Palette, PaletteEvent, Scroll, Toast, footer, list};
use super::{diffview, first_lines, theme};
use crate::fmt::paths::tilde;
use crate::fmt::{now, relative_time};
use crate::git::diff::Size;
use crate::git::status::{FileChange, Status};
use crate::git::worktree::{self, Source, Worktree};

#[derive(Clone)]
enum Cmd {
    Go,
    New,
    Remove,
    Prune,
    Lock,
    Copy,
    Reload,
    Quit,
}

/// What an open dialog will do on yes.
enum Ask {
    Remove { path: PathBuf, force: bool },
    Prune,
}

/// Uncommitted work in a worktree; read in the background, it can take a moment.
enum Load {
    Loading,
    Ready(Box<Status>),
    Failed(String),
    /// Bare, or its folder is gone.
    None,
}

struct Row {
    wt: Worktree,
    short: String,
    subject: String,
    time: i64,
    /// This process runs inside it.
    here: bool,
    load: Load,
}

impl Row {
    fn label(&self) -> String {
        match (&self.wt.branch, self.wt.bare) {
            (_, true) => "(bare)".into(),
            (Some(b), _) => b.clone(),
            (None, _) => format!("detached at {}", self.short),
        }
    }

    /// Uncommitted paths, untracked included; `None` until known.
    fn dirty(&self) -> Option<usize> {
        match &self.load {
            Load::Ready(st) => Some(st.staged.len() + st.unstaged.len() + st.untracked.len() + st.conflicted.len()),
            _ => None,
        }
    }
}

struct App<'r> {
    repo: &'r gix::Repository,
    rows: Vec<Row>,
    selected: usize,
    scroll: usize,
    detail_scroll: Scroll,
    list_area: Rect,
    detail_area: Rect,
    states: Option<Receiver<(PathBuf, Load)>>,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    confirm: Option<(Confirm, Ask)>,
    input: Option<Input>,
    repo_name: String,
    /// Our stdout is captured (`cd "$(gg wt)"`): enter hands the folder over.
    captured: bool,
}

/// Returns the folder picked with enter.
pub fn run(repo: &gix::Repository) -> Result<Option<PathBuf>> {
    let mut app = App {
        repo,
        rows: Vec::new(),
        selected: 0,
        scroll: 0,
        detail_scroll: Scroll::default(),
        list_area: Rect::default(),
        detail_area: Rect::default(),
        states: None,
        toast: None,
        palette: None,
        confirm: None,
        input: None,
        repo_name: crate::git::repo_name(repo),
        captured: !std::io::stdout().is_terminal(),
    };
    app.reload(None)?;
    // Start on the worktree we're in.
    app.selected = app.rows.iter().position(|r| r.here).unwrap_or(0);

    app::run(&mut app)
}

impl Screen for App<'_> {
    type Out = Option<PathBuf>;
    // Folder states arrive one by one; show them soon.
    const WAKE: Duration = Duration::from_millis(150);

    fn draw(&mut self, f: &mut Frame) {
        draw(f, self)
    }
    fn key(&mut self, k: KeyEvent) -> Result<Flow<Self::Out>> {
        Ok(self.on_key(k)?.into())
    }
    fn mouse(&mut self, m: MouseEvent) {
        self.on_mouse(m)
    }
    fn tick(&mut self) -> Result<()> {
        self.poll_states();
        Ok(())
    }
    fn toast(&mut self) -> &mut Option<Toast> {
        &mut self.toast
    }
}

fn commands() -> Vec<Command<Cmd>> {
    vec![
        Command { label: "Go to worktree (print its folder)", keys: "enter", action: Cmd::Go },
        Command { label: "New worktree", keys: "n", action: Cmd::New },
        Command { label: "Remove worktree", keys: "d", action: Cmd::Remove },
        Command { label: "Prune worktrees whose folder is gone", keys: "p", action: Cmd::Prune },
        Command { label: "Lock or unlock worktree", keys: "l", action: Cmd::Lock },
        Command { label: "Copy folder path", keys: "y", action: Cmd::Copy },
        Command { label: "Reload", keys: "r", action: Cmd::Reload },
        Command { label: "Quit", keys: "q", action: Cmd::Quit },
    ]
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Reads every worktree's uncommitted work on its own thread.
fn load_states(paths: Vec<PathBuf>) -> Receiver<(PathBuf, Load)> {
    let (tx, rx) = mpsc::channel();
    for path in paths {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let load = match gix::open(&path) {
                Ok(repo) => match crate::git::status::read(&repo, true) {
                    Ok(st) => Load::Ready(Box::new(st)),
                    Err(e) => Load::Failed(e.to_string()),
                },
                Err(e) => Load::Failed(e.to_string()),
            };
            let _ = tx.send((path, load));
        });
    }
    rx
}

impl App<'_> {
    /// Lists worktrees again and starts reading their state. `select` puts the cursor on that folder.
    fn reload(&mut self, select: Option<&Path>) -> Result<()> {
        let keep = select.map(Path::to_path_buf).or_else(|| self.current().map(|r| r.wt.path.clone()));
        let here = self.repo.workdir().map(Path::to_path_buf);
        let mut rows = Vec::new();
        for wt in worktree::list(self.repo)? {
            let commit = wt.head.and_then(|h| self.repo.find_commit(h).ok());
            let (short, subject, time) = match &commit {
                Some(c) => (
                    c.id().shorten_or_id().to_string(),
                    c.message().map(|m| m.summary().to_str_lossy().into_owned()).unwrap_or_default(),
                    c.time().map(|t| t.seconds).unwrap_or(0),
                ),
                None => (String::new(), String::new(), 0),
            };
            let gone = wt.bare || wt.prunable.is_some() || !wt.path.exists();
            rows.push(Row {
                here: here.as_deref().is_some_and(|h| same_dir(h, &wt.path)),
                load: if gone { Load::None } else { Load::Loading },
                wt,
                short,
                subject,
                time,
            });
        }
        let paths = rows.iter().filter(|r| matches!(r.load, Load::Loading)).map(|r| r.wt.path.clone()).collect();
        self.states = Some(load_states(paths));
        self.rows = rows;
        if let Some(p) = keep.and_then(|k| self.rows.iter().position(|r| same_dir(&r.wt.path, &k))) {
            self.selected = p;
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        Ok(())
    }

    fn poll_states(&mut self) {
        let Some(rx) = &self.states else { return };
        while let Ok((path, load)) = rx.try_recv() {
            if let Some(r) = self.rows.iter_mut().find(|r| r.wt.path == path) {
                r.load = load;
            }
        }
    }

    fn current(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    fn main(&self) -> Option<PathBuf> {
        self.rows.iter().find(|r| r.wt.main).map(|r| r.wt.path.clone())
    }

    /// How a worktree is named in messages: like the list does.
    fn name(&self, p: &Path) -> String {
        crate::fmt::paths::beside(p, self.main().as_deref())
    }

    fn step(&mut self, n: isize) {
        let last = self.rows.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + n).clamp(0, last) as usize;
        self.detail_scroll.reset();
    }

    /// Returns `Some` to quit, with the folder picked (if any).
    fn on_key(&mut self, k: KeyEvent) -> Result<Option<Option<PathBuf>>> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(input) = &mut self.input {
            match input.key(k) {
                Some(InputEvent::Cancel) => self.input = None,
                Some(InputEvent::Done(text)) => {
                    self.input = None;
                    self.create(text.trim())?;
                }
                None => {}
            }
            return Ok(None);
        }
        if ctrl && k.code == KeyCode::Char('c') {
            return Ok(Some(None));
        }
        if self.confirm.is_some() {
            match k.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let (_, ask) = self.confirm.take().unwrap();
                    self.answer(ask)?;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => self.confirm = None,
                _ => {}
            }
            return Ok(None);
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

        if self.detail_scroll.key(k, self.detail_area.height) {
            return Ok(None);
        }
        let cmd = match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(commands()));
                return Ok(None);
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(commands()));
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
            KeyCode::Home | KeyCode::Char('g') => {
                self.step(isize::MIN / 2);
                return Ok(None);
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.step(isize::MAX / 2);
                return Ok(None);
            }
            KeyCode::Enter => Cmd::Go,
            KeyCode::Char('n') => Cmd::New,
            KeyCode::Char('d') => Cmd::Remove,
            KeyCode::Char('p') => Cmd::Prune,
            KeyCode::Char('l') => Cmd::Lock,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('r') => Cmd::Reload,
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Ok(None),
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Result<Option<Option<PathBuf>>> {
        match cmd {
            Cmd::Quit => return Ok(Some(None)),
            Cmd::Reload => self.reload(None)?,
            Cmd::Go => {
                if let Some(r) = self.current() {
                    if matches!(r.load, Load::None) && !r.wt.bare {
                        self.toast = Some(Toast::new("Its folder is gone: p prunes it", theme::YELLOW));
                    } else {
                        let path = r.wt.path.clone();
                        // Printed on the way out; in a plain terminal it's copied too.
                        if !self.captured {
                            crate::clipboard::copy(&path.display().to_string());
                        }
                        return Ok(Some(Some(path)));
                    }
                }
            }
            Cmd::Copy => {
                if let Some(r) = self.current() {
                    crate::clipboard::copy(&r.wt.path.display().to_string());
                    self.toast = Some(Toast::new(format!("Copied {}", tilde(&r.wt.path)), theme::ACCENT));
                }
            }
            Cmd::New => {
                let main = self.main().unwrap_or_default();
                let name = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let head = self.repo.head_id().map(|h| h.shorten_or_id().to_string()).unwrap_or_default();
                self.input = Some(
                    Input::new("New worktree", &self.repo_name, "", "Create")
                        .placeholder("branch name")
                        .note(format!(
                            "Goes next to {name}, as {name}-<branch>. An existing branch (local or\non a remote) is checked out there; a new one starts from HEAD ({head})."
                        )),
                );
            }
            Cmd::Remove => self.ask_remove(),
            Cmd::Prune => self.ask_prune(),
            Cmd::Lock => {
                if let Some(r) = self.current() {
                    let on = r.wt.locked.is_none();
                    let done = if on { "Locked" } else { "Unlocked" };
                    let (ok, out) = worktree::lock(&r.wt.path, on);
                    let name = self.name(&r.wt.path);
                    self.toast = Some(if ok {
                        Toast::new(format!("{done} {name}"), theme::GREEN)
                    } else {
                        Toast::new(first_lines(&out), theme::RED)
                    });
                    self.reload(None)?;
                }
            }
        }
        Ok(None)
    }

    /// `git worktree add` for `branch`: checks it out if it exists, else creates it from HEAD.
    fn create(&mut self, branch: &str) -> Result<()> {
        if branch.is_empty() {
            return Ok(());
        }
        let Some(main) = self.main() else { return Ok(()) };
        let path = worktree::suggest_path(&main, branch);
        if path.exists() {
            self.toast = Some(Toast::new(format!("{} already exists", tilde(&path)), theme::RED));
            return Ok(());
        }
        let from = worktree::source(self.repo, branch);
        let (ok, out) = worktree::add(&path, branch, from);
        self.toast = Some(if ok {
            let how = match from {
                Source::Local => "on",
                Source::Remote => "tracking the remote",
                Source::New => "on new branch",
            };
            Toast::new(format!("Created {} {how} {branch}", self.name(&path)), theme::GREEN)
        } else {
            Toast::new(first_lines(&out), theme::RED)
        });
        self.reload(ok.then_some(path.as_path()))
    }

    fn ask_remove(&mut self) {
        let Some(r) = self.current() else { return };
        let name = self.name(&r.wt.path);
        let refuse = if r.wt.main {
            Some("The main worktree can't be removed".to_string())
        } else if r.here {
            Some("You're inside this worktree. Run gg wt from another one to remove it".into())
        } else if r.wt.locked.is_some() {
            Some("It's locked: l unlocks it first".into())
        } else if matches!(r.load, Load::None) {
            Some("Its folder is already gone: p prunes it".into())
        } else if matches!(r.load, Load::Loading) {
            Some("Still checking it for changes; try again in a moment".into())
        } else {
            None
        };
        if let Some(why) = refuse {
            self.toast = Some(Toast::new(why, theme::YELLOW));
            return;
        }
        let keeps = match &r.wt.branch {
            Some(b) => format!("The branch {b} stays."),
            None => "It's on a detached HEAD: commits made only there may become hard to find.".into(),
        };
        let dirty = r.dirty().unwrap_or(0);
        let (body, force) = if dirty > 0 {
            let s = if dirty == 1 { "" } else { "s" };
            (
                format!(
                    "It has {dirty} file{s} with uncommitted changes (untracked included).\nThey will be lost.\n{keeps}"
                ),
                true,
            )
        } else {
            (format!("Its folder is deleted. Nothing uncommitted is lost.\n{keeps}"), false)
        };
        let confirm = Confirm {
            title: format!("Remove {name}?"),
            body,
            yes: "Remove".into(),
            no: "Cancel".into(),
            danger: force,
        };
        self.confirm = Some((confirm, Ask::Remove { path: r.wt.path.clone(), force }));
    }

    fn ask_prune(&mut self) {
        let gone: Vec<&Row> = self.rows.iter().filter(|r| r.wt.prunable.is_some()).collect();
        if gone.is_empty() {
            self.toast = Some(Toast::new("Nothing to prune: every worktree folder is there", theme::ACCENT));
            return;
        }
        let mut body: String = gone.iter().map(|r| format!("$ {}  {}\n", self.name(&r.wt.path), r.label())).collect();
        body += "Their folders are gone; git forgets them. Branches stay.";
        let n = gone.len();
        let confirm = Confirm {
            title: format!("Prune {n} worktree{}?", if n == 1 { "" } else { "s" }),
            body,
            yes: "Prune".into(),
            no: "Cancel".into(),
            danger: false,
        };
        self.confirm = Some((confirm, Ask::Prune));
    }

    fn answer(&mut self, ask: Ask) -> Result<()> {
        match ask {
            Ask::Remove { path, force } => {
                let (ok, out) = worktree::remove(&path, force);
                self.toast = Some(if ok {
                    Toast::new(format!("Removed {}", self.name(&path)), theme::GREEN)
                } else {
                    Toast::new(first_lines(&out), theme::RED)
                });
            }
            Ask::Prune => {
                let before = self.rows.iter().filter(|r| r.wt.prunable.is_some()).count();
                let (ok, out) = worktree::prune();
                self.toast = Some(if ok {
                    Toast::new(format!("Pruned {before}"), theme::GREEN)
                } else {
                    Toast::new(first_lines(&out), theme::RED)
                });
            }
        }
        self.reload(None)
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if self.palette.is_some() || self.confirm.is_some() || self.input.is_some() {
            return;
        }
        let in_details = list::inside(self.detail_area, &m);
        match m.kind {
            MouseEventKind::ScrollDown if in_details => self.detail_scroll.wheel(true),
            MouseEventKind::ScrollUp if in_details => self.detail_scroll.wheel(false),
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::ScrollUp => self.step(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(i) = list::row_at(self.list_area, self.scroll, &m)
                    && i < self.rows.len()
                {
                    self.selected = i;
                    self.detail_scroll.reset();
                }
            }
            _ => {}
        }
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
    let (list, details) = if body.width >= 120 {
        let [l, _, r] =
            Layout::horizontal([Constraint::Percentage(55), Constraint::Length(1), Constraint::Min(40)]).areas(body);
        (l, r)
    } else {
        // Not `clamp`: it panics when a short terminal makes the upper bound the smaller one.
        let rows = (app.rows.len() as u16 + 2).min(body.height / 2).max(4);
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
            files(&mut out, "staged", &st.staged);
            files(&mut out, "changes", &st.unstaged);
            names(&mut out, "untracked", &st.untracked, theme::MUTED);
            names(&mut out, "conflicts", &st.conflicted, theme::RED);
        }
    }
    out
}

fn files(out: &mut Vec<Line<'static>>, title: &str, list: &[FileChange]) {
    if list.is_empty() {
        return;
    }
    let (mut ta, mut tr) = (0, 0);
    for f in list {
        if let Some(Size::Lines { added, removed }) = f.size {
            ta += added;
            tr += removed;
        }
    }
    out.push(Line::from(vec![
        Span::styled(format!("{title} {}   ", list.len()), theme::faint()),
        Span::styled(format!("+{ta} "), Style::new().fg(theme::GREEN)),
        Span::styled(format!("−{tr}"), Style::new().fg(theme::RED)),
    ]));
    for f in list {
        let (mark, color) = diffview::kind_mark(f.kind);
        let mut spans = vec![
            Span::styled(format!("{mark}  "), Style::new().fg(color)),
            Span::styled(f.path.clone(), theme::text()),
        ];
        if let Some(Size::Lines { added, removed }) = f.size {
            spans.push(Span::styled(format!("  +{added} "), Style::new().fg(theme::GREEN)));
            spans.push(Span::styled(format!("−{removed}"), Style::new().fg(theme::RED)));
        }
        out.push(Line::from(spans));
    }
    out.push(Line::raw(""));
}

fn names(out: &mut Vec<Line<'static>>, title: &str, list: &[String], color: ratatui::style::Color) {
    if list.is_empty() {
        return;
    }
    out.push(Line::styled(format!("{title} {}", list.len()), theme::faint()));
    for p in list {
        out.push(Line::from(vec![Span::styled("?  ", Style::new().fg(color)), Span::styled(p.clone(), theme::text())]));
    }
    out.push(Line::raw(""));
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

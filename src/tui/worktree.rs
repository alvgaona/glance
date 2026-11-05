//! `gg wt`: every worktree of the repo, full screen. See what each holds, add, remove,
//! prune, lock, or pick one: enter prints its folder, so `cd "$(gg wt)"` jumps there.
//! Writes go through `git worktree`.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use gix::bstr::ByteSlice;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};

use super::widgets::{self, Command, Confirm, Input, InputEvent, Palette, PaletteEvent, Toast};
use super::{diffview, enter, first_lines, git, leave, theme};
use crate::core::diff::Size;
use crate::core::status::{FileChange, Status};
use crate::core::worktree::{self, Worktree};
use crate::render::relative_time;

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
    detail_scroll: u16,
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
        detail_scroll: 0,
        list_area: Rect::default(),
        detail_area: Rect::default(),
        states: None,
        toast: None,
        palette: None,
        confirm: None,
        input: None,
        repo_name: crate::core::repo_name(repo),
        captured: !std::io::stdout().is_terminal(),
    };
    app.reload(None)?;
    // Start on the worktree we're in.
    app.selected = app.rows.iter().position(|r| r.here).unwrap_or(0);

    let mut terminal = enter();
    let result = (|| -> Result<Option<PathBuf>> {
        loop {
            if app.toast.as_ref().is_some_and(Toast::expired) {
                app.toast = None;
            }
            app.poll_states();
            terminal.draw(|f| draw(f, &mut app))?;
            if !event::poll(Duration::from_millis(150))? {
                continue;
            }
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if let Some(done) = app.key(k)? {
                        return Ok(done);
                    }
                }
                Event::Mouse(m) => app.mouse(m),
                _ => {}
            }
        }
    })();
    leave();
    result
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

/// `/Users/me/git/app` -> `~/git/app`.
fn tilde(p: &Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && s.starts_with(&home) => format!("~{}", &s[home.len()..]),
        _ => s,
    }
}

/// The name in the list: just the folder when it sits next to the main worktree
/// (`app-feat`), else the whole path.
fn short_path(p: &Path, main: Option<&Path>) -> String {
    let beside = main.is_some_and(|m| m == p || m.parent() == p.parent());
    match p.file_name() {
        Some(name) if beside => name.to_string_lossy().into_owned(),
        _ => tilde(p),
    }
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
                Ok(repo) => match crate::core::status::read(&repo, true) {
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
        short_path(p, self.main().as_deref())
    }

    fn step(&mut self, n: isize) {
        let last = self.rows.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + n).clamp(0, last) as usize;
        self.detail_scroll = 0;
    }

    /// Returns `Some` to quit, with the folder picked (if any).
    fn key(&mut self, k: KeyEvent) -> Result<Option<Option<PathBuf>>> {
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

        let half = self.detail_area.height / 2;
        let cmd = match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(commands()));
                return Ok(None);
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(commands()));
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
                            crate::fzf::copy(&path.display().to_string());
                        }
                        return Ok(Some(Some(path)));
                    }
                }
            }
            Cmd::Copy => {
                if let Some(r) = self.current() {
                    crate::fzf::copy(&r.wt.path.display().to_string());
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
                    let path = r.wt.path.display().to_string();
                    let (verb, done) = if r.wt.locked.is_some() { ("unlock", "Unlocked") } else { ("lock", "Locked") };
                    let (ok, out) = git(&["worktree".into(), verb.into(), path]);
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
        let local = self.repo.find_reference(format!("refs/heads/{branch}").as_str()).is_ok();
        // git checks out a remote branch of the same name as a new tracking branch.
        let remote = self
            .repo
            .remote_names()
            .iter()
            .any(|r| self.repo.find_reference(format!("refs/remotes/{r}/{branch}").as_str()).is_ok());
        let dir = path.display().to_string();
        let args: Vec<String> = if local || remote {
            vec!["worktree".into(), "add".into(), dir, branch.into()]
        } else {
            vec!["worktree".into(), "add".into(), "-b".into(), branch.into(), dir]
        };
        let (ok, out) = git(&args);
        self.toast = Some(if ok {
            let how = if local {
                "on"
            } else if remote {
                "tracking the remote"
            } else {
                "on new branch"
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
                let mut args: Vec<String> = vec!["worktree".into(), "remove".into()];
                if force {
                    args.push("--force".into());
                }
                args.push(path.display().to_string());
                let (ok, out) = git(&args);
                self.toast = Some(if ok {
                    Toast::new(format!("Removed {}", self.name(&path)), theme::GREEN)
                } else {
                    Toast::new(first_lines(&out), theme::RED)
                });
            }
            Ask::Prune => {
                let before = self.rows.iter().filter(|r| r.wt.prunable.is_some()).count();
                let (ok, out) = git(&["worktree".into(), "prune".into()]);
                self.toast = Some(if ok {
                    Toast::new(format!("Pruned {before}"), theme::GREEN)
                } else {
                    Toast::new(first_lines(&out), theme::RED)
                });
            }
        }
        self.reload(None)
    }

    fn mouse(&mut self, m: crossterm::event::MouseEvent) {
        if self.palette.is_some() || self.confirm.is_some() || self.input.is_some() {
            return;
        }
        let inside = |a: Rect| m.column >= a.x && m.column < a.right() && m.row >= a.y && m.row < a.bottom();
        match m.kind {
            MouseEventKind::ScrollDown if inside(self.detail_area) => {
                self.detail_scroll = self.detail_scroll.saturating_add(3)
            }
            MouseEventKind::ScrollUp if inside(self.detail_area) => {
                self.detail_scroll = self.detail_scroll.saturating_sub(3)
            }
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::ScrollUp => self.step(-1),
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) => {
                let i = self.scroll + (m.row - self.list_area.y) as usize;
                if i < self.rows.len() {
                    self.selected = i;
                    self.detail_scroll = 0;
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
        let rows = (app.rows.len() as u16 + 2).clamp(4, body.height / 2);
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
    f.render_widget(Paragraph::new(widgets::truncate(Line::from(spans), area.width)), area);
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
    let mut s = String::new();
    if u.ahead > 0 {
        s += &format!("↑{}", u.ahead);
    }
    if u.behind > 0 {
        if !s.is_empty() {
            s.push(' ');
        }
        s += &format!("↓{}", u.behind);
    }
    s
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

    let now = crate::core::now();
    let main = app.rows.iter().find(|r| r.wt.main).map(|r| r.wt.path.clone());
    let paths: Vec<String> = app.rows.iter().map(|r| short_path(&r.wt.path, main.as_deref())).collect();
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
        f.render_widget(Paragraph::new(widgets::truncate(Line::from(left), l.width)).style(Style::new().bg(bg)), l);
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right).style(Style::new().bg(bg)), rr);
    }
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let block = widgets::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.detail_area = area;
    let Some(r) = app.current() else { return };
    let lines = build_detail(r, inner.width as usize);

    let max = (lines.len() as u16).saturating_sub(inner.height);
    app.detail_scroll = app.detail_scroll.min(max);
    let shown: Vec<Line> = lines
        .into_iter()
        .skip(app.detail_scroll as usize)
        .take(inner.height as usize)
        .map(|l| widgets::truncate(l, inner.width))
        .collect();
    f.render_widget(Paragraph::new(shown), inner);

    if max > 0 {
        let text = if app.detail_scroll < max {
            format!(" ↓ {} more · J/K ", max - app.detail_scroll)
        } else {
            " ↑ J/K ".to_string()
        };
        let w = text.chars().count() as u16;
        let r = Rect { x: area.right().saturating_sub(w + 1), y: area.bottom().saturating_sub(1), width: w, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(text, theme::faint())), r.intersection(area));
    }
}

fn build_detail(r: &Row, width: usize) -> Vec<Line<'static>> {
    let now = crate::core::now();
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
            widgets::wrap(&r.subject, width.saturating_sub(20))
                .into_iter()
                .take(1)
                .map(|s| Span::styled(s, theme::text())),
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
    let mut pairs: Vec<(&str, &str)> = vec![
        ("enter", go),
        ("n", "new"),
        ("d", "remove"),
        ("ctrl+p", "commands"),
        ("p", "prune"),
        ("l", "lock"),
        ("q", "quit"),
    ];
    let room = area.width as usize;
    while pairs.len() > 2 && widgets::hints(&pairs).width() > room {
        pairs.remove(pairs.len() - 2);
    }
    let right = widgets::hints(&pairs);
    let [_, r] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(right), r);
}

//! `gg wt`: every worktree of the repo, full screen. See what each holds, add, remove,
//! prune, lock, or pick one: enter prints its folder, so `cd "$(gg wt)"` jumps there.
//! Writes go through `git worktree`.

mod draw;
mod keys;

use draw::draw;

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

use crate::fmt::paths::tilde;
use crate::fmt::{now, relative_time};
use crate::git::status::Status;
use crate::git::worktree::{self, Source, Worktree};
use crate::tui::app::{self, Flow, Screen};
use crate::tui::kit::{self, Command, Confirm, Input, InputEvent, Palette, PaletteEvent, Scroll, Toast, footer, list};
use crate::tui::panels::changes;
use crate::tui::{first_lines, theme};

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
            Some("You're inside this worktree. Run glance wt from another one to remove it".into())
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

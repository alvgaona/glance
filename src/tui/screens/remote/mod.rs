//! `gg remote`: every remote of the repo, full screen. See where each points, what was
//! last fetched from it and who tracks it; add, rename, remove, change URLs, fetch.
//! Writes go through `git remote`; fetches run in the background and never prompt.

mod draw;
mod keys;

use draw::draw;

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Padding, Paragraph},
};

use crate::fmt::url;
use crate::fmt::{now, relative_time};
use crate::git::remote::{self, Fetched, Remote};
use crate::process;
use crate::tui::app::{self, Flow, Screen};
use crate::tui::kit::{self, Command, Confirm, Input, InputEvent, Palette, PaletteEvent, Scroll, Toast, footer, list};
use crate::tui::{first_lines, theme};

#[derive(Clone)]
enum Cmd {
    Fetch,
    FetchAll,
    Add,
    Rename,
    SetUrl,
    SetPushUrl,
    Remove,
    SetHead,
    Copy,
    Open,
    Reload,
    Quit,
}

/// What the open text field is for.
enum Typing {
    /// Adding: first the URL…
    AddUrl,
    /// …then its name.
    AddName {
        url: String,
    },
    Rename {
        old: String,
    },
    Url {
        name: String,
    },
    PushUrl {
        name: String,
    },
}

/// What an open dialog will do on yes.
enum Ask {
    Remove(String),
}

/// A git command talking to a remote, on a background thread.
struct Job {
    /// "Fetching origin", for the header.
    what: String,
    kind: JobKind,
    rx: Receiver<(bool, String)>,
}

enum JobKind {
    /// One remote, or all of them.
    Fetch(Option<String>),
    SetHead(String),
}

struct App {
    /// Owned and opened again on reload: `git remote` writes the config, and a
    /// repository only reads it once.
    repo: gix::Repository,
    remotes: Vec<Remote>,
    /// The branch checked out here, to mark the remote it tracks.
    branch: Option<String>,
    selected: usize,
    scroll: usize,
    detail_scroll: Scroll,
    list_area: Rect,
    detail_area: Rect,
    job: Option<Job>,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    confirm: Option<(Confirm, Ask)>,
    input: Option<(Input, Typing)>,
    repo_name: String,
}

pub fn run(repo: gix::Repository) -> Result<()> {
    let mut app = App {
        repo_name: crate::git::repo_name(&repo),
        repo,
        remotes: Vec::new(),
        branch: None,
        selected: 0,
        scroll: 0,
        detail_scroll: Scroll::default(),
        list_area: Rect::default(),
        detail_area: Rect::default(),
        job: None,
        toast: None,
        palette: None,
        confirm: None,
        input: None,
    };
    app.load(None)?;
    // Start on the remote this branch tracks.
    app.selected = app.remotes.iter().position(|r| app.tracked_here(r)).unwrap_or(0);

    app::run(&mut app)
}

impl Screen for App {
    type Out = ();
    // Keep the "fetching…" note moving and pick up the result soon.
    const WAKE: Duration = Duration::from_millis(150);

    fn draw(&mut self, f: &mut Frame) {
        draw(f, self)
    }
    fn key(&mut self, k: KeyEvent) -> Result<Flow<()>> {
        Ok(if self.on_key(k)? { Flow::Done(()) } else { Flow::Stay })
    }
    fn mouse(&mut self, m: MouseEvent) {
        self.on_mouse(m)
    }
    fn tick(&mut self) -> Result<()> {
        self.poll_job()
    }
    fn toast(&mut self) -> &mut Option<Toast> {
        &mut self.toast
    }
}

fn commands() -> Vec<Command<Cmd>> {
    vec![
        Command { label: "Fetch this remote (and prune)", keys: "f", action: Cmd::Fetch },
        Command { label: "Fetch all remotes (and prune)", keys: "F", action: Cmd::FetchAll },
        Command { label: "Add remote", keys: "a", action: Cmd::Add },
        Command { label: "Rename remote", keys: "r", action: Cmd::Rename },
        Command { label: "Set fetch URL", keys: "u", action: Cmd::SetUrl },
        Command { label: "Set push URL", keys: "U", action: Cmd::SetPushUrl },
        Command { label: "Remove remote", keys: "d", action: Cmd::Remove },
        Command { label: "Ask the remote for its default branch", keys: "h", action: Cmd::SetHead },
        Command { label: "Copy URL", keys: "y", action: Cmd::Copy },
        Command { label: "Open in the browser", keys: "o", action: Cmd::Open },
        Command { label: "Reload", keys: "ctrl+r", action: Cmd::Reload },
        Command { label: "Quit", keys: "q", action: Cmd::Quit },
    ]
}

/// A name for a new remote: `origin` first, then the URL's owner, then `upstream`.
fn suggest_name(remotes: &[Remote], url: &str) -> String {
    let taken = |n: &str| remotes.iter().any(|r| r.name == n);
    let owner = url::web(url)
        .and_then(|w| w.trim_start_matches("https://").split('/').nth(1).map(str::to_lowercase))
        .filter(|o| !o.is_empty());
    ["origin".to_string()]
        .into_iter()
        .chain(owner)
        .chain(["upstream".to_string()])
        .find(|n| !taken(n))
        .unwrap_or_default()
}

/// `1 remote`, `2 remotes`, `3 branches`.
fn plural(n: usize, one: &str) -> String {
    let s = match n {
        1 => "",
        _ if one.ends_with("ch") => "es",
        _ => "s",
    };
    format!("{n} {one}{s}")
}

impl App {
    /// Reads the remotes again. `select` puts the cursor on that remote.
    fn load(&mut self, select: Option<&str>) -> Result<()> {
        let keep = select.map(str::to_string).or_else(|| self.current().map(|r| r.name.clone()));
        self.repo = gix::open(self.repo.git_dir())?;
        self.branch = self.repo.head_name()?.map(|n| n.shorten().to_string());
        self.remotes = remote::list(&self.repo)?;
        if let Some(i) = keep.and_then(|k| self.remotes.iter().position(|r| r.name == k)) {
            self.selected = i;
        }
        self.selected = self.selected.min(self.remotes.len().saturating_sub(1));
        Ok(())
    }

    fn current(&self) -> Option<&Remote> {
        self.remotes.get(self.selected)
    }

    fn tracked_here(&self, r: &Remote) -> bool {
        self.branch.as_ref().is_some_and(|b| r.tracking.contains(b))
    }

    fn step(&mut self, n: isize) {
        let last = self.remotes.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + n).clamp(0, last) as usize;
        self.detail_scroll.reset();
    }

    /// Runs a git command that talks to a remote on its own thread; `poll_job` reports it.
    fn start(&mut self, what: String, kind: JobKind, args: Vec<String>) {
        if let Some(j) = &self.job {
            self.toast = Some(Toast::new(format!("Still busy: {}…", j.what.to_lowercase()), theme::YELLOW));
            return;
        }
        let job = remote::no_prompt(&self.repo, args);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
        self.job = Some(Job { what, kind, rx });
    }

    fn fetch(&mut self, name: Option<String>) {
        let what = match &name {
            Some(n) => format!("Fetching {n}"),
            None => "Fetching all remotes".into(),
        };
        let args = remote::fetch_args(name.as_deref());
        self.start(what, JobKind::Fetch(name), args);
    }

    fn poll_job(&mut self) -> Result<()> {
        let Some(job) = &self.job else { return Ok(()) };
        let (ok, out) = match job.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => (false, "git stopped".into()),
        };
        let job = self.job.take().unwrap();
        self.load(None)?;
        self.toast = Some(match (ok, job.kind) {
            (false, JobKind::Fetch(_)) => Toast::new(format!("Fetch failed: {}", remote::errors(&out)), theme::RED),
            (false, JobKind::SetHead(_)) => Toast::new(remote::errors(&out), theme::RED),
            (true, JobKind::Fetch(name)) => {
                let f = Fetched::read(&out);
                let who = name.unwrap_or_else(|| "all remotes".into());
                let mut parts = Vec::new();
                if f.new > 0 {
                    parts.push(format!("{} new", f.new));
                }
                if f.updated > 0 {
                    parts.push(format!("{} updated", f.updated));
                }
                if f.pruned > 0 {
                    parts.push(format!("{} pruned", f.pruned));
                }
                let what =
                    if parts.is_empty() { ": nothing new".to_string() } else { format!(" · {}", parts.join(" · ")) };
                Toast::new(format!("Fetched {who}{what}"), theme::GREEN)
            }
            (true, JobKind::SetHead(name)) => {
                let head = self.remotes.iter().find(|r| r.name == name).and_then(|r| r.head.clone());
                Toast::new(
                    format!("{name}'s default branch is {}", head.unwrap_or_else(|| "unknown".into())),
                    theme::GREEN,
                )
            }
        });
        Ok(())
    }

    /// What the text field gave, for what it was opened for.
    fn typed(&mut self, typing: Typing, text: String) -> Result<()> {
        let fail = |out: &str| Some(Toast::new(first_lines(out), theme::RED));
        match typing {
            Typing::AddUrl => {
                if text.is_empty() {
                    return Ok(());
                }
                let name = suggest_name(&self.remotes, &text);
                let input = Input::new("Name the remote at", &url::short(&text), &name, "Add")
                    .placeholder("name, e.g. upstream")
                    .note("It's fetched right after.");
                self.input = Some((input, Typing::AddName { url: text }));
            }
            Typing::AddName { url } => {
                if text.is_empty() {
                    return Ok(());
                }
                if self.remotes.iter().any(|r| r.name == text) {
                    self.toast = Some(Toast::new(format!("There's a remote called {text} already"), theme::YELLOW));
                    return Ok(());
                }
                let (ok, out) = remote::add(&text, &url);
                if !ok {
                    self.toast = fail(&out);
                    return Ok(());
                }
                self.load(Some(&text))?;
                self.toast = Some(Toast::new(format!("Added {text}"), theme::GREEN));
                self.fetch(Some(text));
            }
            Typing::Rename { old } => {
                if text.is_empty() || text == old {
                    return Ok(());
                }
                let (ok, out) = remote::rename(&old, &text);
                if ok {
                    self.toast = Some(Toast::new(format!("Renamed {old} to {text}"), theme::GREEN));
                    self.load(Some(&text))?;
                } else {
                    self.toast = fail(&out);
                }
            }
            Typing::Url { name } => {
                let was = self.remotes.iter().find(|r| r.name == name).and_then(|r| r.url.clone());
                if text.is_empty() {
                    self.toast = Some(Toast::new("A remote needs a URL; d removes it", theme::YELLOW));
                    return Ok(());
                }
                if Some(&text) == was.as_ref() {
                    return Ok(());
                }
                let (ok, out) = remote::set_url(&name, &text);
                self.toast = if ok {
                    Some(Toast::new(format!("{name} now fetches from {}", url::short(&text)), theme::GREEN))
                } else {
                    fail(&out)
                };
                self.load(None)?;
            }
            Typing::PushUrl { name } => {
                let r = self.remotes.iter().find(|r| r.name == name);
                let was = r.and_then(|r| r.push_url.clone());
                let fetch = r.and_then(|r| r.url.clone());
                // Empty, or the fetch URL again: push where it fetches from.
                let new = Some(text).filter(|t| !t.is_empty() && Some(t) != fetch.as_ref());
                if new == was {
                    return Ok(());
                }
                let (ok, out) = remote::set_push_url(&name, new.as_deref());
                self.toast = match (ok, &new) {
                    (true, Some(u)) => {
                        Some(Toast::new(format!("{name} now pushes to {}", url::short(u)), theme::GREEN))
                    }
                    (true, None) => Some(Toast::new(format!("{name} pushes to its fetch URL again"), theme::GREEN)),
                    (false, _) => fail(&out),
                };
                self.load(None)?;
            }
        }
        Ok(())
    }

    fn ask_remove(&mut self) {
        let Some(r) = self.current() else { return };
        let mut body = match r.branches.len() {
            0 => String::new(),
            1 => format!("Its remote branch {}/{} is forgotten here.\n", r.name, r.branches[0].name),
            n => format!("Its {} ({}/…) are forgotten here.\n", plural(n, "remote branch"), r.name),
        };
        match r.tracking.len() {
            0 => {}
            n => {
                let who =
                    if n == 1 { "This local branch stops".into() } else { format!("These {n} local branches stop") };
                body += &format!("{who} tracking it:\n");
                for b in r.tracking.iter().take(6) {
                    body += &format!("$ {b}\n");
                }
                if n > 6 {
                    body += &format!("and {} more\n", n - 6);
                }
            }
        }
        body += "Nothing changes on the remote itself.";
        let confirm = Confirm {
            title: format!("Remove {}?", r.name),
            body,
            yes: "Remove".into(),
            no: "Cancel".into(),
            danger: true,
        };
        self.confirm = Some((confirm, Ask::Remove(r.name.clone())));
    }

    fn answer(&mut self, ask: Ask) -> Result<()> {
        match ask {
            Ask::Remove(name) => {
                let (ok, out) = remote::remove(&name);
                self.toast = Some(if ok {
                    Toast::new(format!("Removed {name}"), theme::GREEN)
                } else {
                    Toast::new(first_lines(&out), theme::RED)
                });
            }
        }
        self.load(None)
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
                    && i < self.remotes.len()
                {
                    self.selected = i;
                    self.detail_scroll.reset();
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Remote {
        Remote {
            name: name.into(),
            url: None,
            push_url: None,
            head: None,
            branches: Vec::new(),
            tracking: Vec::new(),
            fetched: None,
        }
    }

    #[test]
    fn suggests_a_free_name() {
        assert_eq!(suggest_name(&[], "git@github.com:Me/x.git"), "origin");
        assert_eq!(suggest_name(&[named("origin")], "git@github.com:Me/x.git"), "me");
        assert_eq!(suggest_name(&[named("origin")], "../up"), "upstream");
        assert_eq!(suggest_name(&[named("origin"), named("upstream")], "../up"), "");
    }
}

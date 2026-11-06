//! `glance diff`: changed files on the left, the picked file's diff on the right, drawn
//! by delta when it's installed. What is compared and how the diff is cut switch on
//! the screen; git and delta run on a background thread.

mod draw;
mod keys;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Frame, style::Style, text::Line};

use crate::delta::{self, Look};
use crate::git::compare::{self, Cut, Target};
use crate::git::tree::FileStat;
use crate::tui::app::{self, Flow, Screen};
use crate::tui::kit::{Command, Palette, PaletteEvent, Scroll, Toast, ansi, list};
use crate::tui::panels::diff;
use crate::tui::{first_lines, theme};

#[derive(Clone)]
enum Cmd {
    Compare(Target),
    Split,
    Numbers,
    Whitespace,
    More,
    Less,
    Full,
    Files,
    Edit,
    Copy,
    Search,
    Reload,
    Quit,
}

/// What the screen does outside its own state; [`Screen::apply`] does it.
enum Effect {
    /// Read the changed files again.
    Load,
    Copy(String),
    Edit(String),
    EditOnTerminal(String),
}

/// One rendered diff: a file, cut and drawn one way, at one width.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    path: String,
    cut: Cut,
    look: Look,
    width: u16,
}

/// Context sizes `+` and `-` step through.
const CONTEXT: [u32; 7] = [0, 1, 3, 5, 10, 20, 50];

struct App {
    repo: gix::Repository,
    /// Where git runs; `None` is the folder glance started in.
    dir: Option<PathBuf>,
    repo_name: String,
    target: Target,
    files: Vec<FileStat>,
    /// Indexes into `files` that match the filter.
    visible: Vec<usize>,
    query: String,
    searching: bool,
    selected: usize,
    scroll: usize,
    cut: Cut,
    /// Side by side; `None` decides by the pane's width.
    split: Option<bool>,
    numbers: bool,
    hide_files: bool,
    /// delta is installed; without it the diff is drawn here.
    delta: bool,
    cache: HashMap<Key, Vec<Line<'static>>>,
    /// The render running in the background.
    job: Option<(Key, mpsc::Receiver<Vec<Line<'static>>>)>,
    /// The diff pane's width at the last draw.
    width: u16,
    detail_scroll: Scroll,
    detail_for: Option<String>,
    detail_area: ratatui::layout::Rect,
    list_area: ratatui::layout::Rect,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
}

pub fn run(args: &[String]) -> Result<()> {
    let repo = crate::git::open()?;
    let (split, numbers) = (delta::setting("side-by-side"), delta::setting("line-numbers"));
    let mut app = start(repo, None, Target::from_args(args), delta::available())?;
    app.split = split;
    app.numbers = numbers.unwrap_or(true);
    app::run(&mut app)
}

/// The screen as it opens, before the first draw.
fn start(repo: gix::Repository, dir: Option<PathBuf>, target: Target, delta: bool) -> Result<App> {
    let mut app = App {
        repo_name: crate::git::repo_name(&repo),
        repo,
        dir,
        target,
        files: Vec::new(),
        visible: Vec::new(),
        query: String::new(),
        searching: false,
        selected: 0,
        scroll: 0,
        cut: Cut::default(),
        split: None,
        numbers: true,
        hide_files: false,
        delta,
        cache: HashMap::new(),
        job: None,
        width: 0,
        detail_scroll: Scroll::default(),
        detail_for: None,
        detail_area: Default::default(),
        list_area: Default::default(),
        toast: None,
        palette: None,
    };
    app.reload()?;
    Ok(app)
}

impl Screen for App {
    type Out = ();
    type Effect = Effect;

    fn draw(&mut self, f: &mut Frame) {
        draw::draw(f, self)
    }
    fn key(&mut self, k: KeyEvent) -> Flow<(), Effect> {
        self.on_key(k)
    }
    fn mouse(&mut self, m: MouseEvent) {
        self.on_mouse(m)
    }
    fn apply(&mut self, e: Effect) -> Result<Flow<(), Effect>> {
        match e {
            Effect::Load => self.reload()?,
            Effect::Copy(path) => {
                crate::clipboard::copy(&path);
                self.toast = Some(Toast::new(format!("Copied {path}"), theme::ACCENT));
            }
            Effect::Edit(path) => {
                if self.repo.workdir().is_some_and(|d| d.join(&path).is_file()) {
                    return Ok(Flow::Run(Effect::EditOnTerminal(path)));
                }
                self.toast = Some(Toast::new("It's not in the working tree", theme::YELLOW));
            }
            Effect::EditOnTerminal(path) => {
                if let Err(out) = crate::git::run::edit(&self.repo, &path) {
                    let why = if out.is_empty() { "The editor failed".into() } else { first_lines(&out) };
                    self.toast = Some(Toast::new(why, theme::RED));
                }
                self.reload()?;
            }
        }
        Ok(Flow::Stay)
    }
    fn in_terminal(e: &Effect) -> bool {
        matches!(e, Effect::EditOnTerminal(_))
    }
    fn tick(&mut self) -> Result<()> {
        self.poll();
        // Zero until the first draw has measured the pane.
        if self.width == 0 {
            return Ok(());
        }
        let (Some(file), Some(key)) = (self.current().cloned(), self.key_for(self.width)) else { return Ok(()) };
        if self.cache.contains_key(&key) || self.job.as_ref().is_some_and(|(k, _)| *k == key) {
            return Ok(());
        }
        // A newer pick replaces the job; the old thread's answer is dropped.
        let (tx, rx) = mpsc::channel();
        let (dir, target, delta, k) = (self.dir.clone(), self.target.clone(), self.delta, key.clone());
        std::thread::spawn(move || {
            let _ = tx.send(render(dir, &target, &file, &k, delta));
        });
        self.job = Some((key, rx));
        // Most diffs take a few milliseconds: wait a little rather than flash "loading".
        if let Some((_, rx)) = &self.job
            && let Ok(lines) = rx.recv_timeout(Duration::from_millis(60))
        {
            let (key, _) = self.job.take().unwrap();
            self.cache.insert(key, lines);
        }
        Ok(())
    }
    fn toast(&mut self) -> &mut Option<Toast> {
        &mut self.toast
    }
    fn typing(&self) -> bool {
        self.searching
    }
}

/// One file's diff as lines: delta's, or drawn here without it.
fn render(dir: Option<PathBuf>, target: &Target, f: &FileStat, key: &Key, delta: bool) -> Vec<Line<'static>> {
    let red = Style::new().fg(theme::RED);
    let patch = match compare::patch(dir.as_deref(), target, f, key.cut) {
        Ok(p) => p,
        Err(e) => return vec![Line::styled(format!("git diff failed: {}", first_lines(&e.to_string())), red)],
    };
    let mut out = Vec::new();
    if delta {
        match delta::render(&patch, key.width, key.look) {
            Ok(text) => return ansi::lines(&text),
            Err(e) => out.push(Line::styled(format!("delta failed: {}", first_lines(&e)), red)),
        }
    }
    let before = out.len();
    out.extend(compare::lines(&patch).into_iter().map(diff::line));
    if out.len() == before {
        out.push(Line::styled("Binary file, or only its mode changed", theme::faint()));
    }
    out
}

fn commands() -> Vec<Command<Cmd>> {
    vec![
        Command { label: "Compare: unstaged changes", keys: "1", action: Cmd::Compare(Target::Unstaged) },
        Command { label: "Compare: staged changes", keys: "2", action: Cmd::Compare(Target::Staged) },
        Command { label: "Compare: everything since HEAD", keys: "3", action: Cmd::Compare(Target::Head) },
        Command { label: "Side by side: on or off", keys: "s", action: Cmd::Split },
        Command { label: "Line numbers: on or off", keys: "l", action: Cmd::Numbers },
        Command { label: "Ignore whitespace: on or off", keys: "w", action: Cmd::Whitespace },
        Command { label: "More context", keys: "+", action: Cmd::More },
        Command { label: "Less context", keys: "-", action: Cmd::Less },
        Command { label: "Whole file: on or off", keys: "f", action: Cmd::Full },
        Command { label: "File list: show or hide", keys: "z", action: Cmd::Files },
        Command { label: "Edit file", keys: "e", action: Cmd::Edit },
        Command { label: "Copy file path", keys: "y", action: Cmd::Copy },
        Command { label: "Filter files", keys: "/", action: Cmd::Search },
        Command { label: "Reload", keys: "ctrl+r", action: Cmd::Reload },
        Command { label: "Quit", keys: "q", action: Cmd::Quit },
    ]
}

impl App {
    /// Reads the changed files again, keeping the cursor on the same file.
    fn reload(&mut self) -> Result<()> {
        let keep = self.current().map(|f| f.path.clone());
        self.cache.clear();
        self.job = None;
        self.files = compare::files(self.dir.as_deref(), &self.target, self.cut.whitespace).unwrap_or_else(|e| {
            self.toast = Some(Toast::new(first_lines(&e.to_string()), theme::RED));
            Vec::new()
        });
        self.filter();
        if let Some(p) = keep.and_then(|k| self.visible.iter().position(|&i| self.files[i].path == k)) {
            self.selected = p;
        }
        Ok(())
    }

    fn filter(&mut self) {
        let q = self.query.to_lowercase();
        self.visible = (0..self.files.len()).filter(|&i| self.files[i].path.to_lowercase().contains(&q)).collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }

    fn current(&self) -> Option<&FileStat> {
        self.visible.get(self.selected).map(|&i| &self.files[i])
    }

    fn step(&mut self, n: isize) {
        let last = self.visible.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + n).clamp(0, last) as usize;
    }

    /// Side by side at this pane width.
    fn split_at(&self, width: u16) -> bool {
        self.split.unwrap_or(width >= 120)
    }

    /// The cache key for the current file at `width`.
    fn key_for(&self, width: u16) -> Option<Key> {
        let look = Look { split: self.split_at(width), numbers: self.numbers };
        Some(Key { path: self.current()?.path.clone(), cut: self.cut, look, width })
    }

    /// Takes a finished render, if there is one.
    fn poll(&mut self) {
        let Some((_, rx)) = &self.job else { return };
        match rx.try_recv() {
            Ok(lines) => {
                let (key, _) = self.job.take().unwrap();
                self.cache.insert(key, lines);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.job = None,
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if self.palette.is_some() {
            return;
        }
        let inside = |a| list::inside(a, &m);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) => {
                if let Some(i) = list::row_at(self.list_area, self.scroll, &m)
                    && i < self.visible.len()
                {
                    self.selected = i;
                }
            }
            MouseEventKind::ScrollDown if inside(self.detail_area) => self.detail_scroll.wheel(true),
            MouseEventKind::ScrollUp if inside(self.detail_area) => self.detail_scroll.wheel(false),
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::ScrollUp => self.step(-1),
            _ => {}
        }
    }
}

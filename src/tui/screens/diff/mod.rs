//! `glance diff`: changed files on the left, the picked file's diff on the right, drawn
//! by delta when it's installed. What is compared and how the diff is cut switch on
//! the screen; git and delta run on a background thread.

mod draw;
mod keys;
mod saved;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Frame, style::Style, text::Line};

use crate::delta;
use crate::git::compare::{self, Cut, Target};
use crate::git::diff::PatchLine;
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
    /// To the next hunk, or the one before.
    Hunk(bool),
    Files,
    Side,
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
    /// A file to open in the editor, at this line.
    Edit(String, Option<u32>),
    EditOnTerminal(String, Option<u32>),
}

/// What the screen controls in delta's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Look {
    split: bool,
    numbers: bool,
}

/// One file's diff, drawn.
struct Rendered {
    lines: Vec<Line<'static>>,
    hunks: Vec<HunkAt>,
}

/// Where a hunk is.
#[derive(Debug, Clone)]
struct HunkAt {
    /// The line in the drawn diff to scroll to for it.
    at: usize,
    /// Its first line in the new file.
    line: u32,
    /// What git says it's in: a function, a section.
    context: String,
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
    /// Side by side, as set with `s`; `None` decides by the pane's width.
    split: Option<bool>,
    /// `delta.side-by-side` from git config.
    prefer_split: Option<bool>,
    numbers: bool,
    hide_files: bool,
    /// The sidebar, on wide screens.
    side: bool,
    /// Whether the last draw had room for it: only then is history read.
    side_open: bool,
    /// Commits that changed each file, read in the background.
    history: HashMap<String, Vec<compare::Touch>>,
    history_job: Option<(String, mpsc::Receiver<Vec<compare::Touch>>)>,
    /// The commit `glance show` is on.
    details: Option<crate::git::commit::Details>,
    /// The sidebar's hunk rows, for clicks.
    outline_area: ratatui::layout::Rect,
    /// delta is installed; without it the diff is drawn here.
    delta: bool,
    /// New files git doesn't diff, counted for the header.
    untracked: usize,
    cache: HashMap<Key, Rendered>,
    /// The render running in the background, and whether `tick` waited for it yet.
    job: Option<(Key, mpsc::Receiver<Rendered>, bool)>,
    /// The diff pane's width at the last draw.
    width: u16,
    /// The render drawn last: shown while the next one for the same file runs.
    shown: Option<Key>,
    detail_scroll: Scroll,
    detail_for: Option<String>,
    detail_area: ratatui::layout::Rect,
    list_area: ratatui::layout::Rect,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
}

pub fn run(repo: gix::Repository, target: Target) -> Result<()> {
    let mut app = start(repo, None, target, delta::available())?;
    app.prefer_split = delta::setting("side-by-side");
    app.numbers = delta::setting("line-numbers").unwrap_or(true);
    // What was on last time wins over delta's settings.
    saved::load(&mut app);
    app.side = crate::tui::state::sidebar();
    if app.cut.whitespace {
        app.reload()?;
    }
    app::run(&mut app)?;
    saved::save(&app);
    crate::tui::state::set_sidebar(app.side);
    Ok(())
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
        prefer_split: None,
        numbers: true,
        hide_files: false,
        side: true,
        side_open: false,
        history: HashMap::new(),
        history_job: None,
        details: None,
        outline_area: Default::default(),
        delta,
        untracked: 0,
        cache: HashMap::new(),
        job: None,
        width: 0,
        shown: None,
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
            Effect::Edit(path, line) => {
                if self.repo.workdir().is_some_and(|d| d.join(&path).is_file()) {
                    return Ok(Flow::Run(Effect::EditOnTerminal(path, line)));
                }
                self.toast = Some(Toast::new("It's not in the working tree", theme::YELLOW));
            }
            Effect::EditOnTerminal(path, line) => {
                if let Err(out) = crate::git::run::edit(&self.repo, &path, line) {
                    let why = if out.is_empty() { "The editor failed".into() } else { first_lines(&out) };
                    self.toast = Some(Toast::new(why, theme::RED));
                }
                self.reload()?;
            }
        }
        Ok(Flow::Stay)
    }
    fn in_terminal(e: &Effect) -> bool {
        matches!(e, Effect::EditOnTerminal(..))
    }
    fn tick(&mut self) -> Result<()> {
        self.poll();
        self.load_history();
        // Zero until the first draw has measured the pane.
        if self.width == 0 {
            return Ok(());
        }
        // The picked file first, then the next one, so `j` finds it ready.
        let todo = [self.selected, self.selected + 1]
            .into_iter()
            .filter_map(|i| Some((i, self.key_at(i, self.width)?)))
            .find(|(_, k)| !self.cache.contains_key(k));
        let Some((i, key)) = todo else { return Ok(()) };
        if !self.job.as_ref().is_some_and(|(k, ..)| *k == key) {
            // A newer pick replaces the job; the old thread's answer is dropped.
            let (tx, rx) = mpsc::channel();
            let (dir, target, delta, k) = (self.dir.clone(), self.target.clone(), self.delta, key.clone());
            let file = self.files[self.visible[i]].clone();
            std::thread::spawn(move || {
                let _ = tx.send(render(dir, &target, &file, &k, delta));
            });
            self.job = Some((key, rx, false));
        }
        // A render takes tens of milliseconds: wait for the one on screen, once, so it
        // shows the moment it's done rather than at the next wake.
        if i == self.selected
            && let Some((_, rx, waited @ false)) = &mut self.job
        {
            *waited = true;
            if let Ok(r) = rx.recv_timeout(Duration::from_millis(250)) {
                let (key, ..) = self.job.take().unwrap();
                self.store(key, r);
                // Now the next file.
                return self.tick();
            }
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

/// One file's diff: delta's, or drawn here without it.
fn render(dir: Option<PathBuf>, target: &Target, f: &FileStat, key: &Key, delta: bool) -> Rendered {
    let red = Style::new().fg(theme::RED);
    let patch = match compare::patch(dir.as_deref(), target, f, key.cut) {
        Ok(p) => p,
        Err(e) => {
            let lines = vec![Line::styled(format!("git diff failed: {}", first_lines(&e.to_string())), red)];
            return Rendered { lines, hunks: Vec::new() };
        }
    };
    let parsed = compare::lines(&patch);
    let headers = compare::headers(&patch);
    let place = |at: Vec<usize>| -> Vec<HunkAt> {
        at.into_iter()
            .zip(&headers)
            .map(|(at, (line, context))| HunkAt { at, line: *line, context: context.clone() })
            .collect()
    };
    let mut lines = Vec::new();
    if delta {
        let set = [
            ("side-by-side", if key.look.split { "true" } else { "false" }),
            ("line-numbers", if key.look.numbers { "true" } else { "false" }),
            // The pane's title already names the file.
            ("file-style", "omit"),
            ("file-decoration-style", "none"),
        ];
        match delta::render(&patch, key.width, &set) {
            // The line above the header: the top of its box.
            Ok(d) => {
                let hunks = place(d.hunks.into_iter().map(|i| i.saturating_sub(1)).collect());
                return Rendered { lines: ansi::lines(&d.text), hunks };
            }
            Err(e) => lines.push(Line::styled(format!("delta failed: {}", first_lines(&e)), red)),
        }
    }
    let at = lines.len();
    let hunks = place(
        parsed.iter().enumerate().filter(|(_, l)| matches!(l, PatchLine::Hunk { .. })).map(|(i, _)| at + i).collect(),
    );
    lines.extend(parsed.into_iter().map(diff::line));
    if lines.len() == at {
        lines.push(Line::styled("Binary file, or only its mode changed", theme::faint()));
    }
    Rendered { lines, hunks }
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
        Command { label: "Next hunk", keys: "n", action: Cmd::Hunk(true) },
        Command { label: "Previous hunk", keys: "N", action: Cmd::Hunk(false) },
        Command { label: "Whole file: on or off", keys: "f", action: Cmd::Full },
        Command { label: "File list: show or hide", keys: "z", action: Cmd::Files },
        Command { label: "Sidebar: show or hide (wide screens)", keys: "i", action: Cmd::Side },
        Command { label: "Edit file at this hunk", keys: "e", action: Cmd::Edit },
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
        self.history.clear();
        self.history_job = None;
        self.details = match &self.target {
            Target::Commit { id, .. } => crate::git::commit::read(&self.repo, id).ok(),
            _ => None,
        };
        let worktree = matches!(self.target, Target::Unstaged | Target::Head);
        self.untracked = if worktree { compare::untracked(self.dir.as_deref()) } else { 0 };
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

    /// Side by side at this pane width. With delta set to side by side, from 100
    /// columns; narrower, its lines would wrap.
    fn split_at(&self, width: u16) -> bool {
        self.split.unwrap_or(match self.prefer_split {
            Some(false) => false,
            Some(true) => width >= 100,
            None => width >= 120,
        })
    }

    /// The cache key for the `i`th visible file at `width`.
    fn key_at(&self, i: usize, width: u16) -> Option<Key> {
        let look = Look { split: self.split_at(width), numbers: self.numbers };
        Some(Key { path: self.files[*self.visible.get(i)?].path.clone(), cut: self.cut, look, width })
    }

    fn key_for(&self, width: u16) -> Option<Key> {
        self.key_at(self.selected, width)
    }

    /// Keeps a render. Each option and width makes its own, so past 64 only the one on
    /// screen stays.
    fn store(&mut self, key: Key, r: Rendered) {
        if self.cache.len() >= 64 {
            let shown = self.shown.clone();
            self.cache.retain(|k, _| Some(k) == shown.as_ref());
        }
        self.cache.insert(key, r);
    }

    /// The render on screen.
    fn on_screen(&self) -> Option<&Rendered> {
        self.cache.get(self.shown.as_ref()?)
    }

    /// Scrolls to the next hunk below the top of the pane, or the one above.
    fn jump(&mut self, down: bool) {
        let Some(r) = self.on_screen() else { return };
        let top = self.detail_scroll.top() as usize;
        let mut at = r.hunks.iter().map(|h| h.at);
        let to = if down { at.find(|&i| i > top) } else { at.rfind(|&i| i < top) };
        if let Some(i) = to {
            self.detail_scroll.to(i);
        }
    }

    /// The hunk at the top of the pane: the last one starting there or above.
    fn hunk_here(&self) -> Option<usize> {
        let r = self.on_screen()?;
        let top = self.detail_scroll.top() as usize;
        let n = r.hunks.iter().take_while(|h| h.at <= top).count();
        (!r.hunks.is_empty()).then(|| n.saturating_sub(1))
    }

    /// The new file's line at the top of the pane's hunk, for the editor.
    fn line_here(&self) -> Option<u32> {
        Some(self.on_screen()?.hunks.get(self.hunk_here()?)?.line)
    }

    /// Starts reading the current file's commits, when the sidebar shows and they
    /// aren't read yet; takes them when they're in.
    fn load_history(&mut self) {
        if let Some((path, rx)) = &self.history_job
            && let Ok(h) = rx.try_recv()
        {
            self.history.insert(path.clone(), h);
            self.history_job = None;
        }
        let Some(path) = self.current().map(|f| f.path.clone()) else { return };
        let running = self.history_job.as_ref().is_some_and(|(p, _)| *p == path);
        if !self.side_open || running || self.history.contains_key(&path) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let dir = self.dir.clone();
        let from = match &self.target {
            Target::Commit { id, .. } => Some(id.clone()),
            _ => None,
        };
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(compare::history(dir.as_deref(), from.as_deref(), &p, 5));
        });
        self.history_job = Some((path, rx));
    }

    /// Takes a finished render, if there is one.
    fn poll(&mut self) {
        let Some((_, rx, _)) = &self.job else { return };
        match rx.try_recv() {
            Ok(r) => {
                let (key, ..) = self.job.take().unwrap();
                self.store(key, r);
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
            MouseEventKind::Down(MouseButton::Left) if inside(self.outline_area) => {
                let row = (m.row - self.outline_area.y) as usize;
                if let Some(at) = self.on_screen().and_then(|r| r.hunks.get(row)).map(|h| h.at) {
                    self.detail_scroll.to(at);
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

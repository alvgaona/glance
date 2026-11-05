//! `gg stash`: your changes on the left, stashes on the right. Stash what you pick, or
//! apply, pop, drop and branch off a stash. Writes go through git (`git stash …`).

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use gix::ObjectId;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};

use super::stash_new::{Mode, Picked, Picker};
use super::widgets::{self, Command, Confirm, Input, InputEvent, Palette, PaletteEvent, Toast};
use super::{diffview, enter, first_lines, git, leave, theme};
use crate::fmt::{now, relative_time};
use crate::git::stash::{self, Stash};

#[derive(Clone)]
enum Cmd {
    Apply,
    Pop,
    Drop,
    Branch,
    /// Go to the changes pane in this mode.
    New(Mode),
    /// Changes pane: include untracked files or not.
    PickUntracked,
    /// Changes pane: tick every file, or none.
    PickAll,
    Mark,
    ClearMarks,
    Copy,
    Search,
    /// Switch between the changes and the stashes.
    Focus,
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Changes,
    Stashes,
}

/// What an open text field is for.
enum Ask {
    /// `git stash branch <name> <stash>`.
    Branch(Stash),
    /// `git stash push …` with what the picker chose.
    New(Picked),
}

struct App<'r> {
    repo: &'r gix::Repository,
    list: Vec<Stash>,
    /// Indexes into `list` that match the search.
    visible: Vec<usize>,
    selected: usize,
    scroll: usize,
    /// Picked with space, by commit id (indexes shift when one is dropped).
    marked: BTreeSet<ObjectId>,
    query: String,
    searching: bool,
    focus: Pane,
    /// Details by (stash, panel width).
    cache: HashMap<(ObjectId, u16), Rc<Vec<Line<'static>>>>,
    detail_scroll: u16,
    detail_for: Option<ObjectId>,
    detail_area: Rect,
    list_area: Rect,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    confirm: Option<(Confirm, Vec<Stash>)>,
    input: Option<(Input, Ask)>,
    /// The working tree, and what to stash from it.
    changes: Picker,
    changes_area: Rect,
    repo_name: String,
    branch: Option<String>,
    /// Changed + untracked files in the working tree right now.
    dirty: usize,
}

pub fn run(repo: &gix::Repository) -> Result<()> {
    let mut app = App {
        repo,
        list: Vec::new(),
        visible: Vec::new(),
        selected: 0,
        scroll: 0,
        marked: BTreeSet::new(),
        query: String::new(),
        searching: false,
        focus: Pane::Stashes,
        cache: HashMap::new(),
        detail_scroll: 0,
        detail_for: None,
        detail_area: Rect::default(),
        list_area: Rect::default(),
        toast: None,
        palette: None,
        confirm: None,
        input: None,
        changes: Picker::new(&crate::git::status::read(repo, true)?),
        changes_area: Rect::default(),
        repo_name: crate::git::repo_name(repo),
        branch: None,
        dirty: 0,
    };
    app.reload(None)?;
    // Start where there's something to do.
    if app.list.is_empty() && !app.changes.is_empty() {
        app.focus = Pane::Changes;
    }

    let mut terminal = enter();
    let result = (|| -> Result<()> {
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
                    if app.key(k)? {
                        return Ok(());
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
        Command { label: "Apply stash (keep it)", keys: "enter", action: Cmd::Apply },
        Command { label: "Pop stash (apply and drop)", keys: "p", action: Cmd::Pop },
        Command { label: "Drop stash", keys: "d", action: Cmd::Drop },
        Command { label: "New branch from stash", keys: "b", action: Cmd::Branch },
        Command { label: "Stash changes: pick files", keys: "n", action: Cmd::New(Mode::Everything) },
        Command { label: "Stash staged changes only", keys: "s", action: Cmd::New(Mode::Staged) },
        Command { label: "Stash unstaged changes only (keep staged)", keys: "u", action: Cmd::New(Mode::Unstaged) },
        Command { label: "Include untracked files: on or off", keys: "t", action: Cmd::PickUntracked },
        Command { label: "Tick all files, or none", keys: "a", action: Cmd::PickAll },
        Command { label: "Select or unselect stash", keys: "space", action: Cmd::Mark },
        Command { label: "Clear selection", keys: "esc", action: Cmd::ClearMarks },
        Command { label: "Copy stash id", keys: "y", action: Cmd::Copy },
        Command { label: "Search stashes", keys: "/", action: Cmd::Search },
        Command { label: "Switch pane", keys: "tab", action: Cmd::Focus },
        Command { label: "Quit", keys: "q", action: Cmd::Quit },
    ]
}

impl App<'_> {
    /// Reads stashes and the working tree again. `select` puts the cursor on that stash index.
    fn reload(&mut self, select: Option<usize>) -> Result<()> {
        let keep = self.current().map(|s| s.oid);
        self.list = stash::list(self.repo)?;
        self.cache.clear();
        let ids: BTreeSet<ObjectId> = self.list.iter().map(|s| s.oid).collect();
        self.marked.retain(|id| ids.contains(id));
        let st = crate::git::status::read(self.repo, true)?;
        self.changes.update(&st);
        self.branch = st.branch;
        // A file both staged and changed again counts once.
        let paths: BTreeSet<&str> = st
            .staged
            .iter()
            .chain(&st.unstaged)
            .map(|f| f.path.as_str())
            .chain(st.untracked.iter().chain(&st.conflicted).map(String::as_str))
            .collect();
        self.dirty = paths.len();
        self.filter();
        let pos = match select {
            Some(index) => self.visible.iter().position(|&i| self.list[i].index == index),
            None => keep.and_then(|id| self.visible.iter().position(|&i| self.list[i].oid == id)),
        };
        if let Some(p) = pos {
            self.selected = p;
        }
        Ok(())
    }

    fn filter(&mut self) {
        let q = self.query.to_lowercase();
        self.visible = (0..self.list.len())
            .filter(|&i| {
                let s = &self.list[i];
                q.is_empty()
                    || s.message.to_lowercase().contains(&q)
                    || s.branch.as_deref().is_some_and(|b| b.to_lowercase().contains(&q))
            })
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }

    fn current(&self) -> Option<&Stash> {
        self.visible.get(self.selected).map(|&i| &self.list[i])
    }

    /// Marked stashes, or the one under the cursor when nothing is marked.
    fn targets(&self) -> Vec<Stash> {
        if self.marked.is_empty() {
            self.current().cloned().into_iter().collect()
        } else {
            self.list.iter().filter(|s| self.marked.contains(&s.oid)).cloned().collect()
        }
    }

    fn step(&mut self, n: isize) {
        let last = self.visible.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + n).clamp(0, last) as usize;
    }

    /// Returns true to quit.
    fn key(&mut self, k: KeyEvent) -> Result<bool> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if let Some((input, _)) = &mut self.input {
            match input.key(k) {
                Some(InputEvent::Cancel) => self.input = None,
                Some(InputEvent::Done(text)) => {
                    let (_, ask) = self.input.take().unwrap();
                    self.answer(ask, text)?;
                }
                None => {}
            }
            return Ok(false);
        }
        if ctrl && k.code == KeyCode::Char('c') {
            return Ok(true);
        }
        if self.confirm.is_some() {
            match k.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let (_, list) = self.confirm.take().unwrap();
                    self.drop_stashes(list)?;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => self.confirm = None,
                _ => {}
            }
            return Ok(false);
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
            return Ok(false);
        }
        if self.searching {
            match k.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                    self.filter();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.filter();
                }
                KeyCode::Up => self.step(-1),
                KeyCode::Down => self.step(1),
                KeyCode::Char(c) if !ctrl => {
                    self.query.push(c);
                    self.selected = 0;
                    self.filter();
                }
                _ => {}
            }
            return Ok(false);
        }

        // Keys for both panes.
        match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(commands()));
                return Ok(false);
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(commands()));
                return Ok(false);
            }
            KeyCode::Tab | KeyCode::BackTab => return self.run(Cmd::Focus),
            _ => {}
        }

        if self.focus == Pane::Changes {
            if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                return Ok(true);
            }
            if let Some(picked) = self.changes.key(k) {
                let input =
                    Input::new("Stash", &picked.describe(), "", "Stash").spaces().placeholder("message (optional)");
                self.input = Some((input, Ask::New(picked)));
            }
            return Ok(false);
        }

        let half = self.detail_area.height / 2;
        let cmd = match k.code {
            // Details scrolling, as in `gg log`.
            KeyCode::Char('d') if ctrl => {
                self.detail_scroll = self.detail_scroll.saturating_add(half);
                return Ok(false);
            }
            KeyCode::Char('u') if ctrl => {
                self.detail_scroll = self.detail_scroll.saturating_sub(half);
                return Ok(false);
            }
            KeyCode::Down | KeyCode::Char('j') if shift => {
                self.detail_scroll = self.detail_scroll.saturating_add(3);
                return Ok(false);
            }
            KeyCode::Up | KeyCode::Char('k') if shift => {
                self.detail_scroll = self.detail_scroll.saturating_sub(3);
                return Ok(false);
            }
            KeyCode::Char('J') => {
                self.detail_scroll = self.detail_scroll.saturating_add(3);
                return Ok(false);
            }
            KeyCode::Char('K') => {
                self.detail_scroll = self.detail_scroll.saturating_sub(3);
                return Ok(false);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(1);
                return Ok(false);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(-1);
                return Ok(false);
            }
            KeyCode::PageDown => {
                self.step(15);
                return Ok(false);
            }
            KeyCode::PageUp => {
                self.step(-15);
                return Ok(false);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
                return Ok(false);
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.visible.len().saturating_sub(1);
                return Ok(false);
            }
            KeyCode::Enter | KeyCode::Char('a') => Cmd::Apply,
            KeyCode::Char('p') => Cmd::Pop,
            KeyCode::Char('d') => Cmd::Drop,
            KeyCode::Char('b') => Cmd::Branch,
            KeyCode::Char('n') => Cmd::New(Mode::Everything),
            KeyCode::Char(' ') => Cmd::Mark,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('/') => Cmd::Search,
            KeyCode::Esc if !self.marked.is_empty() => Cmd::ClearMarks,
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.filter();
                return Ok(false);
            }
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Ok(false),
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Result<bool> {
        match cmd {
            Cmd::Quit => return Ok(true),
            Cmd::Search => {
                self.focus = Pane::Stashes;
                self.searching = true;
            }
            Cmd::Focus => {
                self.focus = match self.focus {
                    Pane::Changes => Pane::Stashes,
                    Pane::Stashes => Pane::Changes,
                }
            }
            Cmd::ClearMarks => self.marked.clear(),
            Cmd::Mark => {
                if let Some(s) = self.current() {
                    let id = s.oid;
                    if !self.marked.remove(&id) {
                        self.marked.insert(id);
                    }
                    self.step(1);
                }
            }
            Cmd::Copy => {
                if let Some(s) = self.current() {
                    let id = s.oid.to_string();
                    crate::clipboard::copy(&id);
                    self.toast = Some(Toast::new(format!("Copied {}", &id[..7.min(id.len())]), theme::ACCENT));
                }
            }
            Cmd::Apply => self.apply(false)?,
            Cmd::Pop => self.apply(true)?,
            Cmd::Drop => self.ask_drop(),
            Cmd::Branch => {
                if let Some(s) = self.current().cloned() {
                    let input = Input::new("New branch from", &s.name(), &suggest_branch(&s), "Create branch").note(
                        format!("Checks out a new branch at {}, applies the stash there\nand drops it.", s.base_id),
                    );
                    self.input = Some((input, Ask::Branch(s)));
                }
            }
            Cmd::New(mode) => {
                self.focus = Pane::Changes;
                self.changes.set_mode(mode);
            }
            Cmd::PickUntracked => {
                self.focus = Pane::Changes;
                self.changes.toggle_untracked();
            }
            Cmd::PickAll => {
                self.focus = Pane::Changes;
                self.changes.toggle_all();
            }
        }
        Ok(false)
    }

    /// `git stash apply` or `git stash pop` on the current stash.
    fn apply(&mut self, pop: bool) -> Result<()> {
        let Some(s) = self.current().cloned() else { return Ok(()) };
        let verb = if pop { "pop" } else { "apply" };
        let (ok, out) = git(&["stash".into(), verb.into(), s.name()]);
        self.toast = Some(if ok {
            let done = if pop { "Popped" } else { "Applied" };
            Toast::new(format!("{done} {}: {}", s.name(), s.title()), theme::GREEN)
        } else if out.contains("CONFLICT") {
            // git keeps the stash when a pop conflicts.
            let n = out.lines().filter(|l| l.contains("CONFLICT")).count();
            Toast::new(
                format!(
                    "Applied with conflicts in {n} file{}. The stash is kept; fix them, then drop it",
                    if n == 1 { "" } else { "s" }
                ),
                theme::RED,
            )
        } else {
            Toast::new(first_lines(&out), theme::RED)
        });
        self.reload(None)
    }

    fn ask_drop(&mut self) {
        let list = self.targets();
        if list.is_empty() {
            return;
        }
        let (title, body) = if let [s] = list.as_slice() {
            (
                format!("Drop {}?", s.name()),
                format!(
                    "{}\nTo bring it back later (until git cleans up):\n$ git stash store -m \"{}\" {}",
                    s.title(),
                    s.message.replace('"', "'"),
                    s.oid
                ),
            )
        } else {
            let mut body: String =
                list.iter().take(10).map(|s| format!("$ {}  {}  {}\n", s.name(), s.id, s.title())).collect();
            if list.len() > 10 {
                body += &format!("…and {} more\n", list.len() - 10);
            }
            body += "`git stash store <id>` brings one back (until git cleans up).";
            (format!("Drop {} stashes?", list.len()), body)
        };
        self.confirm = Some((Confirm { title, body, yes: "Drop".into(), no: "Cancel".into(), danger: true }, list));
    }

    /// Drops newest index last, so the indexes of the others don't shift under us.
    fn drop_stashes(&mut self, mut list: Vec<Stash>) -> Result<()> {
        list.sort_by_key(|s| std::cmp::Reverse(s.index));
        let mut failed = Vec::new();
        for s in &list {
            let (ok, out) = git(&["stash".into(), "drop".into(), s.name()]);
            if !ok {
                failed.push(out);
            }
        }
        self.marked.clear();
        self.toast = Some(match (failed.first(), list.as_slice()) {
            (Some(err), _) => Toast::new(format!("{} failed: {}", failed.len(), first_lines(err)), theme::RED),
            (None, [s]) => Toast::new(format!("Dropped {}: {}", s.name(), s.title()), theme::GREEN),
            (None, _) => Toast::new(format!("Dropped {} stashes", list.len()), theme::GREEN),
        });
        self.reload(None)
    }

    fn answer(&mut self, ask: Ask, text: String) -> Result<()> {
        match ask {
            Ask::Branch(s) => {
                if text.is_empty() {
                    return Ok(());
                }
                let (ok, out) = git(&["stash".into(), "branch".into(), text.clone(), s.name()]);
                self.toast = Some(if ok {
                    Toast::new(format!("On new branch {text}, with {} applied", s.name()), theme::GREEN)
                } else {
                    Toast::new(first_lines(&out), theme::RED)
                });
                self.reload(None)
            }
            Ask::New(picked) => {
                let (ok, out) = git(&picked.args(&text));
                if ok {
                    self.changes.reset_ticks();
                }
                self.toast = Some(if ok {
                    Toast::new(format!("Stashed {} as stash@{{0}}", picked.describe()), theme::GREEN)
                } else {
                    Toast::new(first_lines(&out), theme::RED)
                });
                self.reload(Some(0))?;
                if self.changes.is_empty() {
                    self.focus = Pane::Stashes;
                }
                Ok(())
            }
        }
    }

    fn mouse(&mut self, m: crossterm::event::MouseEvent) {
        if self.palette.is_some() || self.confirm.is_some() || self.input.is_some() {
            return;
        }
        let inside = |a: Rect| m.column >= a.x && m.column < a.right() && m.row >= a.y && m.row < a.bottom();
        match m.kind {
            MouseEventKind::ScrollDown if inside(self.changes_area) => self.changes.step(1),
            MouseEventKind::ScrollUp if inside(self.changes_area) => self.changes.step(-1),
            MouseEventKind::Down(MouseButton::Left) if inside(self.changes_area) => {
                self.focus = Pane::Changes;
                self.changes.click(m.row);
            }
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) || inside(self.detail_area) => {
                self.focus = Pane::Stashes;
                if inside(self.list_area) {
                    let i = self.scroll + (m.row - self.list_area.y) as usize;
                    if i < self.visible.len() {
                        self.selected = i;
                    }
                }
            }
            MouseEventKind::ScrollDown if inside(self.detail_area) => {
                self.detail_scroll = self.detail_scroll.saturating_add(3)
            }
            MouseEventKind::ScrollUp if inside(self.detail_area) => {
                self.detail_scroll = self.detail_scroll.saturating_sub(3)
            }
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::ScrollUp => self.step(-1),
            _ => {}
        }
    }

    fn detail_lines(&mut self, width: u16) -> Option<Rc<Vec<Line<'static>>>> {
        let s = self.current()?.clone();
        let key = (s.oid, width);
        if let Some(d) = self.cache.get(&key) {
            return Some(d.clone());
        }
        let lines = Rc::new(build_detail(self.repo, &s, self.branch.as_deref(), width as usize));
        self.cache.insert(key, lines.clone());
        Some(lines)
    }
}

/// `stash/try-the-new-parser` from the stash's title.
fn suggest_branch(s: &Stash) -> String {
    let slug: String = s
        .title()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|w| !w.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() { format!("stash-{}", s.index) } else { format!("stash/{slug}") }
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
    // Wide: changes on the left, stashes and the selected one's details on the right.
    // Narrow: all three stacked.
    let list_rows = (app.visible.len() as u16 + 4).clamp(5, body.height / 3);
    let (changes, right) = if body.width >= 110 {
        let [l, _, r] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Length(1), Constraint::Min(40)]).areas(body);
        (l, r)
    } else {
        // As tall as the changes need, leaving room for the stashes below.
        let h = app.changes.height(body.width).min(body.height.saturating_sub(12).max(body.height / 2));
        let [t, _, b] =
            Layout::vertical([Constraint::Length(h), Constraint::Length(1), Constraint::Min(5)]).areas(body);
        (t, b)
    };
    let [list, _, details] =
        Layout::vertical([Constraint::Length(list_rows), Constraint::Length(1), Constraint::Min(3)]).areas(right);
    app.changes_area = changes;
    let focused = app.focus == Pane::Changes;
    app.changes.draw(f, changes, focused);
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
    let n = app.list.len();
    let mut spans = vec![
        Span::styled("stashes", Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.repo_name.clone(), theme::bold()),
        Span::raw("  "),
        Span::styled(format!("{n} stash{}", if n == 1 { "" } else { "es" }), theme::muted()),
    ];
    if let Some(b) = &app.branch {
        spans.push(Span::styled(format!(" · on {b}"), theme::muted()));
    }
    // Applying onto a dirty tree can fail or conflict, so say what's there.
    spans.push(match app.dirty {
        0 => Span::styled(" · working tree clean", theme::faint()),
        1 => Span::styled(" · 1 changed file", Style::new().fg(theme::YELLOW)),
        d => Span::styled(format!(" · {d} changed files"), Style::new().fg(theme::YELLOW)),
    });
    f.render_widget(Paragraph::new(widgets::truncate(Line::from(spans), area.width)), area);
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(0, 0, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let focused = app.focus == Pane::Stashes;

    // Title row, a gap, then the stashes.
    let title = if focused { Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD) } else { theme::bold() };
    let n = app.list.len();
    let count = if n == 0 { "none yet".to_string() } else { n.to_string() };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled("stashes", title),
            Span::styled(format!("   {count}"), theme::muted()),
        ])),
        inner,
    );
    let inner = Rect { y: inner.y + 2, height: inner.height.saturating_sub(2), ..inner };
    app.list_area = inner;

    let height = inner.height as usize;
    if app.selected < app.scroll {
        app.scroll = app.selected;
    } else if height > 0 && app.selected >= app.scroll + height {
        app.scroll = app.selected + 1 - height;
    }
    if app.list.is_empty() {
        let hint = if app.changes.is_empty() { "  Nothing stashed" } else { "  Pick changes on the left, then enter" };
        f.render_widget(Paragraph::new(Span::styled(hint, theme::muted())), inner);
        return;
    }
    if app.visible.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("  No stashes match", theme::muted())), inner);
        return;
    }

    let now = now();
    let index_w = app.list.iter().map(|s| s.index.to_string().len()).max().unwrap_or(1);
    // Fixed columns on the right: branch, "+untracked", age.
    let branch_w =
        app.list.iter().map(|s| s.branch.as_deref().unwrap_or("detached").chars().count()).max().unwrap_or(0);
    let any_untracked = app.list.iter().any(|s| s.untracked.is_some());
    for (row, &i) in app.visible.iter().enumerate().skip(app.scroll).take(height) {
        let s = &app.list[i];
        let r = Rect { y: inner.y + (row - app.scroll) as u16, height: 1, ..inner };
        let selected = row == app.selected;
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };

        let bar =
            if selected && focused { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };
        let mark = if app.marked.contains(&s.oid) {
            Span::styled("✓ ", Style::new().fg(theme::ACCENT))
        } else {
            Span::raw("  ")
        };
        let mut left = vec![
            bar,
            mark,
            Span::styled(format!("{:>index_w$}  ", s.index), theme::faint()),
            // Without -m the "title" is just the last commit's subject: show it quieter.
            Span::styled(s.title().to_string(), if s.is_wip() { theme::muted() } else { theme::text() }),
        ];
        if s.is_wip() {
            left.push(Span::styled("  wip", theme::faint()));
        }

        let branch = s.branch.clone().unwrap_or_else(|| "detached".into());
        let here = s.branch.is_some() && s.branch == app.branch;
        let mut right = vec![Span::styled(
            format!("{branch:<branch_w$}"),
            if here { Style::new().fg(theme::ACCENT) } else { theme::muted() },
        )];
        if any_untracked {
            let tag = if s.untracked.is_some() { "+untracked" } else { "" };
            right.push(Span::styled(format!("  {tag:<10}"), theme::faint()));
        }
        right.push(Span::styled(format!("{:>9}", relative_time(s.time, now)), theme::faint()));
        right.push(Span::raw("  "));
        let right = Line::from(right);

        let [l, rr] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16 + 2)]).areas(r);
        f.render_widget(Paragraph::new(widgets::truncate(Line::from(left), l.width)).style(Style::new().bg(bg)), l);
        f.render_widget(
            Paragraph::new(right).alignment(ratatui::layout::Alignment::Right).style(Style::new().bg(bg)),
            rr,
        );
    }
}

fn draw_details(f: &mut Frame, app: &mut App, area: Rect) {
    let block = widgets::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.detail_area = area;
    let Some(lines) = app.detail_lines(inner.width) else {
        f.render_widget(Paragraph::new(Span::styled("A stash's files and diff show here.", theme::faint())), inner);
        return;
    };

    // Another stash starts at the top.
    let id = app.current().map(|s| s.oid);
    if id != app.detail_for {
        app.detail_for = id;
        app.detail_scroll = 0;
    }
    let max = (lines.len() as u16).saturating_sub(inner.height);
    app.detail_scroll = app.detail_scroll.min(max);

    let shown: Vec<Line> = lines
        .iter()
        .skip(app.detail_scroll as usize)
        .take(inner.height as usize)
        .map(|l| widgets::truncate(l.clone(), inner.width))
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

fn build_detail(repo: &gix::Repository, s: &Stash, branch: Option<&str>, width: usize) -> Vec<Line<'static>> {
    let now = now();
    let label = |t: &str| Span::styled(format!("{t:<10}"), theme::faint());
    let mut out: Vec<Line<'static>> =
        widgets::wrap(s.title(), width).into_iter().map(|l| Line::styled(l, theme::bold())).collect();
    out.push(Line::from(vec![
        Span::styled(s.name(), Style::new().fg(theme::ACCENT)),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(s.id.clone(), theme::muted()),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(relative_time(s.time, now), theme::muted()),
    ]));
    out.push(Line::raw(""));

    let on = s.branch.clone().unwrap_or_else(|| "detached HEAD".into());
    let here = s.branch.is_some() && s.branch.as_deref() == branch;
    let mut branch_row =
        vec![label("branch"), Span::styled(on, Style::new().fg(if here { theme::ACCENT } else { theme::TEXT }))];
    if here {
        branch_row.push(Span::styled("  (current)", theme::faint()));
    }
    out.push(Line::from(branch_row));
    out.push(Line::from(vec![
        label("made on"),
        Span::styled(format!("{}  ", s.base_id), theme::muted()),
        Span::styled(s.base_subject.clone(), theme::text()),
    ]));
    // Has HEAD moved since? Applying then may conflict.
    if let Ok(head) = repo.head_id() {
        let head = head.detach();
        if head != s.base {
            let (ahead, behind) = crate::git::branches::ahead_behind(repo, head, s.base);
            let note = match (ahead, behind) {
                (a, 0) => format!("HEAD is {a} commit{} past it", if a == 1 { "" } else { "s" }),
                _ => "HEAD is on a different line of history".to_string(),
            };
            out.push(Line::from(vec![label(""), Span::styled(note, Style::new().fg(theme::YELLOW))]));
        }
    }

    match stash::files(repo, s) {
        Ok((tracked, untracked)) => {
            out.push(Line::raw(""));
            out.extend(diffview::file_list(&tracked, None));
            if !untracked.is_empty() {
                out.push(Line::raw(""));
                out.extend(diffview::file_list(&untracked, Some("untracked")));
            }
            out.extend(diffview::patches(repo, &tracked));
            out.extend(diffview::patches(repo, &untracked));
        }
        Err(e) => out.push(Line::styled(format!("couldn't read the stash: {e}"), Style::new().fg(theme::RED))),
    }
    out
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let left = if app.searching || !app.query.is_empty() {
        let mut spans =
            vec![Span::styled("/ ", Style::new().fg(theme::ACCENT)), Span::styled(app.query.clone(), theme::text())];
        if app.searching {
            spans.push(Span::styled("▏", Style::new().fg(theme::ACCENT)));
        }
        spans.push(Span::styled(format!("   {} of {}", app.visible.len(), app.list.len()), theme::muted()));
        Line::from(spans)
    } else if !app.marked.is_empty() {
        Line::styled(format!("✓ {} selected", app.marked.len()), Style::new().fg(theme::ACCENT))
    } else {
        Line::raw("")
    };
    // Most important first; the tail of the middle goes when the line is too narrow.
    let mut pairs: Vec<(&str, &str)> = if app.searching {
        vec![("enter", "keep filter"), ("esc", "clear")]
    } else if app.focus == Pane::Changes {
        let mut p = if app.changes.is_empty() { Vec::new() } else { app.changes.hints() };
        p.extend([("ctrl+p", "commands"), ("tab", "stashes"), ("q", "quit")]);
        p
    } else if !app.marked.is_empty() {
        vec![("d", "drop"), ("space", "select"), ("esc", "clear")]
    } else if app.list.is_empty() {
        vec![("tab", "changes"), ("q", "quit")]
    } else {
        vec![
            ("enter", "apply"),
            ("p", "pop"),
            ("d", "drop"),
            ("ctrl+p", "commands"),
            ("b", "branch"),
            ("tab", "changes"),
            ("q", "quit"),
        ]
    };
    let room = (area.width as usize).saturating_sub(left.width() + 3);
    while pairs.len() > 2 && widgets::hints(&pairs).width() > room {
        pairs.remove(pairs.len() - 2);
    }
    let right = widgets::hints(&pairs);
    let [l, r] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(widgets::truncate(left, l.width)), l);
    f.render_widget(Paragraph::new(right), r);
}

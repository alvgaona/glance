//! `gg stash`: your changes on the left, stashes on the right. Stash what you pick, or
//! apply, pop, drop and branch off a stash. Writes go through git (`git stash …`).

mod changes;
mod draw;
mod keys;

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use gix::ObjectId;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};

use changes::{Mode, Picked, Picker};
use draw::{build_detail, draw};

use crate::fmt::{now, relative_time};
use crate::git::stash::{self, Stash};
use crate::tui::app::{self, Flow, Screen};
use crate::tui::kit::{self, Command, Confirm, Input, InputEvent, Palette, PaletteEvent, Scroll, Toast, footer, list};
use crate::tui::panels::{self, diff};
use crate::tui::{first_lines, theme};

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
    detail_scroll: Scroll,
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
        detail_scroll: Scroll::default(),
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

    app::run(&mut app)
}

impl Screen for App<'_> {
    type Out = ();

    fn draw(&mut self, f: &mut Frame) {
        draw(f, self)
    }
    fn key(&mut self, k: KeyEvent) -> Result<Flow<()>> {
        Ok(if self.on_key(k)? { Flow::Done(()) } else { Flow::Stay })
    }
    fn mouse(&mut self, m: MouseEvent) {
        self.on_mouse(m)
    }
    fn toast(&mut self) -> &mut Option<Toast> {
        &mut self.toast
    }
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

    /// `git stash apply` or `git stash pop` on the current stash.
    fn apply(&mut self, pop: bool) -> Result<()> {
        let Some(s) = self.current().cloned() else { return Ok(()) };
        self.toast = Some(match stash::apply(&s, pop) {
            Ok(_) => {
                let done = if pop { "Popped" } else { "Applied" };
                Toast::new(format!("{done} {}: {}", s.name(), s.title()), theme::GREEN)
            }
            Err(out) if out.contains("CONFLICT") => {
                // git keeps the stash when a pop conflicts.
                let n = out.lines().filter(|l| l.contains("CONFLICT")).count();
                Toast::new(
                    format!(
                        "Applied with conflicts in {n} file{}. The stash is kept; fix them, then drop it",
                        if n == 1 { "" } else { "s" }
                    ),
                    theme::RED,
                )
            }
            Err(out) => Toast::new(first_lines(&out), theme::RED),
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
        let failed: Vec<String> = list.iter().filter_map(|s| stash::drop(s).err()).collect();
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
                let done = format!("On new branch {text}, with {} applied", s.name());
                self.toast = Some(Toast::ran(&stash::branch(&s, &text), done));
                self.reload(None)
            }
            Ask::New(picked) => {
                let ran = picked.push.run(&text);
                if ran.is_ok() {
                    self.changes.reset_ticks();
                }
                self.toast = Some(Toast::ran(&ran, format!("Stashed {} as stash@{{0}}", picked.describe())));
                self.reload(Some(0))?;
                if self.changes.is_empty() {
                    self.focus = Pane::Stashes;
                }
                Ok(())
            }
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if self.palette.is_some() || self.confirm.is_some() || self.input.is_some() {
            return;
        }
        let inside = |a: Rect| list::inside(a, &m);
        match m.kind {
            MouseEventKind::ScrollDown if inside(self.changes_area) => self.changes.step(1),
            MouseEventKind::ScrollUp if inside(self.changes_area) => self.changes.step(-1),
            MouseEventKind::Down(MouseButton::Left) if inside(self.changes_area) => {
                self.focus = Pane::Changes;
                self.changes.click(m.row);
            }
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) || inside(self.detail_area) => {
                self.focus = Pane::Stashes;
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

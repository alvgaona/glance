//! `gg br`: branches, full screen. Check out, delete local or remote, clean up in bulk.
//! Writes go through git (`git switch`, `git branch -D`, `git push --delete`).

mod delete;
mod draw;
mod keys;
mod prs;

use draw::{build_detail, draw};

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::mpsc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph, Wrap},
};

use crate::fmt::paths::short as short_path;
use crate::fmt::{now, relative_time};
use crate::git::branches::{self, Branch, Branches};
use crate::git::run::Ran;
use crate::github::{self, Pr};
use crate::process;
use crate::tui::app::{self, Flow, Screen};
use crate::tui::kit::{self, Command, Confirm, Input, InputEvent, Palette, PaletteEvent, Scroll, Toast, footer, list};
use crate::tui::{first_lines, theme};

/// Pull requests, loaded from gh in the background.
enum Prs {
    /// No GitHub remote or no gh.
    Off,
    Loading(mpsc::Receiver<Result<Vec<Pr>>>),
    Ready(Vec<Pr>),
    /// gh failed; the reason was shown in a toast.
    Failed,
}

#[derive(Clone)]
enum Cmd {
    Checkout,
    Rename,
    Fetch,
    OpenPr,
    ReloadPrs,
    DeleteLocal,
    DeleteRemote,
    Mark,
    MarkMerged,
    ClearMarks,
    Copy,
    ToggleRemotes,
    Search,
    Details,
    Quit,
}

/// A delete waiting for the user to confirm it.
enum Pending {
    Local(Vec<Branch>),
    /// (remote, branch) pairs for `git push <remote> --delete <branch>`.
    Remote(Vec<(String, String)>),
}

/// Choosing which remotes to delete from, when there's more than one.
struct RemotePick {
    title: String,
    items: Vec<PickItem>,
    cursor: usize,
}

struct PickItem {
    remote: String,
    branch: String,
    /// How it compares to the local branch: "up to date", "2 to push", ...
    note: String,
    on: bool,
}

impl RemotePick {
    fn chosen(&self) -> Vec<(String, String)> {
        self.items.iter().filter(|i| i.on).map(|i| (i.remote.clone(), i.branch.clone())).collect()
    }

    fn draw(&self, f: &mut Frame, area: Rect) {
        let shown = self.items.len().min(12);
        let start = self.cursor.saturating_sub(shown - 1);
        let r = kit::centered(area, 68, shown as u16 + 9);
        f.render_widget(ratatui::widgets::Clear, r);
        let block = kit::panel(theme::ELEMENT);
        let inner = block.inner(r);
        f.render_widget(block, r);

        let name_w =
            self.items.iter().map(|i| i.remote.chars().count() + i.branch.chars().count() + 1).max().unwrap_or(0);
        let mut lines = vec![Line::styled(self.title.clone(), theme::bold()), Line::raw("")];
        for (n, item) in self.items.iter().enumerate().skip(start).take(shown) {
            let selected = n == self.cursor;
            let bg = if selected { theme::BORDER } else { theme::ELEMENT };
            let check = if item.on {
                Span::styled("✓ ", Style::new().fg(theme::RED).bg(bg))
            } else {
                Span::styled("· ", Style::new().fg(theme::FAINT).bg(bg))
            };
            let full = format!("{}/{}", item.remote, item.branch);
            let pad = " ".repeat(name_w.saturating_sub(full.chars().count()) + 3);
            let name_style = if item.on { theme::text() } else { theme::muted() };
            let mut line = Line::from(vec![
                Span::styled(if selected { "┃ " } else { "  " }, Style::new().fg(theme::ACCENT).bg(bg)),
                check,
                Span::styled(full + &pad, name_style.bg(bg)),
                Span::styled(item.note.clone(), theme::muted().bg(bg)),
            ]);
            let w = line.width();
            line.spans.push(Span::styled(" ".repeat((inner.width as usize).saturating_sub(w)), Style::new().bg(bg)));
            lines.push(kit::truncate(line, inner.width));
        }
        let n = self.items.iter().filter(|i| i.on).count();
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "This removes them for everyone using those remotes. Local branches stay.",
            theme::muted(),
        ));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            kit::pill(&format!("enter  Delete {n}"), if n > 0 { theme::RED } else { theme::BORDER }, theme::BG),
            Span::raw("  "),
            kit::pill("space  toggle", theme::BORDER, theme::TEXT),
            Span::raw("  "),
            kit::pill("esc  Cancel", theme::BORDER, theme::TEXT),
        ]));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

/// What the screen does outside its own state; [`Screen::apply`] does it.
enum Effect {
    ToggleRemotes,
    Rename {
        from: String,
        to: String,
    },
    Checkout(String),
    /// `git fetch --all --prune` in the background.
    Fetch,
    ReloadPrs,
    OpenPr {
        url: String,
        number: u64,
    },
    Copy(String),
    /// Ask before deleting the marked branches (or the current one); building the
    /// question reads their history.
    AskDeleteLocal,
    AskDeleteRemote,
    /// Leave the full screen, run these `git` commands (they may ask for credentials), come back.
    InTerminal(Vec<Vec<String>>),
}

struct App<'r> {
    repo: &'r gix::Repository,
    data: Branches,
    remotes: bool,
    /// Indexes into `data.list` that match the search.
    visible: Vec<usize>,
    selected: usize,
    scroll: usize,
    /// Branch names picked with space.
    marked: BTreeSet<String>,
    query: String,
    searching: bool,
    details: bool,
    /// Details by (branch, tip, panel width): text is wrapped to the width. Filled in `tick`.
    cache: HashMap<(String, gix::ObjectId, u16), Rc<Vec<Line<'static>>>>,
    /// The details panel's width at the last draw.
    detail_width: u16,
    detail_scroll: Scroll,
    /// Which branch `detail_scroll` belongs to; another one starts at the top.
    detail_for: String,
    detail_area: Rect,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    confirm: Option<(Confirm, Pending)>,
    pick: Option<RemotePick>,
    /// The rename dialog and the branch it renames.
    rename: Option<(Input, String)>,
    /// A background `git fetch`, while it runs.
    fetching: Option<mpsc::Receiver<Ran>>,
    list_area: Rect,
    repo_name: String,
    /// What the last single delete removed, for its toast.
    last_deleted: String,
    prs: Prs,
    /// Branch name -> index into the ready PR list.
    pr_of: HashMap<String, usize>,
    /// Remote name -> GitHub account, to tell our PRs from same-named ones in other forks.
    owners: HashMap<String, String>,
}

pub fn run(repo: &gix::Repository) -> Result<()> {
    let mut app = App {
        repo,
        data: branches::read(repo, false)?,
        remotes: false,
        visible: Vec::new(),
        selected: 0,
        scroll: 0,
        marked: BTreeSet::new(),
        query: String::new(),
        searching: false,
        details: true,
        cache: HashMap::new(),
        detail_width: 0,
        detail_scroll: Scroll::default(),
        detail_for: String::new(),
        detail_area: Rect::default(),
        toast: None,
        palette: None,
        confirm: None,
        pick: None,
        rename: None,
        fetching: None,
        list_area: Rect::default(),
        repo_name: crate::git::repo_name(repo),
        last_deleted: String::new(),
        prs: Prs::Off,
        pr_of: HashMap::new(),
        owners: github::remote_owners(repo),
    };
    app.load_prs();
    app.filter();
    // Start on the current branch.
    if let Some(i) = app.visible.iter().position(|&i| app.data.list[i].current) {
        app.selected = i;
    }

    app::run(&mut app)
}

impl Screen for App<'_> {
    type Out = ();
    type Effect = Effect;

    fn draw(&mut self, f: &mut Frame) {
        draw(f, self)
    }
    fn apply(&mut self, e: Effect) -> Result<Flow<(), Effect>> {
        match e {
            Effect::ToggleRemotes => {
                self.remotes = !self.remotes;
                self.reload()?;
                let msg = if self.remotes { "Showing remote branches" } else { "Local branches only" };
                self.toast = Some(Toast::new(msg, theme::ACCENT));
            }
            Effect::Rename { from, to } => {
                let ran = branches::rename(&from, &to);
                self.toast = Some(Toast::ran(&ran, format!("Renamed {from} → {to}")));
                if ran.is_ok() {
                    if self.marked.remove(&from) {
                        self.marked.insert(to.clone());
                    }
                    self.reload_selecting(Some(to))?;
                }
            }
            Effect::Checkout(target) => {
                self.toast = Some(Toast::ran(&branches::switch(&target), format!("Switched to {target}")));
                self.reload()?;
            }
            Effect::Fetch => self.fetch(),
            Effect::ReloadPrs => {
                self.load_prs();
                if matches!(self.prs, Prs::Loading(_)) {
                    self.toast = Some(Toast::new("Reloading pull requests…", theme::ACCENT));
                }
            }
            Effect::OpenPr { url, number } => {
                let msg = if process::open(&url) { format!("Opened #{number}") } else { url };
                self.toast = Some(Toast::new(msg, theme::ACCENT));
            }
            Effect::Copy(name) => {
                crate::clipboard::copy(&name);
                self.toast = Some(Toast::new(format!("Copied {name}"), theme::ACCENT));
            }
            Effect::AskDeleteLocal => self.ask_delete_local(),
            Effect::AskDeleteRemote => self.ask_delete_remote(),
            Effect::InTerminal(cmds) => {
                let results: Vec<Ran> = cmds.iter().map(|a| crate::git::run::git(a)).collect();
                self.report(&results);
                self.reload()?;
            }
        }
        Ok(Flow::Stay)
    }
    fn in_terminal(e: &Effect) -> bool {
        matches!(e, Effect::InTerminal(_))
    }
    fn key(&mut self, k: KeyEvent) -> Flow<(), Effect> {
        self.on_key(k)
    }
    fn mouse(&mut self, m: MouseEvent) {
        self.on_mouse(m)
    }
    fn tick(&mut self) -> Result<()> {
        self.poll_prs();
        self.poll_fetch()?;
        // Zero until the first draw has measured the panel.
        if self.details && self.detail_width > 0 {
            self.load_detail(self.detail_width);
        }
        Ok(())
    }
    fn toast(&mut self) -> &mut Option<Toast> {
        &mut self.toast
    }
}

fn commands() -> Vec<Command<Cmd>> {
    vec![
        Command { label: "Check out this branch", keys: "enter", action: Cmd::Checkout },
        Command { label: "Rename branch", keys: "r", action: Cmd::Rename },
        Command { label: "Fetch all remotes (and prune)", keys: "f", action: Cmd::Fetch },
        Command { label: "Delete local branch", keys: "d", action: Cmd::DeleteLocal },
        Command { label: "Delete branch on remotes", keys: "D", action: Cmd::DeleteRemote },
        Command { label: "Select or unselect", keys: "space", action: Cmd::Mark },
        Command { label: "Select merged and gone branches", keys: "m", action: Cmd::MarkMerged },
        Command { label: "Clear selection", keys: "esc", action: Cmd::ClearMarks },
        Command { label: "Copy branch name", keys: "y", action: Cmd::Copy },
        Command { label: "Open pull request in the browser", keys: "o", action: Cmd::OpenPr },
        Command { label: "Reload pull requests", keys: "R", action: Cmd::ReloadPrs },
        Command { label: "Show or hide remote branches", keys: "a", action: Cmd::ToggleRemotes },
        Command { label: "Search branches", keys: "/", action: Cmd::Search },
        Command { label: "Show or hide details", keys: "tab", action: Cmd::Details },
        Command { label: "Quit", keys: "q", action: Cmd::Quit },
    ]
}

impl App<'_> {
    fn default_name(&self) -> Option<&str> {
        self.data.default.as_ref().map(|(n, _)| n.as_str())
    }

    fn filter(&mut self) {
        let q = self.query.to_lowercase();
        self.visible = (0..self.data.list.len())
            .filter(|&i| q.is_empty() || self.data.list[i].name.to_lowercase().contains(&q))
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }

    fn reload(&mut self) -> Result<()> {
        self.reload_selecting(None)
    }

    /// Reloads, then puts the cursor on `name` (or back on the branch it was on).
    fn reload_selecting(&mut self, name: Option<String>) -> Result<()> {
        let keep = name.or_else(|| self.current().map(|b| b.name.clone()));
        self.data = branches::read(self.repo, self.remotes)?;
        self.cache.clear();
        let names: BTreeSet<String> = self.data.list.iter().map(|b| b.name.clone()).collect();
        self.marked.retain(|m| names.contains(m));
        self.index_prs();
        self.filter();
        if let Some(name) = keep
            && let Some(i) = self.visible.iter().position(|&i| self.data.list[i].name == name)
        {
            self.selected = i;
        }
        Ok(())
    }

    fn current(&self) -> Option<&Branch> {
        self.visible.get(self.selected).map(|&i| &self.data.list[i])
    }

    /// `git fetch --all --prune` on a background thread; `poll_fetch` picks up the result.
    /// It must never prompt (that would draw over the screen), so prompts are turned off:
    /// a remote that needs a password fails and says so.
    fn fetch(&mut self) {
        if self.fetching.is_some() {
            return;
        }
        let job = branches::fetch(self.repo);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
        self.fetching = Some(rx);
    }

    fn poll_fetch(&mut self) -> Result<()> {
        let Some(rx) = &self.fetching else { return Ok(()) };
        let ran = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => Err("fetch stopped".into()),
        };
        self.fetching = None;
        let ran = ran.map_err(|out| format!("Fetch failed: {}", first_lines(&out)));
        self.toast = Some(Toast::ran(&ran, "Fetched all remotes"));
        self.reload()?;
        // Pushes and merges may have changed PRs too.
        if !matches!(self.prs, Prs::Loading(_) | Prs::Off) {
            self.load_prs();
        }
        Ok(())
    }

    /// Marked branches, or the one under the cursor when nothing is marked.
    fn targets(&self) -> Vec<Branch> {
        if self.marked.is_empty() {
            self.current().cloned().into_iter().collect()
        } else {
            self.data.list.iter().filter(|b| self.marked.contains(&b.name)).cloned().collect()
        }
    }

    fn protected(&self, b: &Branch) -> bool {
        b.current || b.worktree.is_some() || (!b.remote && Some(b.name.as_str()) == self.default_name())
    }

    fn step(&mut self, n: isize) {
        let last = self.visible.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + n).clamp(0, last) as usize;
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if self.palette.is_some() || self.confirm.is_some() || self.pick.is_some() || self.rename.is_some() {
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
                    && i < self.visible.len()
                {
                    self.selected = i;
                }
            }
            _ => {}
        }
    }

    /// Reads the current branch's details into the cache, unless they're there.
    fn load_detail(&mut self, width: u16) {
        let Some(b) = self.current().cloned() else { return };
        let key = (b.name.clone(), b.oid, width);
        if !self.cache.contains_key(&key) {
            let default = self.data.default.as_ref();
            let lines = build_detail(self.repo, &b, default, self.protected(&b), self.pr(&b), width as usize);
            self.cache.insert(key, Rc::new(lines));
        }
    }

    fn detail(&self, width: u16) -> Option<Rc<Vec<Line<'static>>>> {
        let b = self.current()?;
        self.cache.get(&(b.name.clone(), b.oid, width)).cloned()
    }
}

/// Local vs. a remote copy, from (ahead, behind): "up to date", "2 to push", "1 to pull".
fn sync_note((ahead, behind): (usize, usize)) -> String {
    crate::fmt::sync::words(ahead, behind)
}

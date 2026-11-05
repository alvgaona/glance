//! `gg br`: branches, full screen. Check out, delete local or remote, clean up in bulk.
//! Writes go through git (`git switch`, `git branch -D`, `git push --delete`).

use std::collections::{BTreeSet, HashMap};
use std::process::{Command as Process, Stdio};
use std::rc::Rc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph, Wrap},
};

use super::theme;
use super::widgets::{self, Command, Confirm, Palette, PaletteEvent, Toast};
use crate::core::branches::{self, Branch, Branches};
use crate::render::relative_time;

#[derive(Clone)]
enum Cmd {
    Checkout,
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
        let r = widgets::centered(area, 68, shown as u16 + 9);
        f.render_widget(ratatui::widgets::Clear, r);
        let block = widgets::panel(theme::ELEMENT);
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
            lines.push(widgets::truncate(line, inner.width));
        }
        let n = self.items.iter().filter(|i| i.on).count();
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "This removes them for everyone using those remotes. Local branches stay.",
            theme::muted(),
        ));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            widgets::pill(&format!("enter  Delete {n}"), if n > 0 { theme::RED } else { theme::BORDER }, theme::BG),
            Span::raw("  "),
            widgets::pill("space  toggle", theme::BORDER, theme::TEXT),
            Span::raw("  "),
            widgets::pill("esc  Cancel", theme::BORDER, theme::TEXT),
        ]));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

/// What the event loop has to do after a key press.
enum Effect {
    Quit,
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
    cache: HashMap<(String, gix::ObjectId), Rc<Vec<Line<'static>>>>,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    confirm: Option<(Confirm, Pending)>,
    pick: Option<RemotePick>,
    list_area: Rect,
    repo_name: String,
    /// What the last single delete removed, for its toast.
    last_deleted: String,
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
        toast: None,
        palette: None,
        confirm: None,
        pick: None,
        list_area: Rect::default(),
        repo_name: crate::core::repo_name(repo),
        last_deleted: String::new(),
    };
    app.filter();
    // Start on the current branch.
    if let Some(i) = app.visible.iter().position(|&i| app.data.list[i].current) {
        app.selected = i;
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
            let effect = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => app.key(k)?,
                Event::Mouse(m) => {
                    app.mouse(m);
                    None
                }
                _ => None,
            };
            match effect {
                Some(Effect::Quit) => return Ok(()),
                Some(Effect::InTerminal(cmds)) => {
                    leave();
                    let results: Vec<_> = cmds.iter().map(|args| git(args)).collect();
                    terminal = enter();
                    app.report(&results);
                    app.reload()?;
                }
                None => {}
            }
        }
    })();
    leave();
    result
}

fn enter() -> DefaultTerminal {
    let t = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    t
}

fn leave() {
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();
}

/// Runs git, returning whether it worked and its output (stderr first; that's where git talks).
/// Prompts for credentials still reach the terminal: ssh and git ask on /dev/tty.
fn git(args: &[String]) -> (bool, String) {
    match Process::new("git").args(args).stdin(Stdio::inherit()).output() {
        Ok(out) => {
            let text = format!("{}{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
            (out.status.success(), text.trim().to_string())
        }
        Err(e) => (false, format!("could not run git: {e}")),
    }
}

fn commands() -> Vec<Command<Cmd>> {
    vec![
        Command { label: "Check out this branch", keys: "enter", action: Cmd::Checkout },
        Command { label: "Delete local branch", keys: "d", action: Cmd::DeleteLocal },
        Command { label: "Delete branch on remotes", keys: "D", action: Cmd::DeleteRemote },
        Command { label: "Select or unselect", keys: "space", action: Cmd::Mark },
        Command { label: "Select merged and gone branches", keys: "m", action: Cmd::MarkMerged },
        Command { label: "Clear selection", keys: "esc", action: Cmd::ClearMarks },
        Command { label: "Copy branch name", keys: "y", action: Cmd::Copy },
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
        let keep = self.current().map(|b| b.name.clone());
        self.data = branches::read(self.repo, self.remotes)?;
        self.cache.clear();
        let names: BTreeSet<String> = self.data.list.iter().map(|b| b.name.clone()).collect();
        self.marked.retain(|m| names.contains(m));
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

    /// Marked branches, or the one under the cursor when nothing is marked.
    fn targets(&self) -> Vec<Branch> {
        if self.marked.is_empty() {
            self.current().cloned().into_iter().collect()
        } else {
            self.data.list.iter().filter(|b| self.marked.contains(&b.name)).cloned().collect()
        }
    }

    fn protected(&self, b: &Branch) -> bool {
        b.current || (!b.remote && Some(b.name.as_str()) == self.default_name())
    }

    fn step(&mut self, n: isize) {
        let last = self.visible.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + n).clamp(0, last) as usize;
    }

    fn key(&mut self, k: KeyEvent) -> Result<Option<Effect>> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Ok(Some(Effect::Quit));
        }
        if let Some(p) = &mut self.pick {
            let last = p.items.len().saturating_sub(1);
            match k.code {
                KeyCode::Down | KeyCode::Char('j') => p.cursor = (p.cursor + 1).min(last),
                KeyCode::Up | KeyCode::Char('k') => p.cursor = p.cursor.saturating_sub(1),
                KeyCode::Char(' ') => {
                    p.items[p.cursor].on = !p.items[p.cursor].on;
                }
                KeyCode::Char('a') => {
                    let all = p.items.iter().all(|i| i.on);
                    p.items.iter_mut().for_each(|i| i.on = !all);
                }
                KeyCode::Enter => {
                    let chosen = p.chosen();
                    if chosen.is_empty() {
                        self.toast = Some(Toast::new("Nothing selected", theme::YELLOW));
                        return Ok(None);
                    }
                    self.pick = None;
                    self.marked.clear();
                    return Ok(Some(Effect::InTerminal(self.remote_deletes(chosen))));
                }
                KeyCode::Esc | KeyCode::Char('q') => self.pick = None,
                _ => {}
            }
            return Ok(None);
        }
        if self.confirm.is_some() {
            return Ok(match k.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let (_, pending) = self.confirm.take().unwrap();
                    self.marked.clear();
                    Some(Effect::InTerminal(match pending {
                        Pending::Local(list) => {
                            self.last_deleted = list.first().map(|b| b.name.clone()).unwrap_or_default();
                            list.iter().map(|b| vec!["branch".into(), "-D".into(), b.name.clone()]).collect()
                        }
                        Pending::Remote(list) => self.remote_deletes(list),
                    }))
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    self.confirm = None;
                    None
                }
                _ => None,
            });
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
            KeyCode::PageDown => {
                self.step(15);
                return Ok(None);
            }
            KeyCode::PageUp => {
                self.step(-15);
                return Ok(None);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
                return Ok(None);
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.visible.len().saturating_sub(1);
                return Ok(None);
            }
            KeyCode::Enter => Cmd::Checkout,
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::SHIFT) => Cmd::DeleteRemote,
            KeyCode::Char('d') => Cmd::DeleteLocal,
            KeyCode::Char('D') => Cmd::DeleteRemote,
            KeyCode::Char(' ') => Cmd::Mark,
            KeyCode::Char('m') => Cmd::MarkMerged,
            KeyCode::Char('y') => Cmd::Copy,
            KeyCode::Char('a') => Cmd::ToggleRemotes,
            KeyCode::Char('/') => Cmd::Search,
            KeyCode::Tab => Cmd::Details,
            KeyCode::Esc if !self.marked.is_empty() => Cmd::ClearMarks,
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.filter();
                return Ok(None);
            }
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Quit,
            _ => return Ok(None),
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Result<Option<Effect>> {
        match cmd {
            Cmd::Quit => return Ok(Some(Effect::Quit)),
            Cmd::Search => self.searching = true,
            Cmd::Details => self.details = !self.details,
            Cmd::ToggleRemotes => {
                self.remotes = !self.remotes;
                self.reload()?;
                let msg = if self.remotes { "Showing remote branches" } else { "Local branches only" };
                self.toast = Some(Toast::new(msg, theme::ACCENT));
            }
            Cmd::Copy => {
                if let Some(b) = self.current() {
                    let name = b.name.clone();
                    crate::fzf::copy(&name);
                    self.toast = Some(Toast::new(format!("Copied {name}"), theme::ACCENT));
                }
            }
            Cmd::Mark => {
                if let Some(b) = self.current() {
                    let name = b.name.clone();
                    if !self.marked.remove(&name) {
                        self.marked.insert(name);
                    }
                    self.step(1);
                }
            }
            Cmd::MarkMerged => {
                let picks: Vec<String> = self
                    .data
                    .list
                    .iter()
                    .filter(|b| !b.remote && (b.merged || b.gone()) && !self.protected(b))
                    .map(|b| b.name.clone())
                    .collect();
                let msg = match picks.len() {
                    0 => "No merged or gone branches".to_string(),
                    n => format!("Selected {n} merged or gone branch{}", if n == 1 { "" } else { "es" }),
                };
                self.marked.extend(picks);
                self.toast = Some(Toast::new(msg, theme::ACCENT));
            }
            Cmd::ClearMarks => self.marked.clear(),
            Cmd::Checkout => {
                let Some(b) = self.current().cloned() else {
                    return Ok(None);
                };
                // For origin/feat, `git switch feat` creates a local branch that tracks it.
                let target = if b.remote {
                    b.name.split_once('/').map_or(b.name.clone(), |(_, n)| n.to_string())
                } else {
                    b.name.clone()
                };
                if self.data.list.iter().any(|x| x.current && x.name == target) {
                    self.toast = Some(Toast::new(format!("Already on {target}"), theme::ACCENT));
                    return Ok(None);
                }
                let (ok, out) = git(&["switch".into(), target.clone()]);
                self.toast = Some(if ok {
                    Toast::new(format!("Switched to {target}"), theme::GREEN)
                } else {
                    Toast::new(first_lines(&out), theme::RED)
                });
                self.reload()?;
            }
            Cmd::DeleteLocal => self.ask_delete_local(),
            Cmd::DeleteRemote => self.ask_delete_remote(),
        }
        Ok(None)
    }

    fn ask_delete_local(&mut self) {
        let all = self.targets();
        if all.iter().any(|b| b.remote) && all.len() == 1 {
            self.toast = Some(Toast::new("That's a remote branch: D deletes it on the remote", theme::YELLOW));
            return;
        }
        let (keep, list): (Vec<Branch>, Vec<Branch>) =
            all.into_iter().filter(|b| !b.remote).partition(|b| self.protected(b));
        if list.is_empty() {
            let why = match keep.first() {
                Some(b) if b.current => {
                    format!("{} is checked out; switch to another branch first", b.name)
                }
                Some(b) => format!("{} is the main branch", b.name),
                None => "Nothing to delete".into(),
            };
            self.toast = Some(Toast::new(why, theme::YELLOW));
            return;
        }
        let main = self.default_name().unwrap_or("main").to_string();
        let only_here = |b: &Branch| branches::only_here(self.repo, b);

        let (title, body) = if list.len() == 1 {
            let b = &list[0];
            let n = only_here(b);
            let mut body = if n == 0 && b.merged {
                format!("It's merged into {main}, so no work is lost.")
            } else if n == 0 {
                "Every commit is also on another branch, so no work is lost.".to_string()
            } else {
                format!(
                    "{n} commit{} on this branch {} nowhere else.\nTo bring it back later:\n$ git branch {} {}",
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "exists" } else { "exist" },
                    b.name,
                    b.id
                )
            };
            if b.gone() {
                body += "\nIts remote branch is already gone.";
            }
            (format!("Delete branch {}?", b.name), body)
        } else {
            let mut body = String::new();
            let mut unmerged = 0;
            for b in list.iter().take(10) {
                let n = only_here(b);
                let note = if n == 0 {
                    if b.merged { "merged" } else { "safe" }.to_string()
                } else {
                    unmerged += 1;
                    format!("{n} commit{} only here", if n == 1 { "" } else { "s" })
                };
                body += &format!("$ {}  {}  ({note})\n", b.name, b.id);
            }
            if list.len() > 10 {
                body += &format!("…and {} more\n", list.len() - 10);
            }
            if unmerged > 0 {
                body += &format!(
                    "{unmerged} of them have commits no other branch has. `git branch <name> <id>` brings one back."
                );
            } else {
                body += "Every commit is also on another branch; no work is lost.";
            }
            (format!("Delete {} branches?", list.len()), body.trim_end().to_string())
        };
        let danger = list.iter().any(|b| only_here(b) > 0);
        self.confirm =
            Some((Confirm { title, body, yes: "Delete".into(), no: "Cancel".into(), danger }, Pending::Local(list)));
    }

    /// `git push <remote> --delete <branch>` for each pair.
    fn remote_deletes(&mut self, list: Vec<(String, String)>) -> Vec<Vec<String>> {
        self.last_deleted = list.first().map(|(r, b)| format!("{b} on {r}")).unwrap_or_default();
        list.into_iter().map(|(remote, branch)| vec!["push".into(), remote, "--delete".into(), branch]).collect()
    }

    fn ask_delete_remote(&mut self) {
        let main = self.default_name().map(str::to_string);
        let picked = self.targets();
        let mut items: Vec<PickItem> = Vec::new();
        for b in &picked {
            for (remote, branch) in b.remote_targets() {
                if Some(&branch) == main.as_ref() {
                    continue;
                }
                let note = match b.copies.iter().find(|c| c.remote == remote && c.branch == branch) {
                    Some(c) => sync_note(branches::ahead_behind(self.repo, b.oid, c.oid)),
                    None => String::new(),
                };
                items.push(PickItem { remote, branch, note, on: true });
            }
        }
        if items.is_empty() {
            self.toast = Some(Toast::new("Not on any remote", theme::YELLOW));
            return;
        }
        if items.len() > 1 {
            let title = if picked.len() == 1 {
                format!("Delete {} on which remotes?", picked[0].name)
            } else {
                format!("Delete {} branches on which remotes?", picked.len())
            };
            self.pick = Some(RemotePick { title, items, cursor: 0 });
            return;
        }
        let targets: Vec<(String, String)> = items.into_iter().map(|i| (i.remote, i.branch)).collect();
        let mut body: String = targets.iter().map(|(r, b)| format!("$ git push {r} --delete {b}\n")).collect();
        body += "This removes it for everyone using the remote. Local branches stay.";
        let title = if targets.len() == 1 {
            format!("Delete {}/{} on the remote?", targets[0].0, targets[0].1)
        } else {
            format!("Delete {} branches on the remote?", targets.len())
        };
        self.confirm = Some((
            Confirm { title, body, yes: "Delete on remote".into(), no: "Cancel".into(), danger: true },
            Pending::Remote(targets),
        ));
    }

    /// Toast summarising what the git commands did.
    fn report(&mut self, results: &[(bool, String)]) {
        let failed: Vec<&String> = results.iter().filter(|(ok, _)| !ok).map(|(_, out)| out).collect();
        self.toast = Some(if failed.is_empty() {
            let n = results.len();
            Toast::new(
                if n == 1 { format!("Deleted {}", self.last_deleted) } else { format!("Deleted {n} branches") },
                theme::GREEN,
            )
        } else {
            Toast::new(format!("{} failed: {}", failed.len(), first_lines(failed[0])), theme::RED)
        });
    }

    fn mouse(&mut self, m: crossterm::event::MouseEvent) {
        if self.palette.is_some() || self.confirm.is_some() || self.pick.is_some() {
            return;
        }
        match m.kind {
            MouseEventKind::ScrollDown => self.step(1),
            MouseEventKind::ScrollUp => self.step(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                let a = self.list_area;
                if m.column >= a.x && m.column < a.right() && m.row >= a.y && m.row < a.bottom() {
                    let i = self.scroll + (m.row - a.y) as usize;
                    if i < self.visible.len() {
                        self.selected = i;
                    }
                }
            }
            _ => {}
        }
    }

    fn detail_lines(&mut self) -> Option<Rc<Vec<Line<'static>>>> {
        let b = self.current()?.clone();
        let key = (b.name.clone(), b.oid);
        if let Some(d) = self.cache.get(&key) {
            return Some(d.clone());
        }
        let default = self.data.default.clone();
        let lines = Rc::new(build_detail(self.repo, &b, default.as_ref(), self.protected(&b)));
        self.cache.insert(key, lines.clone());
        Some(lines)
    }
}

/// Local vs. a remote copy, from (ahead, behind): "up to date", "2 to push", "1 to pull".
fn sync_note((ahead, behind): (usize, usize)) -> String {
    match (ahead, behind) {
        (0, 0) => "up to date".into(),
        (a, 0) => format!("{a} to push"),
        (0, b) => format!("{b} to pull"),
        (a, b) => format!("differs: {a} to push, {b} to pull"),
    }
}

fn first_lines(s: &str) -> String {
    s.lines().filter(|l| !l.trim().is_empty()).take(2).collect::<Vec<_>>().join(" · ")
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
    let (list, details) = if !app.details {
        (body, None)
    } else if body.width >= 110 {
        let [l, _, r] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Length(1), Constraint::Min(30)]).areas(body);
        (l, Some(r))
    } else {
        let [t, _, b] =
            Layout::vertical([Constraint::Percentage(50), Constraint::Length(1), Constraint::Min(5)]).areas(body);
        (t, Some(b))
    };
    draw_list(f, app, list);
    if let Some(d) = details {
        let block = widgets::panel(theme::PANEL);
        match app.detail_lines() {
            Some(lines) => f.render_widget(Paragraph::new((*lines).clone()).wrap(Wrap { trim: false }).block(block), d),
            None => f.render_widget(block, d),
        }
    }
    draw_footer(f, app, footer);

    if let Some(p) = &app.palette {
        p.draw(f, full);
    }
    if let Some((c, _)) = &app.confirm {
        c.draw(f, full);
    }
    if let Some(p) = &app.pick {
        p.draw(f, full);
    }
    if let Some(t) = &app.toast {
        t.draw(f, full);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let local: Vec<&Branch> = app.data.list.iter().filter(|b| !b.remote).collect();
    let merged = local.iter().filter(|b| b.merged).count();
    let gone = local.iter().filter(|b| b.gone()).count();
    let mut spans = vec![
        Span::styled("branches", Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.repo_name.clone(), theme::bold()),
        Span::raw("  "),
        Span::styled(format!("{} local", local.len()), theme::muted()),
    ];
    if merged > 0 {
        spans.push(Span::styled(format!(" · {merged} merged"), theme::muted()));
    }
    if gone > 0 {
        spans.push(Span::styled(format!(" · {gone} gone"), Style::new().fg(theme::RED)));
    }
    if app.remotes {
        spans.push(Span::styled("  · with remotes", theme::faint()));
    }
    f.render_widget(Paragraph::new(widgets::truncate(Line::from(spans), area.width)), area);
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
    if app.visible.is_empty() {
        let msg = if app.query.is_empty() { "No branches" } else { "No branches match" };
        f.render_widget(Paragraph::new(Span::styled(format!("  {msg}"), theme::muted())), inner);
        return;
    }

    let now = crate::core::now();
    for (row, &i) in app.visible.iter().enumerate().skip(app.scroll).take(height) {
        let b = &app.data.list[i];
        let r = Rect { y: inner.y + (row - app.scroll) as u16, height: 1, ..inner };
        let selected = row == app.selected;
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };

        let bar = if selected { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };
        let mark = if app.marked.contains(&b.name) {
            Span::styled("✓ ", Style::new().fg(theme::ACCENT))
        } else {
            Span::raw("  ")
        };
        let dot = if b.current { Span::styled("● ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };
        let name_style = if b.current {
            Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
        } else if b.remote || b.merged || b.gone() {
            theme::muted()
        } else {
            theme::text()
        };
        let left = Line::from(vec![bar, mark, dot, Span::styled(b.name.clone(), name_style)]);

        let mut track: Vec<Span> = Vec::new();
        match &b.upstream {
            _ if b.gone() => track.push(Span::styled("gone", Style::new().fg(theme::RED))),
            Some(up) if up.ahead == 0 && up.behind == 0 => track.push(Span::styled("=", theme::faint())),
            Some(up) => {
                if up.ahead > 0 {
                    track.push(Span::styled(format!("↑{}", up.ahead), Style::new().fg(theme::GREEN)));
                }
                if up.behind > 0 {
                    if !track.is_empty() {
                        track.push(Span::raw(" "));
                    }
                    track.push(Span::styled(format!("↓{}", up.behind), Style::new().fg(theme::RED)));
                }
            }
            None if b.remote => track.push(Span::styled("remote", theme::faint())),
            None => {}
        }
        let track_w = Line::from(track.clone()).width();
        // On several remotes, or on one it doesn't track: "2r".
        let on = b.copies.len();
        let show = on > 1 || (on == 1 && (b.upstream.is_none() || b.gone()));
        let mut right = vec![Span::styled(if show { format!("  {on}r") } else { "    ".into() }, theme::faint())];
        right.push(Span::raw(" ".repeat(9usize.saturating_sub(track_w))));
        right.extend(track);
        right.push(Span::styled(if b.merged { "   merged" } else { "         " }, theme::faint()));
        right.push(Span::styled(format!("{:>9}", relative_time(b.time, now)), theme::faint()));
        right.push(Span::raw("  "));
        let right = Line::from(right);

        let [l, rr] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(r);
        f.render_widget(Paragraph::new(widgets::truncate(left, l.width)).style(Style::new().bg(bg)), l);
        f.render_widget(Paragraph::new(right).style(Style::new().bg(bg)), rr);
    }
}

fn build_detail(
    repo: &gix::Repository,
    b: &Branch,
    default: Option<&(String, gix::ObjectId)>,
    protected: bool,
) -> Vec<Line<'static>> {
    let now = crate::core::now();
    let label = |s: &str| Span::styled(format!("{s:<10}"), theme::faint());
    let mut badges = vec![Span::styled(
        b.name.clone(),
        if b.current { Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD) } else { theme::bold() },
    )];
    for (on, text, color) in [
        (b.current, "checked out", theme::ACCENT),
        (b.remote, "remote", theme::MUTED),
        (b.merged, "merged", theme::GREEN),
        (b.gone(), "gone", theme::RED),
    ] {
        if on {
            badges.push(Span::raw("  "));
            badges.push(Span::styled(format!(" {text} "), Style::new().bg(theme::ELEMENT).fg(color)));
        }
    }
    let mut out = vec![
        Line::from(badges),
        Line::raw(""),
        Line::from(vec![
            Span::styled(b.id.clone(), Style::new().fg(theme::ACCENT)),
            Span::styled("  ·  ", theme::faint()),
            Span::styled(b.author.clone(), theme::muted()),
            Span::styled("  ·  ", theme::faint()),
            Span::styled(relative_time(b.time, now), theme::muted()),
        ]),
        Line::styled(b.summary.clone(), theme::text()),
        Line::raw(""),
    ];

    // Upstream.
    if !b.remote {
        let up: Vec<Span> = match &b.upstream {
            None if b.copies.is_empty() => vec![Span::styled("none (never pushed)", theme::muted())],
            None => vec![Span::styled("none (not tracking a remote branch)", theme::muted())],
            Some(u) if b.gone() => vec![
                Span::styled(u.name.trim_end_matches(" (gone)").to_string(), theme::text()),
                Span::styled("  deleted on the remote", Style::new().fg(theme::RED)),
            ],
            Some(u) => {
                let mut s = vec![Span::styled(u.name.clone(), theme::text()), Span::raw("  ")];
                match (u.ahead, u.behind) {
                    (0, 0) => s.push(Span::styled("up to date", theme::muted())),
                    (a, d) => {
                        if a > 0 {
                            s.push(Span::styled(format!("{a} to push "), Style::new().fg(theme::GREEN)));
                        }
                        if d > 0 {
                            s.push(Span::styled(format!("{d} to pull"), Style::new().fg(theme::RED)));
                        }
                    }
                }
                s
            }
        };
        let mut line = vec![label("upstream")];
        line.extend(up);
        out.push(Line::from(line));

        // Every remote that has it, when that says more than the upstream row.
        if b.copies.len() > 1 || (b.copies.len() == 1 && (b.upstream.is_none() || b.gone())) {
            let w = b.copies.iter().map(|c| c.remote.chars().count() + c.branch.chars().count() + 1).max().unwrap_or(0);
            for (i, c) in b.copies.iter().enumerate() {
                let full = format!("{}/{}", c.remote, c.branch);
                let pad = " ".repeat(w - full.chars().count() + 2);
                let (ahead, behind) = branches::ahead_behind(repo, b.oid, c.oid);
                let color = match (ahead, behind) {
                    (0, 0) => theme::MUTED,
                    (_, 0) => theme::GREEN,
                    (0, _) => theme::RED,
                    _ => theme::YELLOW,
                };
                out.push(Line::from(vec![
                    if i == 0 { label("remotes") } else { label("") },
                    Span::styled(full + &pad, theme::text()),
                    Span::styled(sync_note((ahead, behind)), Style::new().fg(color)),
                ]));
            }
        }
    }

    // Relation to main.
    let Some((main, main_id)) = default else {
        out.push(Line::from(vec![label("main"), Span::styled("no main branch found", theme::muted())]));
        return out;
    };
    if !b.remote && b.name == *main {
        out.push(Line::from(vec![label("main"), Span::styled("this is the main branch", theme::muted())]));
        return out;
    }
    let d = branches::detail(repo, b, Some(*main_id)).ok();
    let state = match &d {
        Some(d) if d.unique_total == 0 => Span::styled(
            if protected { "merged".to_string() } else { format!("merged into {main}, safe to delete") },
            Style::new().fg(theme::GREEN),
        ),
        Some(d) => Span::styled(
            format!("{} commit{} not in {main}", d.unique_total, if d.unique_total == 1 { "" } else { "s" }),
            Style::new().fg(theme::YELLOW),
        ),
        None => Span::styled("couldn't compare", theme::muted()),
    };
    out.push(Line::from(vec![label(main), state]));
    if let Some(d) = &d {
        if let Some((id, time)) = &d.fork {
            out.push(Line::from(vec![
                label("forked"),
                Span::styled(format!("from {main} {}", relative_time(*time, now)), theme::text()),
                Span::styled(format!("  {id}"), theme::faint()),
            ]));
        }
        if d.unique_total > 0 {
            out.push(Line::from(vec![
                label("changes"),
                Span::styled(format!("+{} ", d.added), Style::new().fg(theme::GREEN)),
                Span::styled(format!("−{}", d.removed), Style::new().fg(theme::RED)),
                Span::styled(format!("  in {} file{}", d.files, if d.files == 1 { "" } else { "s" }), theme::muted()),
            ]));
            out.push(Line::raw(""));
            out.push(Line::styled(format!("Commits not in {main}"), theme::faint()));
            for (id, subject) in &d.unique {
                out.push(Line::from(vec![
                    Span::styled(format!("{id}  "), theme::faint()),
                    Span::styled(subject.clone(), theme::text()),
                ]));
            }
            if d.unique_total > d.unique.len() {
                out.push(Line::styled(format!("…and {} more", d.unique_total - d.unique.len()), theme::faint()));
            }
        }
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
        spans.push(Span::styled(format!("   {} of {}", app.visible.len(), app.data.list.len()), theme::muted()));
        Line::from(spans)
    } else if !app.marked.is_empty() {
        Line::styled(format!("✓ {} selected", app.marked.len()), Style::new().fg(theme::ACCENT))
    } else {
        Line::styled(
            format!("{} branch{}", app.visible.len(), if app.visible.len() == 1 { "" } else { "es" }),
            theme::muted(),
        )
    };
    let right = if app.searching {
        widgets::hints(&[("enter", "keep filter"), ("esc", "clear")])
    } else if !app.marked.is_empty() {
        widgets::hints(&[("space", "select"), ("d", "delete"), ("D", "delete on remote"), ("esc", "clear")])
    } else {
        widgets::hints(&[
            ("enter", "checkout"),
            ("space", "select"),
            ("d", "delete"),
            ("ctrl+p", "commands"),
            ("q", "quit"),
        ])
    };
    let [l, r] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(widgets::truncate(left, l.width)), l);
    f.render_widget(Paragraph::new(right), r);
}

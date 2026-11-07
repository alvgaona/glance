//! Full-screen editor for the rebase todo list.

use std::collections::HashMap;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph, Wrap},
};

use crate::fmt::{now, relative_time};
use crate::git::commit::{self, Details};
use crate::git::diff::Size;
use crate::git::todo::{self, Action, Line as Todo};
use crate::tui::app::{self, Flow, Screen};
use crate::tui::kit::sidebar::{self, Section};
use crate::tui::kit::{self, Command, Confirm, Palette, PaletteEvent, Scroll, Toast, footer, list};
use crate::tui::panels::diff;
use crate::tui::theme;

/// What the editor shows at the top.
pub struct Header {
    pub branch: String,
    /// "3 commits since branching off main", "onto 1a2b3c4"...
    pub detail: String,
}

pub enum Outcome {
    Apply(Vec<Todo>),
    Quit,
}

#[derive(Clone)]
enum Cmd {
    Set(Action),
    MoveUp,
    MoveDown,
    Flip,
    Undo,
    Details,
    Side,
    Apply,
    Abort,
}

struct App<'r> {
    repo: &'r gix::Repository,
    /// `lines` and `original` are kept in display order; git wants oldest first.
    lines: Vec<Todo>,
    original: Vec<Todo>,
    newest_first: bool,
    selected: usize,
    /// First visible row of the list.
    scroll: usize,
    details: bool,
    header: Header,
    /// Commit details by todo hash; `None` if it couldn't be read.
    info: HashMap<String, Option<Details>>,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    confirm_abort: Option<Confirm>,
    /// Where the rows were last drawn, for mouse clicks.
    list_area: Rect,
    /// The sidebar, on wide screens. With it open, the details pane is the diff.
    side: bool,
    side_open: bool,
    /// Diffs by (commit, pane width), filled in `tick`.
    diffs: HashMap<(String, u16), diff::Patches>,
    diff_width: u16,
    /// The sidebar's file section, and the file on each of its rows ("also in" rows
    /// have none), for clicks.
    files_area: Rect,
    file_rows: Vec<Option<usize>>,
    detail_scroll: Scroll,
    /// The commit the diff pane is scrolled for: another one starts at the top.
    scrolled_for: Option<String>,
    detail_area: Rect,
}

/// `lines` come in git's order (oldest first); the result is in git's order too.
pub fn run(repo: &gix::Repository, lines: Vec<Todo>, header: Header, newest_first: bool) -> Result<Outcome> {
    let mut app = start(repo, lines, header, newest_first);
    app.side = crate::tui::state::sidebar();
    let out = app::run(&mut app)?;
    crate::tui::state::set_sidebar(app.side);
    Ok(out)
}

/// The screen as it opens, before the first draw.
fn start(repo: &gix::Repository, mut lines: Vec<Todo>, header: Header, newest_first: bool) -> App<'_> {
    // Commit details up front: rebases are short, and it keeps drawing simple.
    let info = lines
        .iter()
        .filter_map(|l| match l {
            Todo::Commit { hash, .. } => Some(hash.clone()),
            Todo::Other(_) => None,
        })
        .take(500)
        .map(|h| {
            let shown = commit::read(repo, &h).ok();
            (h, shown)
        })
        .collect();
    if newest_first {
        lines.reverse();
    }

    App {
        repo,
        original: lines.clone(),
        lines,
        newest_first,
        selected: 0,
        scroll: 0,
        details: true,
        header,
        info,
        toast: None,
        palette: None,
        confirm_abort: None,
        list_area: Rect::default(),
        side: true,
        side_open: false,
        diffs: HashMap::new(),
        diff_width: 0,
        files_area: Rect::default(),
        file_rows: Vec::new(),
        detail_scroll: Scroll::default(),
        scrolled_for: None,
        detail_area: Rect::default(),
    }
}

impl Screen for App<'_> {
    type Out = Outcome;
    type Effect = std::convert::Infallible;

    fn draw(&mut self, f: &mut Frame) {
        draw(f, self)
    }
    fn apply(&mut self, e: Self::Effect) -> Result<Flow<Self::Out, Self::Effect>> {
        match e {}
    }
    fn key(&mut self, k: KeyEvent) -> Flow<Outcome, Self::Effect> {
        self.on_key(k).into()
    }
    fn mouse(&mut self, m: MouseEvent) {
        self.on_mouse(m)
    }
    fn tick(&mut self) -> Result<()> {
        // The diff pane only shows with the sidebar; zero until the first draw.
        if !self.side_open || self.diff_width == 0 {
            return Ok(());
        }
        let Some(Todo::Commit { hash, .. }) = self.lines.get(self.selected) else { return Ok(()) };
        let key = (hash.clone(), self.diff_width);
        if !self.diffs.contains_key(&key) {
            let p = match self.info.get(hash).and_then(|i| i.as_ref()) {
                Some(d) => diff::patches(self.repo, &d.files),
                None => {
                    let lines = vec![Line::styled("Couldn't read this commit.", theme::muted())];
                    diff::Patches { lines, ..Default::default() }
                }
            };
            self.diffs.insert(key, p);
        }
        Ok(())
    }
    fn toast(&mut self) -> &mut Option<Toast> {
        &mut self.toast
    }
}

fn commands() -> Vec<Command<Cmd>> {
    vec![
        Command { label: "Pick: keep the commit", keys: "p", action: Cmd::Set(Action::Pick) },
        Command { label: "Reword: change its message", keys: "r", action: Cmd::Set(Action::Reword) },
        Command { label: "Edit: stop here to amend", keys: "e", action: Cmd::Set(Action::Edit) },
        Command { label: "Squash into the older commit", keys: "s", action: Cmd::Set(Action::Squash) },
        Command { label: "Fixup into the older commit, drop message", keys: "f", action: Cmd::Set(Action::Fixup) },
        Command { label: "Drop the commit", keys: "d", action: Cmd::Set(Action::Drop) },
        Command { label: "Move up", keys: "shift+↑", action: Cmd::MoveUp },
        Command { label: "Move down", keys: "shift+↓", action: Cmd::MoveDown },
        Command { label: "Flip order: newest or oldest first", keys: "o", action: Cmd::Flip },
        Command { label: "Sidebar: show or hide (wide screens)", keys: "i", action: Cmd::Side },
        Command { label: "Undo all changes", keys: "u", action: Cmd::Undo },
        Command { label: "Show or hide details", keys: "tab", action: Cmd::Details },
        Command { label: "Apply and run the rebase", keys: "ctrl+s", action: Cmd::Apply },
        Command { label: "Abort the rebase", keys: "esc", action: Cmd::Abort },
    ]
}

impl App<'_> {
    fn last(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }

    fn select(&mut self, i: usize) {
        self.selected = i.min(self.last());
    }

    fn modified(&self) -> bool {
        self.lines != self.original
    }

    fn on_key(&mut self, k: KeyEvent) -> Option<Outcome> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let alt = k.modifiers.contains(KeyModifiers::ALT);

        if ctrl && k.code == KeyCode::Char('c') {
            return Some(Outcome::Quit);
        }
        if self.confirm_abort.is_some() {
            return match Confirm::answer(&k) {
                Some(true) => Some(Outcome::Quit),
                Some(false) => {
                    self.confirm_abort = None;
                    None
                }
                None => None,
            };
        }
        if let Some(p) = &mut self.palette {
            match p.key(k)? {
                PaletteEvent::Close => self.palette = None,
                PaletteEvent::Run(cmd) => {
                    self.palette = None;
                    return self.run(cmd);
                }
            }
            return None;
        }

        // J/K move commits here, so the diff scrolls with ctrl+d/u and ← → only.
        if ctrl && matches!(k.code, KeyCode::Char('d' | 'u')) || matches!(k.code, KeyCode::Left | KeyCode::Right) {
            self.detail_scroll.key(k, self.detail_area.height);
            return None;
        }
        let i = self.selected;
        let cmd = match k.code {
            KeyCode::Char('p') if ctrl => {
                self.palette = Some(Palette::new(commands()));
                return None;
            }
            KeyCode::Char(':') => {
                self.palette = Some(Palette::new(commands()));
                return None;
            }
            KeyCode::Char('s') if ctrl => Cmd::Apply,
            KeyCode::Char('w') | KeyCode::Enter if !shift => Cmd::Apply,
            KeyCode::Esc | KeyCode::Char('q') => Cmd::Abort,

            KeyCode::Up if shift || alt => Cmd::MoveUp,
            KeyCode::Down if shift || alt => Cmd::MoveDown,
            KeyCode::Char('K') => Cmd::MoveUp,
            KeyCode::Char('J') => Cmd::MoveDown,

            KeyCode::Up | KeyCode::Char('k') => {
                self.select(i.saturating_sub(1));
                return None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.select(i + 1);
                return None;
            }
            KeyCode::PageUp => {
                self.select(i.saturating_sub(10));
                return None;
            }
            KeyCode::PageDown => {
                self.select(i + 10);
                return None;
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.select(0);
                return None;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.select(self.last());
                return None;
            }

            KeyCode::Char('o') => Cmd::Flip,
            KeyCode::Char('u') => Cmd::Undo,
            KeyCode::Tab => Cmd::Details,
            KeyCode::Char('i') => Cmd::Side,
            KeyCode::Char(c) => Cmd::Set(Action::ALL.into_iter().find(|a| a.key() == c)?),
            _ => return None,
        };
        self.run(cmd)
    }

    fn run(&mut self, cmd: Cmd) -> Option<Outcome> {
        let i = self.selected;
        match cmd {
            Cmd::Set(a) => {
                if let Some(Todo::Commit { action, .. }) = self.lines.get_mut(i) {
                    *action = a;
                    // Move on, so a run of commits can be marked quickly.
                    self.select(i + 1);
                }
            }
            Cmd::MoveUp if i > 0 => {
                self.lines.swap(i, i - 1);
                self.select(i - 1);
            }
            Cmd::MoveDown if i < self.last() => {
                self.lines.swap(i, i + 1);
                self.select(i + 1);
            }
            Cmd::MoveUp | Cmd::MoveDown => {}
            Cmd::Flip => {
                self.newest_first = !self.newest_first;
                self.lines.reverse();
                self.original.reverse();
                self.select(self.last() - i);
                let order = if self.newest_first { "Newest first" } else { "Oldest first" };
                self.toast = Some(Toast::new(order, theme::ACCENT));
            }
            Cmd::Undo => {
                self.lines = self.original.clone();
                self.toast = Some(Toast::new("Back to the original list", theme::ACCENT));
            }
            Cmd::Details => self.details = !self.details,
            Cmd::Side => self.side = !self.side,
            Cmd::Apply => {
                let mut lines = self.lines.clone();
                if self.newest_first {
                    lines.reverse();
                }
                if let Some(p) = todo::problem(&lines) {
                    self.toast = Some(Toast::new(format!("Can't apply: {p}"), theme::RED));
                    return None;
                }
                return Some(Outcome::Apply(lines));
            }
            Cmd::Abort => {
                if !self.modified() {
                    return Some(Outcome::Quit);
                }
                self.confirm_abort = Some(Confirm {
                    title: "Abort the rebase?".into(),
                    body: "Your edits to this list are thrown away. The branch stays exactly as it was.".into(),
                    yes: "Abort".into(),
                    no: "Keep editing".into(),
                    danger: true,
                });
            }
        }
        None
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        if self.palette.is_some() || self.confirm_abort.is_some() {
            return;
        }
        let in_diff = self.side_open && list::inside(self.detail_area, &m);
        match m.kind {
            MouseEventKind::ScrollUp if in_diff => self.detail_scroll.wheel(false),
            MouseEventKind::ScrollDown if in_diff => self.detail_scroll.wheel(true),
            MouseEventKind::ScrollUp => self.select(self.selected.saturating_sub(1)),
            MouseEventKind::ScrollDown => self.select(self.selected + 1),
            MouseEventKind::Down(MouseButton::Left) if self.side_open && list::inside(self.files_area, &m) => {
                let row = (m.row - self.files_area.y) as usize;
                let Some(Some(file)) = self.file_rows.get(row) else { return };
                let Some(Todo::Commit { hash, .. }) = self.lines.get(self.selected) else { return };
                if let Some(&at) = self.diffs.get(&(hash.clone(), self.diff_width)).and_then(|d| d.files.get(*file)) {
                    self.detail_scroll.to(at);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(i) = list::row_at(self.list_area, self.scroll, &m) {
                    self.select(i);
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

    // Whole screen, with a one-column gutter so text doesn't touch the edges.
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

    let (list, details, side) = if !app.details {
        (body, None, None)
    } else {
        let p = kit::panes(body, Some(list_width(app)), app.lines.len() as u16 + 2, app.side);
        (p.list, Some(p.main), p.side)
    };
    draw_list(f, app, list);
    app.side_open = side.is_some();
    match (details, side) {
        // Wide: the message and files on the side, the diff in the middle.
        (Some(main), Some(side)) => {
            (app.files_area, app.file_rows) = draw_side(f, app, side);
            draw_diff(f, app, main);
        }
        (Some(d), None) => draw_details(f, app, d),
        _ => {}
    }
    draw_footer(f, app, footer);

    if let Some(p) = &app.palette {
        p.draw(f, full);
    }
    if let Some(c) = &app.confirm_abort {
        c.draw(f, full);
    }
    if let Some(t) = &app.toast {
        t.draw(f, full);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let left = Line::from(vec![
        Span::styled("rebase", Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(app.header.branch.clone(), theme::bold()),
        Span::raw("  "),
        Span::styled(app.header.detail.clone(), theme::muted()),
    ]);
    let mut right = vec![Span::styled(if app.newest_first { "newest first" } else { "oldest first" }, theme::faint())];
    if app.modified() {
        right.push(Span::raw("   "));
        right.push(Span::styled("● ", Style::new().fg(theme::YELLOW)));
        right.push(Span::styled("modified", theme::muted()));
    }
    let right = Line::from(right);
    let [l, _, r] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(3), Constraint::Length(right.width() as u16)])
            .areas(area);
    f.render_widget(Paragraph::new(kit::truncate(left, l.width)), l);
    f.render_widget(Paragraph::new(right), r);
}

fn action_colors(a: Action) -> (Color, Color) {
    match a {
        Action::Pick => (theme::BORDER, theme::MUTED),
        Action::Reword => (theme::BLUE, theme::BG),
        Action::Edit => (theme::YELLOW, theme::BG),
        Action::Squash => (theme::PURPLE, theme::BG),
        Action::Fixup => (Color::Rgb(108, 88, 150), theme::TEXT),
        Action::Drop => (theme::RED, theme::BG),
    }
}

/// The list's width that fits its widest row: bar, action pill, hash, subject, and the
/// `+a −d` on the right, as `draw_list` lays them out.
fn list_width(app: &App) -> u16 {
    let widest = app
        .lines
        .iter()
        .map(|l| match l {
            Todo::Commit { hash, subject, .. } => hash.len() + 2 + 2 + crate::fmt::width(subject) + 16,
            Todo::Other(raw) => crate::fmt::width(raw),
        })
        .max()
        .unwrap_or(0);
    (2 + 8 + 2 + widest).min(u16::MAX as usize) as u16
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().style(Style::new().bg(theme::PANEL)).padding(Padding::new(0, 0, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;

    let height = inner.height as usize;
    app.scroll = list::follow(app.scroll, app.selected, app.selected, height);

    // Squash/fixup fold into the older commit: above when oldest-first, below when newest-first.
    let meld = if app.newest_first { "↓ " } else { "↑ " };

    for (row, line) in app.lines.iter().enumerate().skip(app.scroll).take(height) {
        let y = inner.y + (row - app.scroll) as u16;
        let r = Rect { y, height: 1, ..inner };
        let selected = row == app.selected;
        let bg = if selected { theme::ELEMENT } else { theme::PANEL };
        let bar = if selected { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };

        let (left, right): (Vec<Span>, Vec<Span>) = match line {
            Todo::Commit { action, hash, subject } => {
                let (pbg, pfg) = action_colors(*action);
                let mut subject_style = theme::text();
                if *action == Action::Drop {
                    subject_style = theme::faint().add_modifier(Modifier::CROSSED_OUT);
                } else if action.melds() {
                    subject_style = theme::muted();
                }
                let left = vec![
                    bar,
                    kit::pill(&format!("{:<6}", action.word()), pbg, pfg),
                    Span::raw("  "),
                    Span::styled(hash.clone(), theme::faint()),
                    Span::raw("  "),
                    Span::styled(if action.melds() { meld } else { "" }, Style::new().fg(theme::PURPLE)),
                    Span::styled(subject.clone(), subject_style),
                ];
                let right = match app.info.get(hash).and_then(|i| i.as_ref()).map(totals) {
                    Some((a, d)) => vec![
                        Span::raw("  "),
                        Span::styled(format!("+{a}"), Style::new().fg(theme::GREEN)),
                        Span::raw(" "),
                        Span::styled(format!("−{d}"), Style::new().fg(theme::RED)),
                        Span::raw("  "),
                    ],
                    None => Vec::new(),
                };
                (left, right)
            }
            Todo::Other(raw) => (
                vec![
                    bar,
                    kit::pill("  ··  ", theme::ELEMENT, theme::MUTED),
                    Span::raw("  "),
                    Span::styled(raw.clone(), theme::muted()),
                ],
                Vec::new(),
            ),
        };

        let right = Line::from(right);
        let [l, rr] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(r);
        f.render_widget(Paragraph::new(kit::truncate(Line::from(left), l.width)).style(Style::new().bg(bg)), l);
        f.render_widget(Paragraph::new(right).style(Style::new().bg(bg)), rr);
    }
}

fn draw_details(f: &mut Frame, app: &App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let text: Vec<Line> = match app.lines.get(app.selected) {
        Some(Todo::Commit { hash, action, .. }) => match app.info.get(hash).and_then(|i| i.as_ref()) {
            Some(s) => details(s, *action, app.newest_first),
            None => vec![Line::styled("Couldn't read this commit.", theme::muted())],
        },
        Some(Todo::Other(raw)) => vec![
            Line::styled("Not a commit", theme::bold()),
            Line::raw(""),
            Line::styled("Git runs this line as written:", theme::muted()),
            Line::raw(""),
            Line::styled(raw.clone(), theme::text()),
        ],
        None => Vec::new(),
    };
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).block(block), area);
}

fn details(s: &Details, action: Action, newest_first: bool) -> Vec<Line<'static>> {
    let mut out = vec![
        Line::from(vec![
            Span::styled(s.id.clone(), Style::new().fg(theme::ACCENT)),
            Span::styled("  ·  ", theme::faint()),
            Span::styled(s.author.clone(), theme::muted()),
            Span::styled("  ·  ", theme::faint()),
            Span::styled(relative_time(s.time, now()), theme::muted()),
        ]),
        Line::raw(""),
        Line::styled(s.subject.clone(), theme::bold()),
    ];
    if !s.body.is_empty() {
        out.push(Line::raw(""));
        out.extend(s.body.lines().map(|l| Line::styled(l.to_string(), theme::muted())));
    }

    if let Some((text, color)) = action_note(action, newest_first) {
        out.push(Line::raw(""));
        out.push(Line::from(vec![Span::styled("● ", Style::new().fg(color)), Span::styled(text, theme::text())]));
    }

    out.push(Line::raw(""));
    let n = s.files.len();
    out.push(Line::styled(format!("{n} file{}", if n == 1 { "" } else { "s" }), theme::faint()));
    out.extend(s.files.iter().map(file_row));
    out
}

/// What the chosen action will do, in words.
fn action_note(action: Action, newest_first: bool) -> Option<(String, Color)> {
    let older = if newest_first { "below" } else { "above" };
    match action {
        Action::Pick => None,
        Action::Reword => Some(("Git will ask for a new message.".to_string(), theme::BLUE)),
        Action::Edit => Some(("The rebase stops here so you can amend.".to_string(), theme::YELLOW)),
        Action::Squash => Some((format!("Folded into the commit {older}; messages combined."), theme::PURPLE)),
        Action::Fixup => Some((format!("Folded into the commit {older}; this message is dropped."), theme::PURPLE)),
        Action::Drop => Some(("Removed from the branch.".to_string(), theme::RED)),
    }
}

/// `M  src/app.rs  +5 −1`.
fn file_row(file: &crate::git::tree::FileStat) -> Line<'static> {
    crate::tui::panels::changes::Item::stat(file).row()
}

/// Other commits in the list that change `path`, as `abc1234 ↑2`: how many rows above
/// or below. Moving a commit past one of them is where conflicts come from.
fn also_in(app: &App, path: &str) -> Vec<String> {
    let at = app.selected;
    let touches = |hash: &String| {
        let files = app.info.get(hash).and_then(|i| i.as_ref()).map(|d| d.files.as_slice()).unwrap_or_default();
        files.iter().any(|f| f.path == path || f.from.as_deref() == Some(path))
    };
    app.lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| match l {
            Todo::Commit { hash, action, .. } if i != at && *action != Action::Drop && touches(hash) => {
                let (arrow, n) = if i < at { ("↑", at - i) } else { ("↓", i - at) };
                Some(format!("{} {arrow}{n}", &hash[..7.min(hash.len())]))
            }
            _ => None,
        })
        .collect()
}

/// The sidebar: the commit's message and what its action does, then its files, each
/// with the other commits in the list that change it too. Gives where the files are
/// and the file on each row.
fn draw_side(f: &mut Frame, app: &App, area: Rect) -> (Rect, Vec<Option<usize>>) {
    let w = (area.width as usize).saturating_sub(4);
    let Some(Todo::Commit { hash, action, .. }) = app.lines.get(app.selected) else {
        sidebar::draw(f, area, vec![Section { title: "", count: None, lines: Vec::new() }]);
        return Default::default();
    };
    let Some(d) = app.info.get(hash).and_then(|i| i.as_ref()) else {
        let lines = vec![Line::styled("Couldn't read this commit.", theme::muted())];
        sidebar::draw(f, area, vec![Section { title: "", count: None, lines }]);
        return Default::default();
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(d.id.clone(), Style::new().fg(theme::ACCENT)),
            Span::styled(format!("  {}", d.author), theme::muted()),
        ]),
        Line::styled(relative_time(d.time, now()), theme::faint()),
        Line::raw(""),
    ];
    lines.extend(kit::wrap(&d.subject, w).into_iter().map(|l| Line::styled(l, theme::bold())));
    if !d.body.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(kit::reflow(&d.body, w).into_iter().map(|l| Line::styled(l, theme::muted())));
    }
    if let Some((text, color)) = action_note(*action, app.newest_first) {
        lines.push(Line::raw(""));
        let mut note = kit::wrap(&text, w.saturating_sub(2)).into_iter();
        let first = note.next().unwrap_or_default();
        lines.push(Line::from(vec![Span::styled("● ", Style::new().fg(color)), Span::styled(first, theme::text())]));
        lines.extend(note.map(|l| Line::styled(format!("  {l}"), theme::text())));
    }

    let (mut files, mut rows) = (Vec::new(), Vec::new());
    for (i, file) in d.files.iter().enumerate() {
        files.push(file_row(file));
        rows.push(Some(i));
        let others = also_in(app, &file.path);
        if !others.is_empty() {
            rows.push(None);
            files.push(Line::styled(format!("   also in {}", others.join(", ")), Style::new().fg(theme::YELLOW)));
        }
    }
    let sections = vec![
        Section { title: "", count: None, lines },
        Section { title: "files", count: Some(d.files.len()), lines: files },
    ];
    (sidebar::draw(f, area, sections)[1], rows)
}

/// The selected commit's diff, scrolled with ctrl+d/u or the wheel.
fn draw_diff(f: &mut Frame, app: &mut App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.detail_area = area;
    app.diff_width = inner.width;
    let Some(Todo::Commit { hash, .. }) = app.lines.get(app.selected) else { return };
    if app.scrolled_for.as_ref() != Some(hash) {
        app.scrolled_for = Some(hash.clone());
        app.detail_scroll.reset();
    }
    let Some(p) = app.diffs.get(&(hash.clone(), inner.width)) else { return };
    f.render_widget(Paragraph::new(app.detail_scroll.view(&p.lines, &p.pins, inner)), inner);
    app.detail_scroll.draw_more_keys(f, area, "ctrl+d/u");
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    // Left: what the list will do, e.g. "5 commits  ● 1 fixup  ● 1 drop".
    let commits = app.lines.iter().filter(|l| matches!(l, Todo::Commit { .. })).count();
    let mut left =
        vec![Span::styled(format!("{commits} commit{}", if commits == 1 { "" } else { "s" }), theme::muted())];
    for a in Action::ALL.into_iter().filter(|a| *a != Action::Pick) {
        let n = app.lines.iter().filter(|l| matches!(l, Todo::Commit { action, .. } if *action == a)).count();
        if n > 0 {
            let (color, _) = action_colors(a);
            left.push(Span::raw("   "));
            left.push(Span::styled("● ", Style::new().fg(color)));
            left.push(Span::styled(format!("{n} {}", a.word()), theme::muted()));
        }
    }
    let mut pairs = vec![("ctrl+p", "commands"), ("w", "apply"), ("esc", "abort")];
    if app.detail_scroll.wide() {
        pairs.insert(1, ("← →", "long lines"));
    }
    if app.details {
        footer::sidebar(&mut pairs, area.width);
    }
    footer::draw(f, area, Line::from(left), pairs);
}

fn totals(s: &Details) -> (u32, u32) {
    let (mut a, mut r) = (0, 0);
    for file in &s.files {
        if let Some(Size::Lines { added, removed }) = file.size {
            a += added;
            r += removed;
        }
    }
    (a, r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::run::Scratch;
    use crate::tui::app::press;

    /// Three commits, oldest first: the first and last change a.txt.
    fn screen<'r>(s: &Scratch, repo: &'r gix::Repository) -> App<'r> {
        s.commit("a.txt", "1\n");
        s.commit("b.txt", "b\n");
        s.commit("a.txt", "2\n");
        let ids = |rev: &str| repo.rev_parse_single(rev).unwrap().to_string()[..7].to_string();
        let lines = ["HEAD~2", "HEAD~1", "HEAD"]
            .map(|r| Todo::Commit { action: Action::Pick, hash: ids(r), subject: r.into() })
            .to_vec();
        start(repo, lines, Header { branch: "main".into(), detail: String::new() }, false)
    }

    #[test]
    fn files_say_which_other_commits_change_them() {
        let s = Scratch::new("rebase-overlap");
        let repo = s.repo();
        let mut app = screen(&s, &repo);
        let last = match &app.lines[2] {
            Todo::Commit { hash, .. } => hash.clone(),
            Todo::Other(_) => unreachable!(),
        };
        assert_eq!(also_in(&app, "a.txt"), [format!("{last} ↓2")]);
        // From the middle commit, b.txt is only its own.
        app.selected = 1;
        assert!(also_in(&app, "b.txt").is_empty());
        app.selected = 0;
        // A dropped commit no longer counts.
        app.selected = 2;
        app.key(press(KeyCode::Char('d')));
        app.selected = 0;
        assert!(also_in(&app, "a.txt").is_empty());
    }

    #[test]
    fn wide_screens_show_the_diff_with_the_message_on_the_side() {
        let s = Scratch::new("rebase-side");
        let repo = s.repo();
        let mut app = screen(&s, &repo);
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 24)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        app.tick().unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        let text: String = t.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(app.side_open && text.contains("files   1") && text.contains("also in"), "{text}");
        assert!(app.diffs.keys().any(|(_, w)| *w == app.diff_width));
        // The file's row, then its "also in" row, which goes nowhere.
        assert_eq!(app.file_rows, [Some(0), None]);
        app.key(press(KeyCode::Char('i')));
        t.draw(|f| app.draw(f)).unwrap();
        assert!(!app.side_open);
    }
}

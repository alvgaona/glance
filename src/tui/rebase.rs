//! Full-screen editor for the rebase todo list.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph, Wrap},
};

use super::widgets::{self, Command, Confirm, Palette, PaletteEvent, Toast};
use super::{enter, leave, theme};
use crate::core::diff::Size;
use crate::core::show::{self, Shown};
use crate::core::status::ChangeKind;
use crate::core::todo::{self, Action, Line as Todo};
use crate::render::relative_time;

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
    Apply,
    Abort,
}

struct App {
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
    info: HashMap<String, Option<Shown>>,
    toast: Option<Toast>,
    palette: Option<Palette<Cmd>>,
    confirm_abort: Option<Confirm>,
    /// Where the rows were last drawn, for mouse clicks.
    list_area: Rect,
}

/// `lines` come in git's order (oldest first); the result is in git's order too.
pub fn run(repo: &gix::Repository, mut lines: Vec<Todo>, header: Header, newest_first: bool) -> Result<Outcome> {
    // Commit details up front: rebases are short, and it keeps drawing simple.
    let info = lines
        .iter()
        .filter_map(|l| match l {
            Todo::Commit { hash, .. } => Some(hash.clone()),
            Todo::Other(_) => None,
        })
        .take(500)
        .map(|h| {
            let shown = show::read(repo, &h).ok();
            (h, shown)
        })
        .collect();
    if newest_first {
        lines.reverse();
    }

    let mut app = App {
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
    };

    let mut terminal = enter();
    let result = (|| -> Result<Outcome> {
        loop {
            if app.toast.as_ref().is_some_and(Toast::expired) {
                app.toast = None;
            }
            terminal.draw(|f| draw(f, &mut app))?;
            // Wake up now and then so toasts disappear on their own.
            if !event::poll(Duration::from_millis(200))? {
                continue;
            }
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if let Some(outcome) = app.key(k) {
                        return Ok(outcome);
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
        Command { label: "Pick: keep the commit", keys: "p", action: Cmd::Set(Action::Pick) },
        Command { label: "Reword: change its message", keys: "r", action: Cmd::Set(Action::Reword) },
        Command { label: "Edit: stop here to amend", keys: "e", action: Cmd::Set(Action::Edit) },
        Command { label: "Squash into the older commit", keys: "s", action: Cmd::Set(Action::Squash) },
        Command { label: "Fixup into the older commit, drop message", keys: "f", action: Cmd::Set(Action::Fixup) },
        Command { label: "Drop the commit", keys: "d", action: Cmd::Set(Action::Drop) },
        Command { label: "Move up", keys: "shift+↑", action: Cmd::MoveUp },
        Command { label: "Move down", keys: "shift+↓", action: Cmd::MoveDown },
        Command { label: "Flip order: newest or oldest first", keys: "o", action: Cmd::Flip },
        Command { label: "Undo all changes", keys: "u", action: Cmd::Undo },
        Command { label: "Show or hide details", keys: "tab", action: Cmd::Details },
        Command { label: "Apply and run the rebase", keys: "ctrl+s", action: Cmd::Apply },
        Command { label: "Abort the rebase", keys: "esc", action: Cmd::Abort },
    ]
}

impl App {
    fn last(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }

    fn select(&mut self, i: usize) {
        self.selected = i.min(self.last());
    }

    fn modified(&self) -> bool {
        self.lines != self.original
    }

    fn key(&mut self, k: KeyEvent) -> Option<Outcome> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let alt = k.modifiers.contains(KeyModifiers::ALT);

        if ctrl && k.code == KeyCode::Char('c') {
            return Some(Outcome::Quit);
        }
        if self.confirm_abort.is_some() {
            return match k.code {
                KeyCode::Enter | KeyCode::Char('y') => Some(Outcome::Quit),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    self.confirm_abort = None;
                    None
                }
                _ => None,
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

    fn mouse(&mut self, m: crossterm::event::MouseEvent) {
        if self.palette.is_some() || self.confirm_abort.is_some() {
            return;
        }
        match m.kind {
            MouseEventKind::ScrollUp => self.select(self.selected.saturating_sub(1)),
            MouseEventKind::ScrollDown => self.select(self.selected + 1),
            MouseEventKind::Down(MouseButton::Left) => {
                let a = self.list_area;
                if m.column >= a.x && m.column < a.right() && m.row >= a.y && m.row < a.bottom() {
                    self.select(self.scroll + (m.row - a.y) as usize);
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

    let (list, details) = if !app.details {
        (body, None)
    } else if body.width >= 110 {
        let [l, _, r] =
            Layout::horizontal([Constraint::Percentage(58), Constraint::Length(1), Constraint::Min(30)]).areas(body);
        (l, Some(r))
    } else {
        // Stacked: the list takes only the rows it needs, details get the rest.
        let needed = app.lines.len() as u16 + 2;
        let list_h = needed.min(body.height * 3 / 5);
        let [t, _, b] =
            Layout::vertical([Constraint::Length(list_h), Constraint::Length(1), Constraint::Min(5)]).areas(body);
        (t, Some(b))
    };
    draw_list(f, app, list);
    if let Some(d) = details {
        draw_details(f, app, d);
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
    f.render_widget(Paragraph::new(widgets::truncate(left, l.width)), l);
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
                    widgets::pill(&format!("{:<6}", action.word()), pbg, pfg),
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
                    widgets::pill("  ··  ", theme::ELEMENT, theme::MUTED),
                    Span::raw("  "),
                    Span::styled(raw.clone(), theme::muted()),
                ],
                Vec::new(),
            ),
        };

        let right = Line::from(right);
        let [l, rr] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(r);
        f.render_widget(Paragraph::new(widgets::truncate(Line::from(left), l.width)).style(Style::new().bg(bg)), l);
        f.render_widget(Paragraph::new(right).style(Style::new().bg(bg)), rr);
    }
}

fn draw_details(f: &mut Frame, app: &App, area: Rect) {
    let block = widgets::panel(theme::PANEL);
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

fn details(s: &Shown, action: Action, newest_first: bool) -> Vec<Line<'static>> {
    let mut out = vec![
        Line::from(vec![
            Span::styled(s.id.clone(), Style::new().fg(theme::ACCENT)),
            Span::styled("  ·  ", theme::faint()),
            Span::styled(s.author.clone(), theme::muted()),
            Span::styled("  ·  ", theme::faint()),
            Span::styled(relative_time(s.time, crate::core::now()), theme::muted()),
        ]),
        Line::raw(""),
        Line::styled(s.subject.clone(), theme::bold()),
    ];
    if !s.body.is_empty() {
        out.push(Line::raw(""));
        out.extend(s.body.lines().map(|l| Line::styled(l.to_string(), theme::muted())));
    }

    // What the chosen action will do, in words.
    let older = if newest_first { "below" } else { "above" };
    let what = match action {
        Action::Pick => None,
        Action::Reword => Some(("Git will ask for a new message.".to_string(), theme::BLUE)),
        Action::Edit => Some(("The rebase stops here so you can amend.".to_string(), theme::YELLOW)),
        Action::Squash => Some((format!("Folded into the commit {older}; messages combined."), theme::PURPLE)),
        Action::Fixup => Some((format!("Folded into the commit {older}; this message is dropped."), theme::PURPLE)),
        Action::Drop => Some(("Removed from the branch.".to_string(), theme::RED)),
    };
    if let Some((text, color)) = what {
        out.push(Line::raw(""));
        out.push(Line::from(vec![Span::styled("● ", Style::new().fg(color)), Span::styled(text, theme::text())]));
    }

    out.push(Line::raw(""));
    let n = s.files.len();
    out.push(Line::styled(format!("{n} file{}", if n == 1 { "" } else { "s" }), theme::faint()));
    for file in &s.files {
        let (mark, color) = match file.kind {
            ChangeKind::Added => ("A", theme::GREEN),
            ChangeKind::Deleted => ("D", theme::RED),
            ChangeKind::Renamed | ChangeKind::Copied => ("R", theme::BLUE),
            ChangeKind::Modified | ChangeKind::TypeChange => ("M", theme::YELLOW),
        };
        let mut spans = vec![
            Span::styled(format!("{mark}  "), Style::new().fg(color)),
            Span::styled(file.path.clone(), theme::text()),
        ];
        match file.size {
            Some(Size::Lines { added, removed }) => {
                spans.push(Span::raw("  "));
                if added > 0 {
                    spans.push(Span::styled(format!("+{added} "), Style::new().fg(theme::GREEN)));
                }
                if removed > 0 {
                    spans.push(Span::styled(format!("−{removed}"), Style::new().fg(theme::RED)));
                }
            }
            Some(Size::Binary) => spans.push(Span::styled("  binary", theme::faint())),
            None => {}
        }
        out.push(Line::from(spans));
    }
    out
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
    let right = widgets::hints(&[("ctrl+p", "commands"), ("w", "apply"), ("esc", "abort")]);
    let [l, r] = Layout::horizontal([Constraint::Min(1), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(Paragraph::new(right), r);
}

fn totals(s: &Shown) -> (u32, u32) {
    let (mut a, mut r) = (0, 0);
    for file in &s.files {
        if let Some(Size::Lines { added, removed }) = file.size {
            a += added;
            r += removed;
        }
    }
    (a, r)
}

//! Full-screen editor for the rebase todo list.

use std::collections::HashMap;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::core::diff::Size;
use crate::core::show::{self, Shown};
use crate::core::todo::{self, Action, Line as Todo};
use crate::render::relative_time;

struct App {
    lines: Vec<Todo>,
    original: Vec<Todo>,
    state: ListState,
    preview: bool,
    header: String,
    /// Commit details by todo hash; `None` if it couldn't be read.
    info: HashMap<String, Option<Shown>>,
    message: Option<(String, Color)>,
    confirm_quit: bool,
}

pub enum Outcome {
    Apply(Vec<Todo>),
    Quit,
}

pub fn run(repo: &gix::Repository, lines: Vec<Todo>, header: String) -> Result<Outcome> {
    // Commit details up front: rebases are short, and it keeps the UI simple.
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

    let mut app = App {
        original: lines.clone(),
        lines,
        state: ListState::default().with_selected(Some(0)),
        preview: true,
        header,
        info,
        message: None,
        confirm_quit: false,
    };

    let mut terminal = ratatui::init();
    let result = loop {
        if let Err(e) = terminal.draw(|f| draw(f, &mut app)) {
            break Err(e.into());
        }
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                if let Some(outcome) = app.key(k) {
                    break Ok(outcome);
                }
            }
            Ok(_) => {}
            Err(e) => break Err(e.into()),
        }
    };
    ratatui::restore();
    result
}

impl App {
    fn selected(&self) -> usize {
        self.state.selected().unwrap_or(0).min(self.lines.len().saturating_sub(1))
    }

    fn key(&mut self, k: KeyEvent) -> Option<Outcome> {
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let i = self.selected();
        let last = self.lines.len().saturating_sub(1);
        if !matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
            self.confirm_quit = false;
        }
        self.message = None;

        match k.code {
            KeyCode::Char('c') if ctrl => return Some(Outcome::Quit),
            KeyCode::Char('s') if ctrl => return self.apply(),
            KeyCode::Char('w') => return self.apply(),
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.lines != self.original && !self.confirm_quit {
                    self.confirm_quit = true;
                    self.message = Some(("You have changes. Press q again to abort the rebase.".into(), Color::Yellow));
                } else {
                    return Some(Outcome::Quit);
                }
            }

            // Reorder.
            KeyCode::Char('J') => self.swap(i, i + 1),
            KeyCode::Char('K') if i > 0 => self.swap(i, i - 1),
            KeyCode::Down if shift => self.swap(i, i + 1),
            KeyCode::Up if shift && i > 0 => self.swap(i, i - 1),

            // Move.
            KeyCode::Char('j') | KeyCode::Down => self.state.select(Some((i + 1).min(last))),
            KeyCode::Char('k') | KeyCode::Up => self.state.select(Some(i.saturating_sub(1))),
            KeyCode::Char('g') | KeyCode::Home => self.state.select(Some(0)),
            KeyCode::Char('G') | KeyCode::End => self.state.select(Some(last)),

            KeyCode::Char('u') => {
                self.lines = self.original.clone();
                self.message = Some(("Back to the original list.".into(), Color::DarkGray));
            }
            KeyCode::Tab => self.preview = !self.preview,

            KeyCode::Char(c) => {
                if let Some(a) = Action::ALL.into_iter().find(|a| a.key() == c)
                    && let Some(Todo::Commit { action, .. }) = self.lines.get_mut(i)
                {
                    *action = a;
                    // Move on, so a run of commits can be marked quickly.
                    self.state.select(Some((i + 1).min(last)));
                }
            }
            _ => {}
        }
        None
    }

    fn swap(&mut self, a: usize, b: usize) {
        if b < self.lines.len() {
            self.lines.swap(a, b);
            self.state.select(Some(b));
        }
    }

    fn apply(&mut self) -> Option<Outcome> {
        if let Some(p) = todo::problem(&self.lines) {
            self.message = Some((format!("Can't apply: {p}."), Color::Red));
            return None;
        }
        Some(Outcome::Apply(self.lines.clone()))
    }
}

fn draw(f: &mut Frame, app: &mut App) {
    let [header, body, footer] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(f.area());

    let changed = if app.lines != app.original { " · modified" } else { "" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {}", app.header), Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(changed, Style::new().fg(Color::Yellow)),
        ])),
        header,
    );

    let (list_area, preview_area) = if !app.preview {
        (body, None)
    } else if body.width >= 120 {
        let [l, r] = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(body);
        (l, Some(r))
    } else {
        let [t, b] = Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).areas(body);
        (t, Some(b))
    };

    draw_list(f, app, list_area);
    if let Some(area) = preview_area {
        draw_preview(f, app, area);
    }

    let footer_line = match &app.message {
        Some((m, color)) => Line::styled(format!(" {m}"), Style::new().fg(*color)),
        None => Line::styled(
            " j/k move · J/K reorder · p r e s f d action · u undo all · tab preview · w apply · q abort",
            Style::new().fg(Color::DarkGray),
        ),
    };
    f.render_widget(Paragraph::new(footer_line), footer);
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let sizes: Vec<String> = app
        .lines
        .iter()
        .map(|l| match l {
            Todo::Commit { hash, .. } => app.info.get(hash).and_then(|i| i.as_ref()).map(total).unwrap_or_default(),
            Todo::Other(_) => String::new(),
        })
        .collect();
    let size_w = sizes.iter().map(|s| s.chars().count()).max().unwrap_or(0);

    let items: Vec<ListItem> = app
        .lines
        .iter()
        .zip(&sizes)
        .map(|(l, size)| match l {
            Todo::Commit { action, hash, subject } => {
                let mut subject_style = Style::new();
                if *action == Action::Drop {
                    subject_style = subject_style.fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT);
                }
                let (plus, minus) = size.split_once(' ').unwrap_or((size, ""));
                let pad = " ".repeat(size_w - size.chars().count());
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {:<7}", action.word()), action_style(*action)),
                    Span::styled(hash.clone(), Style::new().fg(Color::Yellow)),
                    Span::raw("  "),
                    Span::styled(plus.to_string(), Style::new().fg(Color::Green)),
                    Span::raw(if minus.is_empty() { "" } else { " " }),
                    Span::styled(minus.to_string(), Style::new().fg(Color::Red)),
                    Span::raw(format!("{pad}  ")),
                    Span::styled(if action.melds() { "↳ " } else { "" }, Style::new().fg(Color::Magenta)),
                    Span::styled(subject.clone(), subject_style),
                ]))
            }
            Todo::Other(raw) => ListItem::new(Line::styled(format!(" {raw}"), Style::new().fg(Color::DarkGray))),
        })
        .collect();

    let list = List::new(items)
        .block(Block::bordered().border_style(Style::new().fg(Color::DarkGray)))
        .highlight_style(Style::new().bg(Color::Indexed(236)).add_modifier(Modifier::BOLD))
        .highlight_symbol("▌");
    f.render_stateful_widget(list, area, &mut app.state);
}

fn draw_preview(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered().border_style(Style::new().fg(Color::DarkGray));
    let text: Vec<Line> = match app.lines.get(app.selected()) {
        Some(Todo::Commit { hash, .. }) => match app.info.get(hash).and_then(|i| i.as_ref()) {
            Some(s) => details(s),
            None => vec![Line::styled("Couldn't read this commit.", Style::new().fg(Color::DarkGray))],
        },
        Some(Todo::Other(_)) => vec![Line::styled(
            "Not a commit: git runs this line as written.",
            Style::new().fg(Color::DarkGray),
        )],
        None => Vec::new(),
    };
    let title = match app.lines.get(app.selected()) {
        Some(Todo::Commit { hash, .. }) => format!(" {hash} "),
        _ => String::new(),
    };
    f.render_widget(Paragraph::new(text).block(block.title(title)).wrap(Wrap { trim: false }), area);
}

fn details(s: &Shown) -> Vec<Line<'static>> {
    let dim = Style::new().fg(Color::DarkGray);
    let mut out = vec![
        Line::styled(s.subject.clone(), Style::new().add_modifier(Modifier::BOLD)),
        Line::from(vec![
            Span::styled(s.author.clone(), Style::new().fg(Color::Blue)),
            Span::styled(format!(" · {}", relative_time(s.time, crate::core::now())), dim),
        ]),
    ];
    if !s.body.is_empty() {
        out.push(Line::raw(""));
        out.extend(s.body.lines().map(|l| Line::raw(l.to_string())));
    }
    out.push(Line::raw(""));
    for file in &s.files {
        let mut spans = vec![
            Span::styled(format!("{} ", crate::render::status::mark(file.kind)), dim),
            Span::raw(file.path.clone()),
        ];
        match file.size {
            Some(Size::Lines { added, removed }) => {
                if added > 0 {
                    spans.push(Span::styled(format!("  +{added}"), Style::new().fg(Color::Green)));
                }
                if removed > 0 {
                    spans.push(Span::styled(format!("  −{removed}"), Style::new().fg(Color::Red)));
                }
            }
            Some(Size::Binary) => spans.push(Span::styled("  binary", dim)),
            None => {}
        }
        out.push(Line::from(spans));
    }
    out
}

/// "+12 −3" for the whole commit (plain text; coloured by the caller).
fn total(s: &Shown) -> String {
    let (mut a, mut r) = (0, 0);
    for file in &s.files {
        if let Some(Size::Lines { added, removed }) = file.size {
            a += added;
            r += removed;
        }
    }
    format!("+{a} −{r}")
}

fn action_style(a: Action) -> Style {
    match a {
        Action::Pick => Style::new(),
        Action::Reword => Style::new().fg(Color::Blue),
        Action::Edit => Style::new().fg(Color::Yellow),
        Action::Squash => Style::new().fg(Color::Magenta),
        Action::Fixup => Style::new().fg(Color::Magenta).add_modifier(Modifier::DIM),
        Action::Drop => Style::new().fg(Color::Red),
    }
}

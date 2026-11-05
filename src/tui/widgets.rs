//! Building blocks shared by the full-screen views: command palette, dialog, toast, footer.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Padding, Paragraph},
};

use super::theme;

/// Area of `w`×`h` centred in `area`, clamped to fit.
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let [r] = Layout::horizontal([Constraint::Length(w.min(area.width))]).flex(Flex::Center).areas(area);
    let [r] = Layout::vertical([Constraint::Length(h.min(area.height))]).flex(Flex::Center).areas(r);
    r
}

/// A panel: background colour and inner padding, no border.
pub fn panel(bg: Color) -> Block<'static> {
    Block::new().style(Style::new().bg(bg)).padding(Padding::new(2, 2, 1, 1))
}

/// "key label" pairs, key bright and label muted: `ctrl+p commands   w apply`.
pub fn hints(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(key.to_string(), theme::text()));
        spans.push(Span::styled(format!(" {label}"), theme::muted()));
    }
    Line::from(spans)
}

/// A coloured tag like ` fixup `.
pub fn pill(text: &str, bg: Color, fg: Color) -> Span<'static> {
    Span::styled(format!(" {text} "), Style::new().bg(bg).fg(fg))
}

// ---------------------------------------------------------------- toast

pub struct Toast {
    text: String,
    color: Color,
    until: Instant,
}

impl Toast {
    pub fn new(text: impl Into<String>, color: Color) -> Self {
        Self { text: text.into(), color, until: Instant::now() + Duration::from_millis(2800) }
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.until
    }

    /// Top-right corner, with a coloured bar on the left.
    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let width = (self.text.chars().count() as u16 + 6).min(area.width.saturating_sub(4)).max(10);
        let lines = self.text.chars().count() as u16 / width.saturating_sub(6).max(1) + 1;
        let r = Rect {
            x: area.right().saturating_sub(width + 2),
            y: area.y + 1,
            width,
            height: lines + 2,
        }
        .intersection(area);
        f.render_widget(Clear, r);
        let bar = Block::new()
            .style(Style::new().bg(theme::ELEMENT))
            .borders(ratatui::widgets::Borders::LEFT)
            .border_set(ratatui::symbols::border::THICK)
            .border_style(Style::new().fg(self.color).bg(theme::ELEMENT))
            .padding(Padding::new(1, 1, 1, 1));
        f.render_widget(
            Paragraph::new(Span::styled(self.text.clone(), theme::text()))
                .wrap(ratatui::widgets::Wrap { trim: true })
                .block(bar),
            r,
        );
    }
}

// ---------------------------------------------------------------- dialog

/// A centred yes/no dialog. Enter confirms, Esc cancels.
pub struct Confirm {
    pub title: String,
    pub body: String,
    pub yes: String,
    pub no: String,
    pub danger: bool,
}

impl Confirm {
    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let w = 56;
        let body_lines = (self.body.chars().count() as u16).div_ceil(w - 6);
        let r = centered(area, w, body_lines + 7);
        f.render_widget(Clear, r);
        let yes_color = if self.danger { theme::RED } else { theme::ACCENT };
        let text = vec![
            Line::styled(self.title.clone(), theme::bold()),
            Line::raw(""),
            Line::styled(self.body.clone(), theme::muted()),
            Line::raw(""),
            Line::from(vec![
                pill(&format!("enter  {}", self.yes), yes_color, theme::BG),
                Span::raw("  "),
                pill(&format!("esc  {}", self.no), theme::BORDER, theme::TEXT),
            ]),
        ];
        f.render_widget(
            Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: true }).block(panel(theme::ELEMENT)),
            r,
        );
    }
}

// ---------------------------------------------------------------- command palette

pub struct Command<T> {
    pub label: &'static str,
    /// Shortcut shown on the right, e.g. "shift+↑".
    pub keys: &'static str,
    pub action: T,
}

pub enum PaletteEvent<T> {
    Close,
    Run(T),
}

/// ctrl+p list of every action: type to filter, enter to run.
pub struct Palette<T: Clone> {
    commands: Vec<Command<T>>,
    query: String,
    selected: usize,
}

impl<T: Clone> Palette<T> {
    pub fn new(commands: Vec<Command<T>>) -> Self {
        Self { commands, query: String::new(), selected: 0 }
    }

    fn matches(&self) -> Vec<usize> {
        let q: Vec<char> = self.query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
        (0..self.commands.len())
            .filter(|&i| {
                // Letters of the query in order, anywhere in the label ("fxp" finds "fixup").
                let mut want = q.iter().peekable();
                for c in self.commands[i].label.to_lowercase().chars() {
                    if want.peek() == Some(&&c) {
                        want.next();
                    }
                }
                want.peek().is_none()
            })
            .collect()
    }

    pub fn key(&mut self, k: KeyEvent) -> Option<PaletteEvent<T>> {
        let found = self.matches();
        match k.code {
            KeyCode::Esc => return Some(PaletteEvent::Close),
            KeyCode::Char('p') if k.modifiers.contains(KeyModifiers::CONTROL) => return Some(PaletteEvent::Close),
            KeyCode::Enter => {
                return found.get(self.selected).map(|&i| PaletteEvent::Run(self.commands[i].action.clone()));
            }
            KeyCode::Down => self.selected = (self.selected + 1).min(found.len().saturating_sub(1)),
            KeyCode::Char('n') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.selected = (self.selected + 1).min(found.len().saturating_sub(1))
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.selected = 0;
            }
            _ => {}
        }
        None
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let found = self.matches();
        let w = 60;
        let rows = found.len().clamp(1, 12) as u16;
        let r = centered(area, w, rows + 6);
        let r = Rect { y: area.y + area.height / 5, ..r }.intersection(area);
        f.render_widget(Clear, r);
        let block = panel(theme::ELEMENT);
        let inner = block.inner(r);
        f.render_widget(block, r);

        let [title, input, _, list] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)])
                .areas(inner);
        let [t_left, t_right] = Layout::horizontal([Constraint::Min(1), Constraint::Length(3)]).areas(title);
        f.render_widget(Paragraph::new(Span::styled("Commands", theme::bold())), t_left);
        f.render_widget(Paragraph::new(Span::styled("esc", theme::muted())), t_right);

        let input_line = if self.query.is_empty() {
            Line::from(vec![Span::styled("▏", Style::new().fg(theme::ACCENT)), Span::styled("Search", theme::faint())])
        } else {
            Line::from(vec![Span::styled(self.query.clone(), theme::text()), Span::styled("▏", Style::new().fg(theme::ACCENT))])
        };
        f.render_widget(Paragraph::new(input_line), input);

        if found.is_empty() {
            f.render_widget(Paragraph::new(Span::styled("No matching commands", theme::muted())), list);
            return;
        }
        // Keep the selection on screen.
        let visible = list.height as usize;
        let start = self.selected.saturating_sub(visible.saturating_sub(1));
        for (row, &i) in found.iter().enumerate().skip(start).take(visible) {
            let cmd = &self.commands[i];
            let y = list.y + (row - start) as u16;
            let line_area = Rect { x: list.x - 1, y, width: list.width + 2, height: 1 };
            let selected = row == self.selected;
            let (bg, fg, key_fg) = if selected {
                (theme::ACCENT, theme::BG, theme::BG)
            } else {
                (theme::ELEMENT, theme::TEXT, theme::MUTED)
            };
            let [l, rgt] = Layout::horizontal([Constraint::Min(1), Constraint::Length(cmd.keys.chars().count() as u16 + 1)])
                .areas(line_area);
            let style = Style::new().bg(bg).fg(fg);
            f.render_widget(Paragraph::new(Span::styled(format!(" {}", cmd.label), style)).style(Style::new().bg(bg)), l);
            f.render_widget(
                Paragraph::new(Span::styled(cmd.keys, Style::new().fg(key_fg).bg(bg).add_modifier(Modifier::empty())))
                    .style(Style::new().bg(bg)),
                rgt,
            );
        }
    }
}

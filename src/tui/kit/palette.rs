//! ctrl+p list of every action: type to filter, enter to run.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::layout::{centered, panel};
use crate::tui::theme;

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
            KeyCode::Char('p') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(PaletteEvent::Close);
            }
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
            Line::from(vec![
                Span::styled(self.query.clone(), theme::text()),
                Span::styled("▏", Style::new().fg(theme::ACCENT)),
            ])
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
            let [l, rgt] =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(cmd.keys.chars().count() as u16 + 1)])
                    .areas(line_area);
            let style = Style::new().bg(bg).fg(fg);
            f.render_widget(
                Paragraph::new(Span::styled(format!(" {}", cmd.label), style)).style(Style::new().bg(bg)),
                l,
            );
            f.render_widget(
                Paragraph::new(Span::styled(cmd.keys, Style::new().fg(key_fg).bg(bg).add_modifier(Modifier::empty())))
                    .style(Style::new().bg(bg)),
                rgt,
            );
        }
    }
}

//! A centred one-line text field: "Rename feat/x", "Stash your changes".

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::layout::{centered, panel};
use super::text::{pill, truncate};
use crate::tui::theme;

pub struct Input {
    /// Bold title, then `subject` in the accent colour.
    pub title: String,
    pub subject: String,
    /// Muted lines under the field.
    pub note: Option<String>,
    /// Shown faint while the field is empty.
    pub placeholder: Option<String>,
    pub yes: String,
    /// Allow spaces (messages) or not (branch names).
    pub spaces: bool,
    input: Vec<char>,
    /// Cursor position in `input`.
    at: usize,
}

pub enum InputEvent {
    Cancel,
    /// The text, trimmed. May be empty.
    Done(String),
}

impl Input {
    pub fn new(title: &str, subject: &str, initial: &str, yes: &str) -> Self {
        let input: Vec<char> = initial.chars().collect();
        Self {
            title: title.into(),
            subject: subject.into(),
            note: None,
            placeholder: None,
            yes: yes.into(),
            spaces: false,
            at: input.len(),
            input,
        }
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    pub fn spaces(mut self) -> Self {
        self.spaces = true;
        self
    }

    pub fn key(&mut self, k: KeyEvent) -> Option<InputEvent> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => return Some(InputEvent::Cancel),
            KeyCode::Enter => return Some(InputEvent::Done(self.input.iter().collect::<String>().trim().to_string())),
            KeyCode::Left => self.at = self.at.saturating_sub(1),
            KeyCode::Right => self.at = (self.at + 1).min(self.input.len()),
            KeyCode::Home => self.at = 0,
            KeyCode::End => self.at = self.input.len(),
            KeyCode::Char('a') if ctrl => self.at = 0,
            KeyCode::Char('e') if ctrl => self.at = self.input.len(),
            KeyCode::Char('u') if ctrl => {
                self.input.drain(..self.at);
                self.at = 0;
            }
            // Delete back to the previous '/', '-', '_' or space.
            KeyCode::Char('w') if ctrl => {
                let stop = |c: &char| matches!(c, '/' | '-' | '_' | ' ');
                let mut start = self.at;
                while start > 0 && stop(&self.input[start - 1]) {
                    start -= 1;
                }
                while start > 0 && !stop(&self.input[start - 1]) {
                    start -= 1;
                }
                self.input.drain(start..self.at);
                self.at = start;
            }
            KeyCode::Backspace if self.at > 0 => {
                self.at -= 1;
                self.input.remove(self.at);
            }
            KeyCode::Delete if self.at < self.input.len() => {
                self.input.remove(self.at);
            }
            KeyCode::Char(c) if !ctrl && (self.spaces || !c.is_whitespace()) => {
                self.input.insert(self.at, c);
                self.at += 1;
            }
            _ => {}
        }
        None
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let note_rows = self.note.as_ref().map_or(0, |n| n.lines().count() as u16 + 1);
        let r = centered(area, 64, 8 + note_rows);
        f.render_widget(Clear, r);
        let block = panel(theme::ELEMENT);
        let inner = block.inner(r);
        f.render_widget(block, r);

        // The field on a slightly lighter strip, with the cursor as a block.
        let w = inner.width as usize;
        let start = (self.at + 1).saturating_sub(w.saturating_sub(1));
        let field = Style::new().bg(theme::BORDER).fg(theme::TEXT);
        let mut spans = Vec::new();
        for (i, c) in self.input.iter().enumerate().skip(start) {
            let style = if i == self.at { Style::new().bg(theme::ACCENT).fg(theme::BG) } else { field };
            spans.push(Span::styled(c.to_string(), style));
        }
        if self.at == self.input.len() {
            spans.push(Span::styled(" ", Style::new().bg(theme::ACCENT)));
        }
        if self.input.is_empty()
            && let Some(p) = &self.placeholder
        {
            spans.push(Span::styled(p.clone(), Style::new().bg(theme::BORDER).fg(theme::MUTED)));
        }
        let used = Line::from(spans.clone()).width();
        spans.push(Span::styled(" ".repeat(w.saturating_sub(used)), field));

        let mut lines = vec![
            Line::from(vec![
                Span::styled(format!("{} ", self.title), theme::bold()),
                Span::styled(self.subject.clone(), theme::bold().fg(theme::ACCENT)),
            ]),
            Line::raw(""),
            truncate(Line::from(spans), inner.width),
            Line::raw(""),
        ];
        if let Some(note) = &self.note {
            lines.extend(note.lines().map(|l| Line::styled(l.to_string(), theme::muted())));
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(vec![
            pill(&format!("enter  {}", self.yes), theme::ACCENT, theme::BG),
            Span::raw("  "),
            pill("esc  Cancel", theme::BORDER, theme::TEXT),
        ]));
        f.render_widget(Paragraph::new(lines), inner);
    }
}

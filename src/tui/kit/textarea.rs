//! A text field that can hold one line or many: a commit summary and its description.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::theme;

pub struct TextArea {
    lines: Vec<Vec<char>>,
    /// Cursor: line, then position in it.
    row: usize,
    col: usize,
    multiline: bool,
    /// First line shown.
    top: usize,
}

impl TextArea {
    pub fn new(text: &str, multiline: bool) -> Self {
        let mut lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
        if !multiline {
            lines.truncate(1);
        }
        let row = lines.len() - 1;
        let col = lines[row].len();
        Self { lines, row, col, multiline, top: 0 }
    }

    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.iter().collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.is_empty())
    }

    /// Characters on the first line (for a summary's length).
    pub fn first_len(&self) -> usize {
        self.lines[0].len()
    }

    /// Editing keys; true if `k` was one of them. Enter adds a line only when multiline.
    pub fn key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Enter if self.multiline && !ctrl && !alt => {
                let rest = self.lines[self.row].split_off(self.col);
                self.lines.insert(self.row + 1, rest);
                self.row += 1;
                self.col = 0;
            }
            KeyCode::Left if self.col > 0 => self.col -= 1,
            KeyCode::Left if self.row > 0 => {
                self.row -= 1;
                self.col = self.lines[self.row].len();
            }
            KeyCode::Right if self.col < self.lines[self.row].len() => self.col += 1,
            KeyCode::Right if self.row + 1 < self.lines.len() => {
                self.row += 1;
                self.col = 0;
            }
            KeyCode::Up if self.row > 0 => {
                self.row -= 1;
                self.col = self.col.min(self.lines[self.row].len());
            }
            KeyCode::Down if self.row + 1 < self.lines.len() => {
                self.row += 1;
                self.col = self.col.min(self.lines[self.row].len());
            }
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = self.lines[self.row].len(),
            KeyCode::Char('a') if ctrl => self.col = 0,
            KeyCode::Char('e') if ctrl => self.col = self.lines[self.row].len(),
            KeyCode::Char('u') if ctrl => {
                self.lines[self.row].drain(..self.col);
                self.col = 0;
            }
            KeyCode::Char('k') if ctrl => {
                self.lines[self.row].truncate(self.col);
            }
            // Back to the start of the word.
            KeyCode::Char('w') if ctrl => {
                let line = &mut self.lines[self.row];
                let mut start = self.col;
                while start > 0 && line[start - 1] == ' ' {
                    start -= 1;
                }
                while start > 0 && line[start - 1] != ' ' {
                    start -= 1;
                }
                line.drain(start..self.col);
                self.col = start;
            }
            KeyCode::Backspace if self.col > 0 => {
                self.col -= 1;
                self.lines[self.row].remove(self.col);
            }
            // At the start of a line: join it to the one above.
            KeyCode::Backspace if self.row > 0 => {
                let line = self.lines.remove(self.row);
                self.row -= 1;
                self.col = self.lines[self.row].len();
                self.lines[self.row].extend(line);
            }
            KeyCode::Delete if self.col < self.lines[self.row].len() => {
                self.lines[self.row].remove(self.col);
            }
            KeyCode::Delete if self.row + 1 < self.lines.len() => {
                let next = self.lines.remove(self.row + 1);
                self.lines[self.row].extend(next);
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                self.lines[self.row].insert(self.col, c);
                self.col += 1;
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Backspace | KeyCode::Delete => {}
            _ => return false,
        }
        true
    }

    /// The text on a field-coloured strip, with the cursor as a block when `focused`.
    /// Lines longer than the field scroll sideways on the cursor's line only.
    pub fn draw(&mut self, f: &mut Frame, area: Rect, focused: bool, placeholder: &str) {
        let field = Style::new().bg(theme::BORDER).fg(theme::TEXT);
        let height = area.height as usize;
        if self.row < self.top {
            self.top = self.row;
        } else if self.row >= self.top + height {
            self.top = self.row + 1 - height;
        }
        let w = area.width as usize;
        let mut out = Vec::new();
        for (r, line) in self.lines.iter().enumerate().skip(self.top).take(height) {
            let here = focused && r == self.row;
            let start = if here { (self.col + 1).saturating_sub(w.saturating_sub(1)) } else { 0 };
            let mut spans = Vec::new();
            for (i, c) in line.iter().enumerate().skip(start).take(w) {
                let style = if here && i == self.col { Style::new().bg(theme::ACCENT).fg(theme::BG) } else { field };
                spans.push(Span::styled(c.to_string(), style));
            }
            if here && self.col == line.len() {
                spans.push(Span::styled(" ", Style::new().bg(theme::ACCENT)));
            }
            if r == 0 && self.is_empty() {
                spans.push(Span::styled(placeholder.to_string(), Style::new().bg(theme::BORDER).fg(theme::MUTED)));
            }
            out.push(Line::from(spans).style(field));
        }
        // Rows past the text still show the field.
        while out.len() < height {
            out.push(Line::raw("").style(field));
        }
        f.render_widget(Paragraph::new(out), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(t: &mut TextArea, code: KeyCode) {
        t.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn typed(t: &mut TextArea, s: &str) {
        for c in s.chars() {
            press(t, if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) });
        }
    }

    #[test]
    fn edits_lines() {
        let mut t = TextArea::new("", true);
        typed(&mut t, "ab\ncd");
        assert_eq!(t.text(), "ab\ncd");
        // Backspace at the start of a line joins it to the one above.
        press(&mut t, KeyCode::Home);
        press(&mut t, KeyCode::Backspace);
        assert_eq!(t.text(), "abcd");
        // The cursor stays at the join; enter there splits again.
        press(&mut t, KeyCode::Enter);
        assert_eq!(t.text(), "ab\ncd");
        press(&mut t, KeyCode::End);
        press(&mut t, KeyCode::Left);
        press(&mut t, KeyCode::Enter);
        assert_eq!(t.text(), "ab\nc\nd");

        // One line only: enter isn't text.
        let mut one = TextArea::new("x\ny", false);
        assert_eq!(one.text(), "x");
        assert!(!one.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    }
}

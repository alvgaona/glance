//! How far a details panel is scrolled.
//!
//! J/K (or shift+↓/↑) move it 3 rows, ctrl+d/u half a panel, the mouse wheel 3 rows.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Frame, layout::Rect, text::Span, widgets::Paragraph};

use crate::tui::theme;

#[derive(Default)]
pub struct Scroll {
    top: u16,
    /// Rows past the bottom of the panel, as of the last draw.
    below: u16,
}

impl Scroll {
    /// The first line shown.
    pub fn top(&self) -> u16 {
        self.top
    }

    pub fn reset(&mut self) {
        self.top = 0;
    }

    /// The details keys; true if `k` was one of them. `height` is the panel's.
    pub fn key(&mut self, k: KeyEvent, height: u16) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        // Some terminals send shift+j as 'j' with SHIFT rather than 'J'.
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let half = (height / 2).max(1);
        match k.code {
            KeyCode::Char('d') if ctrl => self.down(half),
            KeyCode::Char('u') if ctrl => self.up(half),
            KeyCode::Down | KeyCode::Char('j') if shift => self.down(3),
            KeyCode::Up | KeyCode::Char('k') if shift => self.up(3),
            KeyCode::Char('J') => self.down(3),
            KeyCode::Char('K') => self.up(3),
            _ => return false,
        }
        true
    }

    /// One notch of the mouse wheel.
    pub fn wheel(&mut self, down: bool) {
        if down { self.down(3) } else { self.up(3) }
    }

    fn down(&mut self, n: u16) {
        self.top = self.top.saturating_add(n);
    }

    fn up(&mut self, n: u16) {
        self.top = self.top.saturating_sub(n);
    }

    /// Keeps it within `lines` lines shown `height` at a time; call before drawing.
    pub fn fit(&mut self, lines: usize, height: u16) {
        let max = (lines.min(u16::MAX as usize) as u16).saturating_sub(height);
        self.top = self.top.min(max);
        self.below = max - self.top;
    }

    /// "↓ 12 more · J/K" on the bottom edge of `area` while there's more, "↑ J/K" at the end.
    pub fn draw_more(&self, f: &mut Frame, area: Rect) {
        if self.top == 0 && self.below == 0 {
            return;
        }
        let text = if self.below > 0 { format!(" ↓ {} more · J/K ", self.below) } else { " ↑ J/K ".to_string() };
        let w = text.chars().count() as u16;
        let r = Rect { x: area.right().saturating_sub(w + 1), y: area.bottom().saturating_sub(1), width: w, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(text, theme::faint())), r.intersection(area));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn moves_and_stays_in_bounds() {
        let mut s = Scroll::default();
        assert!(s.key(key(KeyCode::Char('J'), KeyModifiers::SHIFT), 20));
        assert!(s.key(key(KeyCode::Char('d'), KeyModifiers::CONTROL), 20));
        assert_eq!(s.top(), 13);
        // 30 lines in a 20-row panel: at most 10 down.
        s.fit(30, 20);
        assert_eq!((s.top(), s.below), (10, 0));
        s.wheel(false);
        assert_eq!(s.top(), 7);
        assert!(!s.key(key(KeyCode::Char('j'), KeyModifiers::NONE), 20));
        // Everything fits: back to the top.
        s.fit(5, 20);
        assert_eq!((s.top(), s.below), (0, 0));
    }
}

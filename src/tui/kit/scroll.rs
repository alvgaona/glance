//! How far a details panel is scrolled.
//!
//! J/K (or shift+↓/↑) move it 3 rows, ctrl+d/u half a panel, the mouse wheel 3 rows,
//! ← → long lines 8 columns.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::theme;

#[derive(Default)]
pub struct Scroll {
    top: u16,
    /// Rows past the bottom of the panel, as of the last draw.
    below: u16,
    /// Columns scrolled sideways, and how far long lines let it go (as of the last draw).
    left: u16,
    over: u16,
}

/// Columns ← and → scroll long lines by.
const SIDEWAYS: u16 = 8;

impl Scroll {
    /// The first line shown.
    pub fn top(&self) -> u16 {
        self.top
    }

    /// Back to the top, and the start of the lines.
    pub fn reset(&mut self) {
        self.top = 0;
        self.left = 0;
    }

    /// Lines run past the panel's side, so ← → scroll them (as of the last draw).
    pub fn wide(&self) -> bool {
        self.over > 0
    }

    /// Puts line `top` first; [`Scroll::fit`] keeps it in bounds.
    pub fn to(&mut self, top: usize) {
        self.top = top.min(u16::MAX as usize) as u16;
    }

    /// Scrolls just enough to show lines `first..=last` in a panel `height` rows tall
    /// (the top of them, if they don't all fit).
    pub fn show(&mut self, first: usize, last: usize, height: u16) {
        let last = last.min(first + (height as usize).saturating_sub(1));
        self.top = super::list::follow(self.top as usize, first, last, height as usize).min(u16::MAX as usize) as u16;
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
            KeyCode::Left => self.left = self.left.saturating_sub(SIDEWAYS),
            KeyCode::Right => self.left = (self.left + SIDEWAYS).min(self.over),
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

    /// Fits it to `lines` drawn in `area` and gives the ones on screen, scrolled sideways.
    /// Line `i` keeps its first `pins[i]` columns in place (`u16::MAX`: all of it); lines
    /// past the end of `pins` keep none.
    pub fn view(&mut self, lines: &[Line<'static>], pins: &[u16], area: Rect) -> Vec<Line<'static>> {
        self.fit(lines.len(), area.height);
        let longest = lines.iter().map(Line::width).max().unwrap_or(0);
        self.over = longest.saturating_sub(area.width as usize).min(u16::MAX as usize) as u16;
        self.left = self.left.min(self.over);
        lines
            .iter()
            .enumerate()
            .skip(self.top as usize)
            .take(area.height as usize)
            .map(|(i, l)| {
                let pin = pins.get(i).copied().unwrap_or(0);
                super::truncate(super::skip(l.clone(), pin, self.left), area.width)
            })
            .collect()
    }

    /// "↓ 12 more · J/K" on the bottom edge of `area` while there's more, "↑ J/K" at the end.
    pub fn draw_more(&self, f: &mut Frame, area: Rect) {
        self.draw_more_keys(f, area, "J/K")
    }

    /// [`Scroll::draw_more`] naming other keys, for screens where J/K do something else.
    pub fn draw_more_keys(&self, f: &mut Frame, area: Rect, keys: &str) {
        if self.top == 0 && self.below == 0 {
            return;
        }
        let text =
            if self.below > 0 { format!(" ↓ {} more · {keys} ", self.below) } else { format!(" ↑ {keys} ") };
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

    #[test]
    fn scrolls_long_lines_sideways() {
        let mut s = Scroll::default();
        let area = Rect::new(0, 0, 10, 5);
        let lines = vec![Line::raw("0123456789abcdefghijklmnopqrstuvwxyz")];
        assert_eq!(s.view(&lines, &[], area), vec![Line::raw("012345678…")]);
        assert!(s.wide());
        let right = key(KeyCode::Right, KeyModifiers::NONE);
        s.key(right, 5);
        assert_eq!(s.view(&lines, &[], area), vec![Line::raw("89abcdefg…")]);
        // Never past the end of the longest line.
        for _ in 0..10 {
            s.key(right, 5);
        }
        assert_eq!(s.view(&lines, &[], area), vec![Line::raw("qrstuvwxyz")]);
        assert!(s.key(key(KeyCode::Left, KeyModifiers::NONE), 5));
        assert_eq!(s.left, 18);
        s.reset();
        assert_eq!(s.left, 0);
        // Everything fits: nothing to scroll.
        assert_eq!(s.view(&[Line::raw("short")], &[], area), vec![Line::raw("short")]);
        assert!(!s.wide());
    }

    #[test]
    fn pinned_columns_stay_while_the_rest_scrolls() {
        let mut s = Scroll::default();
        let area = Rect::new(0, 0, 12, 5);
        let lines = vec![Line::raw("   1 +0123456789abcdef"), Line::raw("   ⋯ line 1 of a long header")];
        s.view(&lines, &[6, u16::MAX], area);
        s.key(key(KeyCode::Right, KeyModifiers::NONE), 5);
        let shown: Vec<String> = s.view(&lines, &[6, u16::MAX], area).iter().map(|l| l.to_string()).collect();
        assert_eq!(shown, ["   1 +89abc…", "   ⋯ line 1…"]);
    }
}

//! A centred yes/no question.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use super::layout::{centered, panel};
use super::text::pill;
use crate::tui::theme;

/// Enter confirms, Esc cancels.
pub struct Confirm {
    pub title: String,
    pub body: String,
    pub yes: String,
    pub no: String,
    pub danger: bool,
}

impl Confirm {
    /// `body` may have several lines; lines starting with `$ ` are shown as commands.
    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let yes_color = if self.danger { theme::RED } else { theme::ACCENT };
        let mut text = vec![Line::styled(self.title.clone(), theme::bold()), Line::raw("")];
        for l in self.body.lines() {
            text.push(match l.strip_prefix("$ ") {
                Some(cmd) => Line::styled(format!("  {cmd}"), Style::new().fg(theme::ACCENT)),
                None => Line::styled(l.to_string(), theme::muted()),
            });
        }
        text.push(Line::raw(""));
        text.push(Line::from(vec![
            pill(&format!("enter  {}", self.yes), yes_color, theme::BG),
            Span::raw("  "),
            pill(&format!("esc  {}", self.no), theme::BORDER, theme::TEXT),
        ]));
        let p = Paragraph::new(text).wrap(Wrap { trim: true }).block(panel(theme::ELEMENT));
        // As tall as the text once wrapped (the panel's sides take 4 columns).
        let w = 64.min(area.width);
        let h = p.line_count(w.saturating_sub(4)) as u16;
        let r = centered(area, w, h);
        f.render_widget(Clear, r);
        f.render_widget(p, r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    /// The screen as text, one string per row.
    fn render(c: &Confirm, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| c.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect()).collect()
    }

    #[test]
    fn grows_with_wrapped_text() {
        // Words that break early: counting characters says 2 rows, wrapping takes 3.
        let body = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb cccc dddd\nlast line";
        let c = Confirm {
            title: "A title long enough that it has to wrap onto a second row here".into(),
            body: body.into(),
            yes: "Yes".into(),
            no: "No".into(),
            danger: false,
        };
        for (w, h) in [(80, 30), (40, 30)] {
            let screen = render(&c, w, h).join("\n");
            for want in ["last line", "dddd", "enter  Yes", "second row"] {
                assert!(screen.contains(want), "{want:?} missing at {w}x{h}:\n{screen}");
            }
        }
    }
}

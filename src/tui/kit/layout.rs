//! Where floating boxes go and how panels look.

use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Padding},
};

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

/// The sidebar shows from this many columns, this wide.
const SIDE_FROM: u16 = 180;
const SIDE_WIDTH: u16 = 44;

/// A screen's body, split: the list, the main pane, and on wide screens a sidebar.
#[derive(Debug, PartialEq, Eq)]
pub struct Panes {
    /// Empty when the screen has no list right now.
    pub list: Rect,
    pub main: Rect,
    pub side: Option<Rect>,
}

/// Splits `body` the same way on every screen. The list is `want` columns (its widest
/// row, padding included), at least 28 and at most half the room, when there are 100
/// columns or more; narrower, it goes on top, `rows` tall, at most half the height.
/// `None`: no list, the main pane takes it all. `side`: a sidebar from [`SIDE_FROM`].
pub fn panes(body: Rect, want: Option<u16>, rows: u16, side: bool) -> Panes {
    let (rest, side) = if side && body.width >= SIDE_FROM {
        let [m, _, s] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(1), Constraint::Length(SIDE_WIDTH)]).areas(body);
        (m, Some(s))
    } else {
        (body, None)
    };
    let Some(want) = want else { return Panes { list: Rect::default(), main: rest, side } };
    let (list, main) = if rest.width >= 100 {
        // Not `clamp`: it panics when the upper bound is the smaller one.
        let w = want.min(rest.width / 2).max(28);
        let [l, _, m] =
            Layout::horizontal([Constraint::Length(w), Constraint::Length(1), Constraint::Min(1)]).areas(rest);
        (l, m)
    } else {
        let h = rows.min(rest.height / 2).max(4);
        let [t, _, b] =
            Layout::vertical([Constraint::Length(h), Constraint::Length(1), Constraint::Min(1)]).areas(rest);
        (t, b)
    };
    Panes { list, main, side }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_fit_the_list_and_add_a_sidebar_when_wide() {
        let body = |w, h| Rect::new(0, 0, w, h);
        // Short rows: the minimum; long ones: half.
        assert_eq!(panes(body(150, 40), Some(10), 5, true).list.width, 28);
        assert_eq!(panes(body(150, 40), Some(500), 5, true).list.width, 75);
        let wide = panes(body(200, 40), Some(40), 5, true);
        assert_eq!((wide.list.width, wide.side.map(|s| s.width)), (40, Some(44)));
        assert_eq!(wide.main.width, 200 - 40 - 1 - 1 - 44);
        // Narrow: stacked, the list as tall as its rows.
        let narrow = panes(body(80, 40), Some(40), 6, true);
        assert_eq!((narrow.list.width, narrow.list.height, narrow.side), (80, 6, None));
        assert_eq!(panes(body(150, 40), None, 5, false).main, body(150, 40));
    }
}

//! Scrolling a list so the selected row stays on screen, and finding the row under a click.

use crossterm::event::MouseEvent;
use ratatui::layout::{Position, Rect};

/// First row to show so rows `first..=last` (the selection, maybe with a heading above
/// it) stay on a screen `height` rows tall, moving from `top` as little as possible.
pub fn follow(top: usize, first: usize, last: usize, height: usize) -> usize {
    if first < top {
        first
    } else if height > 0 && last >= top + height {
        last + 1 - height
    } else {
        top
    }
}

/// The row a click landed on, counting from the list's first row, if it was in `area`.
pub fn row_at(area: Rect, top: usize, m: &MouseEvent) -> Option<usize> {
    inside(area, m).then(|| top + (m.row - area.y) as usize)
}

pub fn inside(area: Rect, m: &MouseEvent) -> bool {
    area.contains(Position::new(m.column, m.row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_selection() {
        // Already on screen: no move.
        assert_eq!(follow(5, 7, 7, 10), 5);
        // Above: scroll up to it.
        assert_eq!(follow(5, 2, 2, 10), 2);
        // Below: just enough to show it on the last row.
        assert_eq!(follow(0, 12, 12, 10), 3);
        // A heading above the selection comes along.
        assert_eq!(follow(5, 4, 5, 10), 4);
        assert_eq!(follow(0, 0, 0, 0), 0);
    }
}

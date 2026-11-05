//! Lays out the commit graph as rows of box-drawing cells.
//!
//! Each lane takes two columns: the lane itself and a spacer to its right,
//! so horizontal connectors have room (`●─╯`, `├─╮`).

use gix::ObjectId;

use super::log::Commit;

#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: char,
    /// Which lane this cell belongs to, for colouring.
    pub lane: usize,
}

#[derive(Debug)]
pub enum Row {
    /// The row showing commit `index`.
    Commit { index: usize, cells: Vec<Cell> },
    /// A connector row below a merge commit, leading to its other parents.
    Edge { cells: Vec<Cell> },
}

/// `commits` must list children before their parents.
pub fn layout(commits: &[Commit]) -> Vec<Row> {
    // lanes[i] = the commit that lane i is waiting for.
    let mut lanes: Vec<Option<ObjectId>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());

    for (index, c) in commits.iter().enumerate() {
        let mut hits: Vec<usize> =
            lanes.iter().enumerate().filter(|(_, l)| **l == Some(c.oid)).map(|(i, _)| i).collect();
        let col = if hits.is_empty() {
            // A tip nobody was waiting for.
            let i = free_slot(&mut lanes);
            lanes[i] = Some(c.oid);
            i
        } else {
            hits.remove(0)
        };

        // Commit row; other lanes waiting for this commit join it here (`●─╯`).
        let mut cells = base(&lanes);
        cells[2 * col] = Cell { ch: '●', lane: col };
        for &p in &hits {
            connect(&mut cells, col, p, '╯');
            lanes[p] = None;
        }
        rows.push(Row::Commit { index, cells });

        // The first parent continues this lane; others get a lane each.
        lanes[col] = c.parents.first().copied();
        let mut targets = Vec::new();
        for &parent in c.parents.iter().skip(1) {
            let target = match lanes.iter().position(|l| *l == Some(parent)) {
                Some(m) => (m, if m > col { '┤' } else { '├' }),
                None => {
                    let n = free_slot(&mut lanes);
                    lanes[n] = Some(parent);
                    (n, if n > col { '╮' } else { '╭' })
                }
            };
            targets.push(target);
        }
        if !targets.is_empty() {
            let mut cells = base(&lanes);
            let right = targets.iter().any(|(t, _)| *t > col);
            let left = targets.iter().any(|(t, _)| *t < col);
            let ch = match (left, right) {
                (true, true) => '┼',
                (false, true) => '├',
                _ => '┤',
            };
            cells[2 * col] = Cell { ch, lane: col };
            for (t, end) in targets {
                connect(&mut cells, col, t, end);
            }
            rows.push(Row::Edge { cells });
        }

        while lanes.last() == Some(&None) {
            lanes.pop();
        }
    }
    rows
}

/// First empty lane, or a new one on the right.
fn free_slot(lanes: &mut Vec<Option<ObjectId>>) -> usize {
    if let Some(i) = lanes.iter().position(Option::is_none) {
        return i;
    }
    lanes.push(None);
    lanes.len() - 1
}

/// A `│` for every active lane.
fn base(lanes: &[Option<ObjectId>]) -> Vec<Cell> {
    let mut cells = vec![Cell { ch: ' ', lane: 0 }; lanes.len() * 2];
    for (i, l) in lanes.iter().enumerate() {
        if l.is_some() {
            cells[2 * i] = Cell { ch: '│', lane: i };
        }
    }
    cells
}

/// Horizontal line from lane `from` to lane `to`, ending in `end`. Crossed lanes become `┼`.
fn connect(cells: &mut [Cell], from: usize, to: usize, end: char) {
    let (a, b) = if from < to { (2 * from + 1, 2 * to) } else { (2 * to + 1, 2 * from) };
    for cell in &mut cells[a..b] {
        cell.ch = match cell.ch {
            '│' => '┼',
            // Another join already ends here; the line passes through it.
            '╯' => '┴',
            '╮' => '┬',
            '●' | '├' | '┤' | '┼' | '┴' | '┬' => cell.ch,
            _ => '─',
        };
        if cell.ch == '─' {
            cell.lane = to;
        }
    }
    cells[2 * to] = Cell { ch: end, lane: to };
}

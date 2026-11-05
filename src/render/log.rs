use owo_colors::{OwoColorize, Style};

use super::{Paint, relative_time};
use crate::core::graph::{self, Cell, Row};
use crate::core::log::{Commit, RefLabel};

pub fn print(commits: &[Commit], p: &Paint) {
    for line in lines(commits, p) {
        println!("{line}");
    }
}

/// Graph plus one formatted line per commit. Commit lines contain the short id
/// as their first hex word; connector lines have no id.
pub fn lines(commits: &[Commit], p: &Paint) -> Vec<String> {
    let now = crate::core::now();
    let ages: Vec<String> = commits.iter().map(|c| relative_time(c.time, now)).collect();
    let age_w = ages.iter().map(|a| a.len()).max().unwrap_or(0);
    let author_w = commits.iter().map(|c| short_author(&c.author).len()).max().unwrap_or(0);

    let rows = graph::layout(commits);
    let graph_w = rows
        .iter()
        .map(|r| match r {
            Row::Commit { cells, .. } | Row::Edge { cells } => cells.len(),
        })
        .max()
        .unwrap_or(0);

    rows.iter()
        .map(|row| match row {
            Row::Edge { cells } => draw(cells, p).trim_end().to_string(),
            Row::Commit { index, cells } => {
                let c = &commits[*index];
                let summary_style = if c.is_merge { p.dim() } else { Style::new() };
                let pad = " ".repeat(graph_w - cells.len());
                format!(
                    "{}{pad} {}  {:>age_w$}  {:<author_w$}  {}{}",
                    draw(cells, p),
                    c.id.style(p.yellow()),
                    ages[*index].style(p.dim()),
                    short_author(&c.author).style(p.blue()),
                    labels(&c.refs, p),
                    c.summary.style(summary_style),
                )
            }
        })
        .collect()
}

fn draw(cells: &[Cell], p: &Paint) -> String {
    const LANE_COLORS: [fn(Style) -> Style; 6] = [
        |s| s.cyan(),
        |s| s.green(),
        |s| s.yellow(),
        |s| s.magenta(),
        |s| s.blue(),
        |s| s.red(),
    ];
    cells
        .iter()
        .map(|c| {
            if c.ch == ' ' {
                " ".to_string()
            } else {
                let color = LANE_COLORS[c.lane % LANE_COLORS.len()](Style::new());
                c.ch.style(p.pick(color)).to_string()
            }
        })
        .collect()
}

/// First name only, to keep the column narrow.
fn short_author(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

pub fn labels(refs: &[RefLabel], p: &Paint) -> String {
    if refs.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = refs
        .iter()
        .map(|r| match r {
            RefLabel::Head(n) => format!("{}", n.style(p.head())),
            RefLabel::Branch(n) => format!("{}", n.style(p.green())),
            RefLabel::Remote(n) => format!("{}", n.style(p.red())),
            RefLabel::Tag(n) => format!("{}", format!("#{n}").style(p.magenta())),
        })
        .collect();
    format!("({}) ", parts.join(", "))
}

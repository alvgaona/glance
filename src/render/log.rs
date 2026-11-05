use owo_colors::OwoColorize;

use super::{Paint, relative_time};
use crate::core::log::{Commit, RefLabel};

pub fn print(commits: &[Commit], p: &Paint) {
    for line in lines(commits, p) {
        println!("{line}");
    }
}

/// One formatted line per commit, starting with the short id.
pub fn lines(commits: &[Commit], p: &Paint) -> Vec<String> {
    let now = crate::core::now();
    let ages: Vec<String> = commits.iter().map(|c| relative_time(c.time, now)).collect();
    let age_w = ages.iter().map(|a| a.len()).max().unwrap_or(0);
    let author_w = commits.iter().map(|c| short_author(&c.author).len()).max().unwrap_or(0);

    commits
        .iter()
        .zip(&ages)
        .map(|(c, age)| {
            let summary_style = if c.is_merge { p.dim() } else { owo_colors::Style::new() };
            format!(
                "{}  {:>age_w$}  {:<author_w$}  {}{}",
                c.id.style(p.yellow()),
                age.style(p.dim()),
                short_author(&c.author).style(p.blue()),
                labels(&c.refs, p),
                c.summary.style(summary_style),
            )
        })
        .collect()
}

/// First name only, to keep the column narrow.
fn short_author(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

fn labels(refs: &[RefLabel], p: &Paint) -> String {
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
    format!("{} ", format!("({})", parts.join(", ")))
}

//! `gg stash` when piped: one line per stash.

use owo_colors::OwoColorize;

use super::{Paint, Tone};
use crate::fmt::{pad, relative_time, width};
use crate::git::stash::Stash;

pub fn print(list: &[Stash], p: &Paint) {
    super::out(lines(list, p));
}

pub fn lines(list: &[Stash], p: &Paint) -> Vec<String> {
    let now = crate::fmt::now();
    let ages: Vec<String> = list.iter().map(|s| relative_time(s.time, now)).collect();
    let age_w = ages.iter().map(|a| a.len()).max().unwrap_or(0);
    let branch = |s: &Stash| s.branch.as_deref().unwrap_or("-").to_string();
    let branch_w = list.iter().map(|s| width(&branch(s))).max().unwrap_or(0);
    list.iter()
        .zip(&ages)
        .map(|(s, age)| {
            format!(
                "{}  {:>age_w$}  {}{}  {}",
                s.name().style(p.tone(Tone::Accent)),
                age.style(p.tone(Tone::Faint)),
                branch(s).style(p.tone(Tone::Muted)),
                pad(&branch(s), branch_w),
                s.title()
            )
        })
        .collect()
}

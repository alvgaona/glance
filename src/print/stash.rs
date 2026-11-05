//! `gg stash` when piped: one line per stash.

use owo_colors::OwoColorize;

use super::{Paint, Tone};
use crate::fmt::relative_time;
use crate::git::stash::Stash;

pub fn print(list: &[Stash], p: &Paint) {
    let now = crate::fmt::now();
    let ages: Vec<String> = list.iter().map(|s| relative_time(s.time, now)).collect();
    let age_w = ages.iter().map(|a| a.len()).max().unwrap_or(0);
    let branch_w = list.iter().map(|s| s.branch.as_deref().unwrap_or("-").len()).max().unwrap_or(0);
    for (s, age) in list.iter().zip(&ages) {
        let branch = s.branch.as_deref().unwrap_or("-");
        println!(
            "{}  {:>age_w$}  {:<branch_w$}  {}",
            s.name().style(p.tone(Tone::Accent)),
            age.style(p.tone(Tone::Faint)),
            branch.style(p.tone(Tone::Muted)),
            s.title()
        );
    }
}

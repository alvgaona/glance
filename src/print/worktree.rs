//! `gg wt --list`: one line per worktree.

use owo_colors::OwoColorize;

use super::{Paint, Tone};
use crate::git::worktree::Worktree;

pub fn print(list: &[Worktree], p: &Paint) {
    lines(list, p).iter().for_each(|l| println!("{l}"));
}

pub fn lines(list: &[Worktree], p: &Paint) -> Vec<String> {
    let paths: Vec<String> = list.iter().map(|w| w.path.display().to_string()).collect();
    let path_w = paths.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    list.iter()
        .zip(&paths)
        .map(|(w, path)| {
            let branch = match (&w.branch, w.bare) {
                (_, true) => "(bare)".to_string(),
                (Some(b), _) => b.clone(),
                (None, _) => w.head.map(|h| format!("(detached at {})", &h.to_string()[..7])).unwrap_or_default(),
            };
            let tags = [w.locked.as_ref().map(|_| "locked"), w.prunable.as_ref().map(|_| "prunable")];
            format!(
                "{:<path_w$}  {}  {}",
                path,
                branch.style(p.tone(if w.branch.is_some() { Tone::Accent } else { Tone::Muted })),
                tags.into_iter().flatten().collect::<Vec<_>>().join(" ").style(p.tone(Tone::Yellow))
            )
        })
        .collect()
}

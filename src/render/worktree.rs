//! `gg wt --list`: one line per worktree.

use owo_colors::OwoColorize;

use super::{Paint, Tone};
use crate::core::worktree::Worktree;

pub fn print(list: &[Worktree], p: &Paint) {
    let paths: Vec<String> = list.iter().map(|w| w.path.display().to_string()).collect();
    let path_w = paths.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    for (w, path) in list.iter().zip(&paths) {
        let branch = match (&w.branch, w.bare) {
            (_, true) => "(bare)".to_string(),
            (Some(b), _) => b.clone(),
            (None, _) => w.head.map(|h| format!("(detached at {})", &h.to_string()[..7])).unwrap_or_default(),
        };
        let mut tags = Vec::new();
        if w.locked.is_some() {
            tags.push("locked");
        }
        if w.prunable.is_some() {
            tags.push("prunable");
        }
        println!(
            "{:<path_w$}  {}  {}",
            path,
            branch.style(p.tone(if w.branch.is_some() { Tone::Accent } else { Tone::Muted })),
            tags.join(" ").style(p.tone(Tone::Yellow))
        );
    }
}

//! `gg remote --list`: one line per remote, and its push URL when it has its own.

use owo_colors::OwoColorize;

use super::{Paint, Tone};
use crate::git::remote::Remote;

pub fn print(list: &[Remote], p: &Paint) {
    lines(list, p).iter().for_each(|l| println!("{l}"));
}

pub fn lines(list: &[Remote], p: &Paint) -> Vec<String> {
    let name_w = list.iter().map(|r| r.name.chars().count()).max().unwrap_or(0);
    list.iter()
        .flat_map(|r| {
            let fetch = format!(
                "{:<name_w$}  {}",
                r.name.style(p.tone(Tone::Accent)),
                r.url.as_deref().unwrap_or("(no url)").style(p.tone(Tone::Text))
            );
            let push = r.push_url.as_ref().map(|push| {
                format!("{:<name_w$}  {} {}", "", "push".style(p.tone(Tone::Faint)), push.style(p.tone(Tone::Yellow)))
            });
            std::iter::once(fetch).chain(push)
        })
        .collect()
}

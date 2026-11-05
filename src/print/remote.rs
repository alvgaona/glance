//! `gg remote --list`: one line per remote, and its push URL when it has its own.

use owo_colors::OwoColorize;

use super::{Paint, Tone};
use crate::git::remote::Remote;

pub fn print(list: &[Remote], p: &Paint) {
    let name_w = list.iter().map(|r| r.name.chars().count()).max().unwrap_or(0);
    for r in list {
        println!(
            "{:<name_w$}  {}",
            r.name.style(p.tone(Tone::Accent)),
            r.url.as_deref().unwrap_or("(no url)").style(p.tone(Tone::Text))
        );
        if let Some(push) = &r.push_url {
            println!("{:<name_w$}  {} {}", "", "push".style(p.tone(Tone::Faint)), push.style(p.tone(Tone::Yellow)));
        }
    }
}

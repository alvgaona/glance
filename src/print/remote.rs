//! `gg remote --list`: one line per remote, and its push URL when it has its own.

use crate::fmt::{pad, width};
use owo_colors::OwoColorize;

use super::{Paint, Tone};
use crate::git::remote::Remote;

pub fn print(list: &[Remote], p: &Paint) {
    super::out(lines(list, p));
}

pub fn lines(list: &[Remote], p: &Paint) -> Vec<String> {
    let name_w = list.iter().map(|r| width(&r.name)).max().unwrap_or(0);
    list.iter()
        .flat_map(|r| {
            let fetch = format!(
                "{}{}  {}",
                r.name.style(p.tone(Tone::Accent)),
                pad(&r.name, name_w),
                r.url.as_deref().unwrap_or("(no url)").style(p.tone(Tone::Text))
            );
            let push = r.push_url.as_ref().map(|push| {
                format!(
                    "{}  {} {}",
                    " ".repeat(name_w),
                    "push".style(p.tone(Tone::Faint)),
                    push.style(p.tone(Tone::Yellow))
                )
            });
            std::iter::once(fetch).chain(push)
        })
        .collect()
}

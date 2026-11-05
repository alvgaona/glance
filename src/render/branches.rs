use owo_colors::OwoColorize;

use super::{Paint, relative_time};
use crate::core::branches::Branches;

pub fn print(b: &Branches, p: &Paint) {
    let now = crate::core::now();
    let name_w = b.list.iter().map(|x| x.name.chars().count()).max().unwrap_or(0);
    let ages: Vec<String> = b.list.iter().map(|x| relative_time(x.time, now)).collect();
    let age_w = ages.iter().map(|a| a.len()).max().unwrap_or(0);
    let tracks: Vec<(String, usize)> = b.list.iter().map(|x| tracking(x, p)).collect();
    let track_w = tracks.iter().map(|(_, w)| *w).max().unwrap_or(0);

    for ((br, age), (track, tw)) in b.list.iter().zip(&ages).zip(&tracks) {
        let marker = if br.current { "*".style(p.head()).to_string() } else { " ".into() };
        let name_pad = " ".repeat(name_w - br.name.chars().count());
        let name = if br.current {
            br.name.style(p.head()).to_string()
        } else if br.merged {
            br.name.style(p.dim()).to_string()
        } else {
            br.name.clone()
        };
        let merged = if br.merged { "merged".style(p.dim()).to_string() } else { "      ".into() };
        println!(
            "{marker} {name}{name_pad}  {:>age_w$}  {track}{}  {merged}  {} {}",
            age.style(p.dim()),
            " ".repeat(track_w - tw),
            br.id.style(p.yellow()),
            br.summary.style(p.dim()),
        );
    }
    if let Some(def) = &b.default {
        let n = b.list.iter().filter(|x| x.merged).count();
        if n > 0 {
            println!("\n{}", format!("{n} merged into {def}").style(p.dim()));
        }
    }
}

/// "↑2 ↓1", "gone", "=" or "" and its visible width.
fn tracking(b: &crate::core::branches::Branch, p: &Paint) -> (String, usize) {
    let Some(up) = &b.upstream else {
        return (String::new(), 0);
    };
    if up.name.ends_with("(gone)") {
        return ("gone".style(p.red()).to_string(), 4);
    }
    let mut parts = Vec::new();
    let mut width = 0;
    if up.ahead > 0 {
        let s = format!("↑{}", up.ahead);
        width += s.chars().count();
        parts.push(s.style(p.green()).to_string());
    }
    if up.behind > 0 {
        let s = format!("↓{}", up.behind);
        width += s.chars().count();
        parts.push(s.style(p.red()).to_string());
    }
    if parts.is_empty() {
        return ("=".style(p.dim()).to_string(), 1);
    }
    width += parts.len() - 1;
    (parts.join(" "), width)
}

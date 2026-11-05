//! `gg br` when piped: one line per branch.

use owo_colors::OwoColorize;

use super::Paint;
use crate::fmt::{pad, relative_time, width};
use crate::git::branches::Branches;

pub fn print(b: &Branches, p: &Paint) {
    super::out(lines(b, p));
}

pub fn lines(b: &Branches, p: &Paint) -> Vec<String> {
    let now = crate::fmt::now();
    let name_w = b.list.iter().map(|x| width(&x.name)).max().unwrap_or(0);
    let ages: Vec<String> = b.list.iter().map(|x| relative_time(x.time, now)).collect();
    let age_w = ages.iter().map(|a| a.len()).max().unwrap_or(0);
    let tracks: Vec<(String, usize)> = b.list.iter().map(|x| tracking(x, p)).collect();
    let track_w = tracks.iter().map(|(_, w)| *w).max().unwrap_or(0);

    let rows = b.list.iter().zip(&ages).zip(&tracks).map(|((br, age), (track, tw))| {
        let marker = if br.current { "*".style(p.head()).to_string() } else { " ".into() };
        let name_pad = pad(&br.name, name_w);
        let name = if br.current {
            br.name.style(p.head()).to_string()
        } else if br.merged {
            br.name.style(p.dim()).to_string()
        } else {
            br.name.clone()
        };
        let merged = if br.merged { "merged".style(p.dim()).to_string() } else { "      ".into() };
        format!(
            "{marker} {name}{name_pad}  {:>age_w$}  {track}{}  {merged}  {} {}",
            age.style(p.dim()),
            " ".repeat(track_w - tw),
            br.id.style(p.yellow()),
            br.summary.style(p.dim()),
        )
    });
    let merged = b.list.iter().filter(|x| x.merged).count();
    let footer = b
        .default
        .as_ref()
        .filter(|_| merged > 0)
        .map(|(def, _)| format!("\n{}", format!("{merged} merged into {def}").style(p.dim())));
    rows.chain(footer).collect()
}

/// "↑2 ↓1", "gone", "=" or "" and its visible width.
fn tracking(b: &crate::git::branches::Branch, p: &Paint) -> (String, usize) {
    let Some(up) = &b.upstream else {
        return (String::new(), 0);
    };
    if up.name.ends_with("(gone)") {
        return ("gone".style(p.red()).to_string(), 4);
    }
    let plain = crate::fmt::sync::arrows(up.ahead, up.behind);
    if plain.is_empty() {
        return ("=".style(p.dim()).to_string(), 1);
    }
    (p.arrows(up.ahead, up.behind).join(" "), width(&plain))
}

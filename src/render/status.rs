//! `gg st`: printed status in the look of the full-screen views (same palette, tags, accent bar).

use owo_colors::OwoColorize;

use super::{Paint, Tone};
use crate::core::diff::Size;
use crate::core::status::{ChangeKind, FileChange, Status};

/// Widest `■` bar for the biggest change.
const BAR: usize = 10;

pub fn print(st: &Status, p: &Paint) {
    print_header(st, p);

    let clean = st.staged.is_empty() && st.unstaged.is_empty() && st.untracked.is_empty() && st.conflicted.is_empty();
    if clean {
        println!();
        println!(
            "  {} {}",
            "✓".style(p.tone(Tone::Green)),
            "Nothing to commit, working tree clean".style(p.tone(Tone::Muted))
        );
        return;
    }

    // One column layout for every section, so sizes and bars line up down the page.
    let rows: Vec<&FileChange> = st.staged.iter().chain(&st.unstaged).collect();
    let name_w = rows
        .iter()
        .map(|f| display_name(f).chars().count())
        .chain(st.conflicted.iter().chain(&st.untracked).map(|s| s.chars().count()))
        .max()
        .unwrap_or(0);
    let (add_w, del_w, biggest) = rows.iter().filter_map(|f| f.size).fold((0, 0, 0u32), |(a, d, m), s| match s {
        Size::Lines { added, removed } => (
            a.max(if added > 0 { added.to_string().len() + 1 } else { 0 }),
            d.max(if removed > 0 { removed.to_string().len() + 1 } else { 0 }),
            m.max(added + removed),
        ),
        Size::Binary => (a, d, m),
    });
    let cols = Columns { name_w, add_w, del_w, biggest };

    if !st.conflicted.is_empty() {
        section("CONFLICTS", Tone::Red, st.conflicted.len(), None, p);
        for path in &st.conflicted {
            println!("  {}  {}", "!".style(p.tone_bold(Tone::Red)), path_text(path, Tone::Red, p));
        }
    }
    if !st.staged.is_empty() {
        section("STAGED", Tone::Green, st.staged.len(), Some(&st.staged), p);
        for f in &st.staged {
            file_row(f, &cols, p);
        }
    }
    if !st.unstaged.is_empty() {
        section("CHANGES", Tone::Yellow, st.unstaged.len(), Some(&st.unstaged), p);
        for f in &st.unstaged {
            file_row(f, &cols, p);
        }
    }
    if !st.untracked.is_empty() {
        section("UNTRACKED", Tone::Muted, st.untracked.len(), None, p);
        for path in &st.untracked {
            println!("  {}  {}", "?".style(p.tone(Tone::Faint)), path_text(path, Tone::Text, p));
        }
    }
}

struct Columns {
    name_w: usize,
    add_w: usize,
    del_w: usize,
    biggest: u32,
}

/// `┃ main  e6f5685 · origin/main ↑1 · 1 stash` and, during a rebase, how to go on.
fn print_header(st: &Status, p: &Paint) {
    let sep = format!(" {} ", "·".style(p.tone(Tone::Faint)));
    let mut parts: Vec<String> = Vec::new();

    let mut first = match &st.branch {
        Some(b) => b.style(p.tone_bold(Tone::Accent)).to_string(),
        None => "detached".style(p.tone_bold(Tone::Yellow)).to_string(),
    };
    first += "  ";
    first += &match &st.head {
        Some(h) => h.style(p.tone(Tone::Muted)).to_string(),
        None => "no commits yet".style(p.tone(Tone::Muted)).to_string(),
    };
    parts.push(first);

    if let Some(up) = &st.upstream {
        let mut s = up.name.style(p.tone(Tone::Muted)).to_string();
        if up.ahead == 0 && up.behind == 0 {
            s += &format!(" {}", "up to date".style(p.tone(Tone::Faint)));
        }
        if up.ahead > 0 {
            s += &format!(" {}", format!("↑{}", up.ahead).style(p.tone(Tone::Green)));
        }
        if up.behind > 0 {
            s += &format!(" {}", format!("↓{}", up.behind).style(p.tone(Tone::Red)));
        }
        parts.push(s);
    } else if st.branch.is_some() && st.head.is_some() {
        parts.push("no upstream".style(p.tone(Tone::Faint)).to_string());
    }
    if st.stashes > 0 {
        let s = if st.stashes == 1 { "" } else { "es" };
        parts.push(format!("{} stash{s}", st.stashes).style(p.tone(Tone::Purple)).to_string());
    }

    let bar = "┃".style(p.tone(Tone::Accent));
    let mut line = format!("{bar} {}", parts.join(&sep));
    if let Some(op) = &st.in_progress {
        line += &format!("  {}", format!(" {op} ").style(p.tag(Tone::Red)));
    }
    println!("{line}");
    if st.in_progress.as_deref().is_some_and(|op| op.starts_with("rebasing")) {
        let cmds = ["--continue", "--skip", "--abort", "--edit-todo"]
            .map(|c| c.style(p.tone(Tone::Text)).to_string())
            .join(&sep);
        println!("{bar} {} {cmds}", "gg rebase".style(p.tone(Tone::Muted)));
    }
}

/// ` STAGED  3 files  +24 −5`
fn section(title: &str, tone: Tone, n: usize, files: Option<&[FileChange]>, p: &Paint) {
    let noun = if n == 1 { "file" } else { "files" };
    let mut line = format!(
        "\n  {}  {}",
        format!(" {title} ").style(p.tag(tone)),
        format!("{n} {noun}").style(p.tone(Tone::Muted))
    );
    if let Some(files) = files {
        let (a, d) = files.iter().filter_map(|f| f.size).fold((0, 0), |(a, d), s| match s {
            Size::Lines { added, removed } => (a + added, d + removed),
            Size::Binary => (a, d),
        });
        if a > 0 {
            line += &format!("  {}", format!("+{a}").style(p.tone(Tone::Green)));
        }
        if d > 0 {
            line += &format!(" {}", format!("−{d}").style(p.tone(Tone::Red)));
        }
    }
    println!("{line}");
}

/// `M  src/core/status.rs      +20 −3  ■■■■■■`
fn file_row(f: &FileChange, c: &Columns, p: &Paint) {
    let (letter, tone) = kind(f.kind);
    let name = match &f.from {
        Some(from) => format!(
            "{} {} {}",
            path_text(from, Tone::Muted, p),
            "→".style(p.tone(Tone::Faint)),
            path_text(&f.path, Tone::Text, p)
        ),
        None => path_text(&f.path, if f.kind == ChangeKind::Deleted { Tone::Muted } else { Tone::Text }, p),
    };
    let pad = " ".repeat(c.name_w.saturating_sub(display_name(f).chars().count()));

    let mut tail = String::new();
    match f.size {
        Some(Size::Lines { added, removed }) => {
            let plus = if added > 0 { format!("+{added}") } else { String::new() };
            let minus = if removed > 0 { format!("−{removed}") } else { String::new() };
            tail += &format!("{:>w$}", plus, w = c.add_w).style(p.tone(Tone::Green)).to_string();
            if c.del_w > 0 {
                tail += " ";
                tail += &format!("{:<w$}", minus, w = c.del_w).style(p.tone(Tone::Red)).to_string();
            }
            tail += &format!("  {}", bar(added, removed, c.biggest, p));
        }
        Some(Size::Binary) => tail += &"binary".style(p.tone(Tone::Faint)).to_string(),
        None => {}
    }
    let letter = letter.style(p.tone_bold(tone));
    if tail.is_empty() {
        println!("  {letter}  {name}");
    } else {
        println!("  {letter}  {name}{pad}  {tail}");
    }
}

/// `■■■■□` sized against the biggest change: green for added lines, red for removed.
fn bar(added: u32, removed: u32, biggest: u32, p: &Paint) -> String {
    let total = added + removed;
    if total == 0 || biggest == 0 {
        return String::new();
    }
    let both = added > 0 && removed > 0;
    // At least one block of each colour when both kinds of change are there.
    let len = ((total as usize * BAR).div_ceil(biggest as usize)).clamp(if both { 2 } else { 1 }, BAR);
    let green = ((added as usize * len) as f64 / total as f64).round() as usize;
    let green = if both { green.clamp(1, len - 1) } else { green };
    let red = len - green;
    format!("{}{}", "■".repeat(green).style(p.tone(Tone::Green)), "■".repeat(red).style(p.tone(Tone::Red)))
}

/// `src/core/` faint, `status.rs` bright.
fn path_text(path: &str, tone: Tone, p: &Paint) -> String {
    let trimmed = path.trim_end_matches('/');
    let (dir, file) = match trimmed.rfind('/') {
        Some(i) => trimmed.split_at(i + 1),
        None => ("", trimmed),
    };
    let slash = if path.ends_with('/') { "/" } else { "" };
    format!("{}{}", dir.style(p.tone(Tone::Muted)), format!("{file}{slash}").style(p.tone(tone)))
}

/// The path as shown, without colour, for lining up columns.
fn display_name(f: &FileChange) -> String {
    match &f.from {
        Some(from) => format!("{from} → {}", f.path),
        None => f.path.clone(),
    }
}

fn kind(k: ChangeKind) -> (&'static str, Tone) {
    let tone = match k {
        ChangeKind::Added => Tone::Green,
        ChangeKind::Modified => Tone::Yellow,
        ChangeKind::Deleted => Tone::Red,
        ChangeKind::Renamed | ChangeKind::Copied => Tone::Purple,
        ChangeKind::TypeChange => Tone::Blue,
    };
    (mark(k), tone)
}

pub fn mark(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "A",
        ChangeKind::Modified => "M",
        ChangeKind::Deleted => "D",
        ChangeKind::Renamed => "R",
        ChangeKind::Copied => "C",
        ChangeKind::TypeChange => "T",
    }
}

#[cfg(test)]
mod tests {
    use super::bar;
    use crate::render::Paint;

    #[test]
    fn bars() {
        let p = Paint { on: false, rgb: false };
        assert_eq!(bar(0, 0, 10, &p), "");
        assert_eq!(bar(60, 0, 60, &p), "■".repeat(10));
        // Tiny next to a big change: still visible.
        assert_eq!(bar(1, 0, 1000, &p), "■");
        // Both kinds, tiny: one block each (used to panic).
        assert_eq!(bar(1, 1, 1000, &p), "■■");
        assert_eq!(bar(3, 3, 6, &p).chars().count(), 10);
    }
}

use owo_colors::{OwoColorize, Style};

use super::Paint;
use crate::core::status::{ChangeKind, FileChange, Status};

pub fn print(st: &Status, p: &Paint) {
    print_header(st, p);

    let clean = st.staged.is_empty()
        && st.unstaged.is_empty()
        && st.untracked.is_empty()
        && st.conflicted.is_empty();
    if clean {
        println!("\n  {}", "nothing to commit, working tree clean".style(p.dim()));
        return;
    }

    if !st.conflicted.is_empty() {
        section("Conflicts", st.conflicted.len(), Style::new().red(), p);
        for path in &st.conflicted {
            println!("  {}  {}", "!".style(p.red()), path.style(p.red()));
        }
    }
    if !st.staged.is_empty() {
        section("Staged", st.staged.len(), Style::new().green(), p);
        files(&st.staged, p.green(), p);
    }
    if !st.unstaged.is_empty() {
        section("Unstaged", st.unstaged.len(), Style::new().yellow(), p);
        files(&st.unstaged, p.yellow(), p);
    }
    if !st.untracked.is_empty() {
        section("Untracked", st.untracked.len(), Style::new().dimmed(), p);
        for path in &st.untracked {
            println!("  {}  {}", "?".style(p.dim()), path);
        }
    }
}

fn print_header(st: &Status, p: &Paint) {
    let mut line = String::new();
    match &st.branch {
        Some(b) => line += &format!("{}", b.style(p.head())),
        None => line += &format!("{}", "detached".style(p.yellow())),
    }
    if let Some(h) = &st.head {
        line += &format!(" {}", h.style(p.dim()));
    } else {
        line += &format!(" {}", "(no commits yet)".style(p.dim()));
    }
    if let Some(up) = &st.upstream {
        line += &format!("  {} {}", "→".style(p.dim()), up.name.style(p.dim()));
        match (up.ahead, up.behind) {
            (0, 0) => line += &format!(" {}", "up to date".style(p.dim())),
            (a, b) => {
                if a > 0 {
                    line += &format!(" {}", format!("↑{a}").style(p.green()));
                }
                if b > 0 {
                    line += &format!(" {}", format!("↓{b}").style(p.red()));
                }
            }
        }
    } else if st.branch.is_some() {
        line += &format!("  {}", "no upstream".style(p.dim()));
    }
    if st.stashes > 0 {
        let s = if st.stashes == 1 { "" } else { "es" };
        line += &format!("  {}", format!("{} stash{s}", st.stashes).style(p.magenta()));
    }
    if let Some(op) = &st.in_progress {
        line += &format!("  {}", format!("[{op} in progress]").style(p.pick(Style::new().red().bold())));
    }
    println!("{line}");
}

/// `color` is a plain style (e.g. `Style::new().green()`); it is only applied when colour is on.
fn section(title: &str, n: usize, color: Style, p: &Paint) {
    println!("\n{} {}", title.style(p.pick(color.bold())), format!("({n})").style(p.dim()));
}

fn files(list: &[FileChange], style: Style, p: &Paint) {
    for f in list {
        let mark = match f.kind {
            ChangeKind::Added => "A",
            ChangeKind::Modified => "M",
            ChangeKind::Deleted => "D",
            ChangeKind::Renamed => "R",
            ChangeKind::Copied => "C",
            ChangeKind::TypeChange => "T",
        };
        match &f.from {
            Some(from) => println!(
                "  {}  {} {} {}",
                mark.style(style),
                from.style(p.dim()),
                "→".style(p.dim()),
                f.path
            ),
            None => println!("  {}  {}", mark.style(style), f.path),
        }
    }
}

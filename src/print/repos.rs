//! `gg repos`: every repo in a folder, one line each.

use owo_colors::OwoColorize;

use super::Paint;
use crate::fmt::relative_time;
use crate::git::repos::Repo;

pub fn print(repos: &[Repo], p: &Paint) {
    super::out(lines(repos, p));
}

pub fn lines(repos: &[Repo], p: &Paint) -> Vec<String> {
    if repos.is_empty() {
        return vec!["no git repos in this folder".style(p.dim()).to_string()];
    }
    let now = crate::fmt::now();
    let name_w = repos.iter().map(|r| r.name.chars().count()).max().unwrap_or(0);
    let branch = |r: &Repo| match &r.state {
        Ok(s) => s.status.branch.clone().unwrap_or_else(|| "detached".into()),
        Err(_) => String::new(),
    };
    let branch_w = repos.iter().map(|r| branch(r).chars().count()).max().unwrap_or(0);

    let (mut dirty, mut unpushed) = (0, 0);
    let mut out = Vec::new();
    for r in repos {
        let name_pad = " ".repeat(name_w - r.name.chars().count());
        let s = match &r.state {
            Ok(s) => s,
            Err(e) => {
                out.push(format!("{}{name_pad}  {}", r.name.style(p.red()), e.style(p.red())));
                continue;
            }
        };
        let st = &s.status;
        let b = branch(r);
        let b_pad = " ".repeat(branch_w - b.chars().count());
        let b_shown = match &st.branch {
            Some(_) => b.style(p.pick(owo_colors::Style::new().cyan())).to_string(),
            None => b.style(p.yellow()).to_string(),
        };
        let age = s.last_commit.map(|t| relative_time(t, now)).unwrap_or_default();

        // What needs attention, most important first.
        let mut notes: Vec<String> = Vec::new();
        if let Some(op) = &st.in_progress {
            let op = if op.starts_with("rebasing") { op.clone() } else { format!("{op} in progress") };
            notes.push(op.style(p.red()).to_string());
        }
        if !st.conflicted.is_empty() {
            notes.push(format!("{} conflicts", st.conflicted.len()).style(p.red()).to_string());
        }
        let changed = st.staged.len() + st.unstaged.len();
        if changed > 0 {
            notes.push(format!("{changed} changed").style(p.yellow()).to_string());
        }
        if !st.untracked.is_empty() {
            notes.push(format!("{} untracked", st.untracked.len()).style(p.dim()).to_string());
        }
        if changed > 0 || !st.conflicted.is_empty() {
            dirty += 1;
        }
        match &st.upstream {
            Some(up) if up.name.ends_with("(gone)") => notes.push("upstream gone".style(p.red()).to_string()),
            Some(up) => {
                if up.ahead > 0 {
                    unpushed += 1;
                    notes.push(format!("↑{}", up.ahead).style(p.green()).to_string());
                }
                if up.behind > 0 {
                    notes.push(format!("↓{}", up.behind).style(p.red()).to_string());
                }
            }
            None if st.branch.is_some() && st.head.is_some() => {
                notes.push("no upstream".style(p.dim()).to_string());
            }
            None => {}
        }
        if st.stashes > 0 {
            notes.push(format!("{} stash", st.stashes).style(p.magenta()).to_string());
        }
        let notes = if notes.is_empty() { "clean".style(p.dim()).to_string() } else { notes.join("  ") };

        out.push(format!(
            "{}{name_pad}  {b_shown}{b_pad}  {:>7}  {notes}",
            r.name.style(p.pick(owo_colors::Style::new().bold())),
            age.style(p.dim())
        ));
    }
    out.push(format!(
        "\n{}",
        format!("{} repos · {dirty} with changes · {unpushed} with unpushed commits", repos.len()).style(p.dim())
    ));
    out
}

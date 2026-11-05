//! Reads and writes git's rebase todo file (`.git/rebase-merge/git-rebase-todo`).

use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl Action {
    pub const ALL: [Action; 6] = [Action::Pick, Action::Reword, Action::Edit, Action::Squash, Action::Fixup, Action::Drop];

    pub fn word(self) -> &'static str {
        match self {
            Action::Pick => "pick",
            Action::Reword => "reword",
            Action::Edit => "edit",
            Action::Squash => "squash",
            Action::Fixup => "fixup",
            Action::Drop => "drop",
        }
    }

    /// Key that sets this action in the editor (also git's short form).
    pub fn key(self) -> char {
        self.word().chars().next().unwrap()
    }

    /// Melds into the commit above it.
    pub fn melds(self) -> bool {
        matches!(self, Action::Squash | Action::Fixup)
    }

    fn parse(word: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.word() == word || word.len() == 1 && a.key().to_string() == word)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// pick/reword/edit/squash/fixup/drop of one commit.
    Commit { action: Action, hash: String, subject: String },
    /// Anything else (exec, break, label, update-ref, `fixup -C`...), kept as written.
    Other(String),
}

/// Parses the todo file, dropping comments and blank lines (git ignores them anyway).
pub fn parse(text: &str) -> Vec<Line> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut parts = l.splitn(3, ' ');
            let word = parts.next().unwrap_or_default();
            let hash = parts.next().unwrap_or_default();
            // Newer git writes `pick <hash> # <subject>`; git only reads the hash.
            let subject = parts.next().unwrap_or_default();
            let subject = subject.strip_prefix("# ").unwrap_or(subject);
            match Action::parse(word) {
                // `fixup -C <hash>` and friends carry flags; keep them verbatim.
                Some(action) if !hash.is_empty() && !hash.starts_with('-') => Line::Commit {
                    action,
                    hash: hash.to_string(),
                    subject: subject.to_string(),
                },
                _ => Line::Other(l.to_string()),
            }
        })
        .collect()
}

pub fn render(lines: &[Line]) -> String {
    let mut out = String::new();
    for l in lines {
        match l {
            Line::Commit { action, hash, subject } => {
                out += &format!("{} {hash} # {subject}\n", action.word());
            }
            Line::Other(raw) => {
                out += raw;
                out.push('\n');
            }
        }
    }
    out
}

/// Why git would reject this todo list, if it would.
pub fn problem(lines: &[Line]) -> Option<String> {
    let first = lines.iter().find_map(|l| match l {
        Line::Commit { action, .. } if *action != Action::Drop => Some(*action),
        _ => None,
    })?;
    first
        .melds()
        .then(|| format!("the oldest commit can't be '{}': there's no older commit to fold it into", first.word()))
}

pub fn read_file(path: &std::path::Path) -> Result<Vec<Line>> {
    Ok(parse(&std::fs::read_to_string(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let text = "pick a1b2c3d # Add thing\n# comment\n\nf 9f8e7d6 fix typo\nexec cargo test\nfixup -C 1234567 msg\n";
        let lines = parse(text);
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[1],
            Line::Commit { action: Action::Fixup, hash: "9f8e7d6".into(), subject: "fix typo".into() }
        );
        assert_eq!(lines[3], Line::Other("fixup -C 1234567 msg".into()));
        assert_eq!(
            render(&lines),
            "pick a1b2c3d # Add thing\nfixup 9f8e7d6 # fix typo\nexec cargo test\nfixup -C 1234567 msg\n"
        );
    }

    #[test]
    fn first_commit_cannot_meld() {
        assert!(problem(&parse("squash a1b2c3d x\npick 9f8e7d6 y")).is_some());
        assert!(problem(&parse("drop a1b2c3d x\nfixup 9f8e7d6 y")).is_some());
        assert!(problem(&parse("pick a1b2c3d x\nfixup 9f8e7d6 y")).is_none());
    }
}

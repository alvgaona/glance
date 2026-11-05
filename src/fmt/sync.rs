//! How far a branch is from another: as arrows, or in words.

/// `↑2 ↓1`, `↑2`, or empty when in sync.
pub fn arrows(ahead: usize, behind: usize) -> String {
    match (ahead, behind) {
        (0, 0) => String::new(),
        (a, 0) => format!("↑{a}"),
        (0, b) => format!("↓{b}"),
        (a, b) => format!("↑{a} ↓{b}"),
    }
}

/// "up to date", "2 to push", "1 to pull", "differs: 2 to push, 1 to pull".
pub fn words(ahead: usize, behind: usize) -> String {
    match (ahead, behind) {
        (0, 0) => "up to date".into(),
        (a, 0) => format!("{a} to push"),
        (0, b) => format!("{b} to pull"),
        (a, b) => format!("differs: {a} to push, {b} to pull"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both() {
        assert_eq!(arrows(0, 0), "");
        assert_eq!(arrows(2, 1), "↑2 ↓1");
        assert_eq!(words(0, 3), "3 to pull");
    }
}

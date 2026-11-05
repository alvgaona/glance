//! Folder paths, short enough for a list or a message.

use std::path::Path;

/// `/Users/me/git/app` -> `~/git/app`.
pub fn tilde(p: &Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && s.starts_with(&home) => format!("~{}", &s[home.len()..]),
        _ => s,
    }
}

/// `~/git/x`; past 40 characters only the last two parts: `…/git/x`.
pub fn short(p: &Path) -> String {
    let s = tilde(p);
    if s.chars().count() <= 40 {
        return s;
    }
    let parts: Vec<&str> = s.rsplitn(3, '/').collect();
    match parts.as_slice() {
        [last, parent, _] => format!("…/{parent}/{last}"),
        _ => s,
    }
}

/// Just the folder name when `p` sits next to `main` (`app-feat`), else `tilde`.
pub fn beside(p: &Path, main: Option<&Path>) -> String {
    let next_to = main.is_some_and(|m| m == p || m.parent() == p.parent());
    match p.file_name() {
        Some(name) if next_to => name.to_string_lossy().into_owned(),
        _ => tilde(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beside_main() {
        let main = Path::new("/r/app");
        assert_eq!(beside(Path::new("/r/app-fix"), Some(main)), "app-fix");
        assert_eq!(beside(Path::new("/r/app"), Some(main)), "app");
        assert_eq!(beside(Path::new("/elsewhere/x"), Some(main)), "/elsewhere/x");
    }
}

//! What screens remember between runs: small text files in `~/.local/state/glance/`
//! (or under `$XDG_STATE_HOME`). Best effort: nothing here is worth failing for.

use std::path::PathBuf;

fn path(name: &str) -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("glance").join(name))
}

pub fn read(name: &str) -> Option<String> {
    std::fs::read_to_string(path(name)?).ok()
}

pub fn write(name: &str, text: &str) {
    let Some(p) = path(name) else { return };
    let _ = p.parent().map(std::fs::create_dir_all);
    let _ = std::fs::write(p, text);
}

/// The sidebar on wide screens: on unless it was turned off with `i`.
pub fn sidebar() -> bool {
    read("sidebar").is_none_or(|t| t.trim() != "off")
}

pub fn set_sidebar(on: bool) {
    write("sidebar", if on { "on\n" } else { "off\n" });
}

//! The diff screen's options, kept between runs in `~/.local/state/glance/diff`
//! (or under `$XDG_STATE_HOME`). Side by side isn't kept: it follows the window.

use std::path::PathBuf;

use super::App;

fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("glance/diff"))
}

pub(super) fn load(app: &mut App) {
    if let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) {
        read(&text, app);
    }
}

/// Best effort: options not kept are no reason to fail.
pub(super) fn save(app: &App) {
    let Some(p) = path() else { return };
    let _ = p.parent().map(std::fs::create_dir_all);
    let _ = std::fs::write(p, text(app));
}

fn text(app: &App) -> String {
    format!(
        "numbers={}\nwhitespace={}\ncontext={}\nfull={}\nhide_files={}\n",
        app.numbers, app.cut.whitespace, app.cut.context, app.cut.full, app.hide_files
    )
}

/// `key=value` lines; unknown keys and bad values are skipped.
fn read(text: &str, app: &mut App) {
    for (key, value) in text.lines().filter_map(|l| l.split_once('=')) {
        let on = value.trim() == "true";
        match key.trim() {
            "numbers" => app.numbers = on,
            "whitespace" => app.cut.whitespace = on,
            "full" => app.cut.full = on,
            "hide_files" => app.hide_files = on,
            "context" => {
                if let Ok(n) = value.trim().parse::<u32>() {
                    app.cut.context = n.min(1000);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::compare::Target;
    use crate::git::run::Scratch;

    #[test]
    fn options_round_trip() {
        let s = Scratch::new("diff-saved");
        s.commit("a.txt", "1\n");
        let mut app = super::super::start(s.repo(), Some(s.0.clone()), Target::Unstaged, false).unwrap();
        (app.numbers, app.cut.whitespace, app.cut.context, app.hide_files) = (false, true, 10, true);
        let saved = text(&app);
        let mut back = super::super::start(s.repo(), Some(s.0.clone()), Target::Unstaged, false).unwrap();
        read(&format!("{saved}junk\ncontext=lots\n"), &mut back);
        assert_eq!(text(&back), saved);
    }
}

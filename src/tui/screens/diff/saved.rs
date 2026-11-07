//! The diff screen's options, kept between runs in `~/.local/state/glance/diff`.
//! Side by side isn't kept: it follows the window. The sidebar is kept for every
//! screen, in `crate::tui::state`.

use super::App;
use crate::tui::state;

pub(super) fn load(app: &mut App) {
    if let Some(text) = state::read("diff") {
        read(&text, app);
    }
}

pub(super) fn save(app: &App) {
    state::write("diff", &text(app));
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
        let mut app = super::super::start(s.repo(), Some(s.0.clone()), Target::Unstaged).unwrap();
        (app.numbers, app.cut.whitespace, app.cut.context, app.hide_files) = (false, true, 10, true);
        let saved = text(&app);
        let mut back = super::super::start(s.repo(), Some(s.0.clone()), Target::Unstaged).unwrap();
        read(&format!("{saved}junk\ncontext=lots\n"), &mut back);
        assert_eq!(text(&back), saved);
    }
}

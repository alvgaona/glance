//! Keys give effects on a repo with only its main worktree.

use super::*;
use crate::git::run::Scratch;
use crate::tui::app::{press, type_in};

fn key(app: &mut App, code: KeyCode) -> Flow<Option<PathBuf>, Effect> {
    app.key(press(code))
}

#[test]
fn keys_give_effects() {
    let s = Scratch::new("worktree-keys");
    s.commit("a.txt", "one\n");
    let repo = s.repo();
    let mut app = start(&repo).unwrap();

    assert!(matches!(key(&mut app, KeyCode::Enter), Flow::Run(Effect::Go(p)) if p.ends_with(s.0.file_name().unwrap())));
    // The main worktree can't be removed: a toast, no dialog.
    assert!(matches!(key(&mut app, KeyCode::Char('d')), Flow::Stay));
    assert!(app.confirm.is_none() && app.toast.is_some());

    key(&mut app, KeyCode::Char('n'));
    type_in(&mut app, "feat");
    assert!(matches!(key(&mut app, KeyCode::Enter), Flow::Run(Effect::Create(b)) if b == "feat"));
    assert!(matches!(key(&mut app, KeyCode::Char('q')), Flow::Done(None)));
}

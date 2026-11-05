//! Keys give effects on a repo with one stash.

use super::*;
use crate::git::run::Scratch;
use crate::tui::app::press;

fn scratch(name: &str) -> Scratch {
    let s = Scratch::new(name);
    s.commit("a.txt", "one\n");
    s.write("a.txt", "two\n");
    s.git(&["stash", "push", "-q", "-m", "wip on a"]);
    s
}

fn key(app: &mut App, code: KeyCode) -> Flow<(), Effect> {
    app.key(press(code))
}

#[test]
fn apply_pop_and_drop() {
    let s = scratch("stash-keys");
    let repo = s.repo();
    let mut app = start(&repo).unwrap();
    assert!(app.focus == Pane::Stashes);
    assert!(matches!(key(&mut app, KeyCode::Char('a')), Flow::Run(Effect::Apply { pop: false, .. })));
    assert!(matches!(key(&mut app, KeyCode::Char('p')), Flow::Run(Effect::Apply { pop: true, .. })));

    // Drop asks first; the dialog holds the stashes to drop.
    assert!(matches!(key(&mut app, KeyCode::Char('d')), Flow::Stay));
    assert!(matches!(&app.confirm, Some((_, Effect::Drop(list))) if list.len() == 1));
    key(&mut app, KeyCode::Char('n'));
    assert!(app.confirm.is_none());
}

#[test]
fn branch_suggests_a_name_from_the_message() {
    let s = scratch("stash-branch");
    let repo = s.repo();
    let mut app = start(&repo).unwrap();
    key(&mut app, KeyCode::Char('b'));
    let Flow::Run(Effect::Branch { name, .. }) = key(&mut app, KeyCode::Enter) else { panic!("enter should branch") };
    assert_eq!(name, "stash/wip-on-a");
}

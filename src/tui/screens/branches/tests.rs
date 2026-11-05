//! Keys give effects; deleting asks first and runs git on the terminal.

use super::*;
use crate::git::run::Scratch;
use crate::tui::app::{ctrl, press, type_in};

/// main checked out, and topic at the same commit.
fn scratch(name: &str) -> Scratch {
    let s = Scratch::new(name);
    s.commit("a.txt", "one\n");
    s.git(&["branch", "topic"]);
    s
}

fn key(app: &mut App, code: KeyCode) -> Flow<(), Effect> {
    app.key(press(code))
}

fn select(app: &mut App, name: &str) {
    app.selected = app.visible.iter().position(|&i| app.data.list[i].name == name).unwrap();
}

#[test]
fn starts_on_the_current_branch_and_copies_it() {
    let s = scratch("branches-copy");
    let repo = s.repo();
    let mut app = start(&repo).unwrap();
    assert!(matches!(key(&mut app, KeyCode::Char('y')), Flow::Run(Effect::Copy(n)) if n == "main"));
    // Enter on it: already there.
    assert!(matches!(key(&mut app, KeyCode::Enter), Flow::Stay));
    select(&mut app, "topic");
    assert!(matches!(key(&mut app, KeyCode::Enter), Flow::Run(Effect::Checkout(n)) if n == "topic"));
}

#[test]
fn rename_types_the_new_name() {
    let s = scratch("branches-rename");
    let repo = s.repo();
    let mut app = start(&repo).unwrap();
    key(&mut app, KeyCode::Char('r'));
    assert!(matches!(app.key(ctrl('u')), Flow::Stay));
    type_in(&mut app, "trunk");
    let Flow::Run(Effect::Rename { from, to }) = key(&mut app, KeyCode::Enter) else { panic!("enter should rename") };
    assert_eq!((from.as_str(), to.as_str()), ("main", "trunk"));
}

#[test]
fn delete_asks_then_runs_on_the_terminal() {
    let s = scratch("branches-delete");
    let repo = s.repo();
    let mut app = start(&repo).unwrap();

    // The current branch can't go.
    let Flow::Run(e @ Effect::AskDeleteLocal) = key(&mut app, KeyCode::Char('d')) else { panic!() };
    assert!(matches!(app.apply(e).unwrap(), Flow::Stay));
    assert!(app.confirm.is_none());
    assert!(app.toast.as_ref().unwrap().text().contains("checked out"));

    select(&mut app, "topic");
    let Flow::Run(e) = key(&mut app, KeyCode::Char('d')) else { panic!() };
    app.apply(e).unwrap();
    assert!(app.confirm.is_some());
    let Flow::Run(Effect::InTerminal(cmds)) = key(&mut app, KeyCode::Char('y')) else { panic!("yes should delete") };
    assert!(App::in_terminal(&Effect::InTerminal(cmds.clone())));
    assert!(cmds.iter().all(|c| c.contains(&"topic".to_string())) && cmds.len() == 1);
}

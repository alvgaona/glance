//! Keys give effects and options; the diff is drawn without delta so the tests don't
//! need it installed.

use super::*;
use crate::git::run::Scratch;
use crate::tui::app::{ctrl, press, type_in};

fn key(app: &mut App, code: KeyCode) -> Flow<(), Effect> {
    app.key(press(code))
}

/// a.txt changed and unstaged, b.txt added and staged.
fn screen(name: &str) -> (Scratch, App) {
    let s = Scratch::new(name);
    s.commit("a.txt", "1\n2\n");
    s.write("a.txt", "1\n2\n3\n");
    s.write("b.txt", "new\n");
    s.git(&["add", "b.txt"]);
    let app = start(s.repo(), Some(s.0.clone()), Target::Unstaged, false).unwrap();
    (s, app)
}

#[test]
fn switches_what_is_compared() {
    let (_s, mut app) = screen("diff-target");
    assert_eq!(app.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
    assert!(matches!(key(&mut app, KeyCode::Char('2')), Flow::Run(Effect::Load)));
    app.apply(Effect::Load).unwrap();
    assert_eq!(app.files[0].path, "b.txt");
    // Already there: nothing to load.
    assert!(matches!(key(&mut app, KeyCode::Char('2')), Flow::Stay));
    key(&mut app, KeyCode::Char('3'));
    app.apply(Effect::Load).unwrap();
    assert_eq!(app.files.len(), 2);
}

#[test]
fn renders_the_picked_file_and_its_options() {
    let (_s, mut app) = screen("diff-render");
    app.width = 80;
    app.tick().unwrap();
    // The background render may outlast tick's short wait.
    while app.job.is_some() {
        std::thread::sleep(Duration::from_millis(5));
        app.poll();
    }
    let lines = &app.cache[&app.key_for(80).unwrap()];
    assert!(lines.iter().any(|l| l.to_string().contains("+3")), "{lines:?}");

    // Options change the key, so the next tick renders again.
    let before = app.key_for(80);
    key(&mut app, KeyCode::Char('+'));
    assert_eq!(app.cut.context, 5);
    key(&mut app, KeyCode::Char('f'));
    assert!(app.cut.full);
    assert_ne!(app.key_for(80), before);
    // Off a wide-pane default, `s` fixes the split.
    assert!(app.split_at(130));
    key(&mut app, KeyCode::Char('s'));
    assert_eq!(app.split, Some(true));
}

#[test]
fn filters_copies_and_edits() {
    let (_s, mut app) = screen("diff-keys");
    key(&mut app, KeyCode::Char('3'));
    app.apply(Effect::Load).unwrap();
    key(&mut app, KeyCode::Char('/'));
    assert!(app.typing());
    type_in(&mut app, "b.");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.visible.len(), 1);
    assert!(matches!(key(&mut app, KeyCode::Char('y')), Flow::Run(Effect::Copy(p)) if p == "b.txt"));
    assert!(matches!(key(&mut app, KeyCode::Char('e')), Flow::Run(Effect::Edit(p)) if p == "b.txt"));
    assert!(matches!(key(&mut app, KeyCode::Char('w')), Flow::Run(Effect::Load)));
    assert!(matches!(app.key(ctrl('r')), Flow::Run(Effect::Load)));
    assert!(matches!(key(&mut app, KeyCode::Char('q')), Flow::Done(())));
}

//! Keys give effects and options; the diff is drawn without delta so the tests don't
//! need it installed.

use ratatui::{Terminal, backend::TestBackend};

use super::*;
use crate::git::run::Scratch;
use crate::tui::app::{ctrl, press, type_in};

fn key(app: &mut App, code: KeyCode) -> Flow<(), Effect> {
    app.key(press(code))
}

/// a.txt: 30 lines, lines 2 and 25 changed, unstaged. b.txt added and staged.
fn screen(name: &str) -> (Scratch, App) {
    let s = Scratch::new(name);
    let old: String = (1..=30).map(|i| format!("{i}\n")).collect();
    s.commit("a.txt", &old);
    s.write("a.txt", &old.replace("\n2\n", "\ntwo\n").replace("25\n", "twenty-five\n"));
    s.write("b.txt", "new\n");
    s.git(&["add", "b.txt"]);
    let app = start(s.repo(), Some(s.0.clone()), Target::Unstaged, false).unwrap();
    (s, app)
}

/// Draws at `w`×`h` as the event loop does (draw, tick, draw) once the renders are in;
/// gives the screen's text.
fn draw(app: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    app.tick().unwrap();
    while app.job.is_some() {
        std::thread::sleep(Duration::from_millis(5));
        app.poll();
    }
    t.draw(|f| app.draw(f)).unwrap();
    let cells: Vec<&str> = t.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    cells.chunks(w as usize).map(|r| r.concat()).collect::<Vec<_>>().join("\n")
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
fn draws_wide_and_narrow() {
    let (s, mut app) = screen("diff-draw");
    s.write("loose.txt", "x\n");
    app.reload().unwrap();
    let wide = draw(&mut app, 150, 40);
    assert!(wide.contains("1 untracked, not shown"), "{wide}");
    // The list and the diff side by side on one row.
    assert!(wide.lines().any(|l| l.contains("M  a.txt") && l.matches("a.txt").count() == 2), "{wide}");
    assert!(wide.contains("+twenty-five"), "{wide}");
    // Resized: the list goes on top, and the diff is drawn again for the new width.
    let narrow = draw(&mut app, 80, 30);
    assert!(narrow.contains("+two"), "{narrow}");
    assert!(app.cache.keys().any(|k| k.width < 80) && app.cache.keys().any(|k| k.width > 80));
}

#[test]
fn jumps_between_hunks_and_edits_there() {
    let (_s, mut app) = screen("diff-hunks");
    draw(&mut app, 100, 12);
    let hunks = app.on_screen().unwrap().hunks.clone();
    assert_eq!(hunks.iter().map(|h| h.1).collect::<Vec<_>>(), [1, 22]);
    assert!(matches!(key(&mut app, KeyCode::Char('e')), Flow::Run(Effect::Edit(p, Some(1))) if p == "a.txt"));
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(app.detail_scroll.top() as usize, hunks[1].0);
    assert!(matches!(key(&mut app, KeyCode::Char('e')), Flow::Run(Effect::Edit(_, Some(22)))));
    key(&mut app, KeyCode::Char('N'));
    assert_eq!(app.detail_scroll.top(), 0);
}

#[test]
fn options_and_the_next_file() {
    let (_s, mut app) = screen("diff-options");
    key(&mut app, KeyCode::Char('3'));
    app.apply(Effect::Load).unwrap();
    draw(&mut app, 100, 20);
    // The next file is ready before `j`.
    assert!(app.key_at(1, app.width).is_some_and(|k| app.cache.contains_key(&k)));

    // Options change the key, so the next tick renders again.
    let before = app.key_for(80);
    key(&mut app, KeyCode::Char('+'));
    assert_eq!(app.cut.context, 5);
    key(&mut app, KeyCode::Char('f'));
    assert!(app.cut.full);
    assert_ne!(app.key_for(80), before);

    // delta's side by side, but not where lines would wrap.
    app.prefer_split = Some(true);
    assert!(!app.split_at(90) && app.split_at(110));
    app.prefer_split = None;
    assert!(!app.split_at(110) && app.split_at(130));
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
    assert!(matches!(key(&mut app, KeyCode::Char('e')), Flow::Run(Effect::Edit(p, None)) if p == "b.txt"));
    assert!(matches!(key(&mut app, KeyCode::Char('w')), Flow::Run(Effect::Load)));
    assert!(matches!(app.key(ctrl('r')), Flow::Run(Effect::Load)));
    assert!(matches!(key(&mut app, KeyCode::Char('q')), Flow::Done(())));
}

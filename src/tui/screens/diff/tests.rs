//! Keys give effects and options, and what the screen draws.

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
    let app = start(s.repo(), Some(s.0.clone()), Target::Unstaged).unwrap();
    (s, app)
}

/// Draws at `w`×`h` as the event loop does (draw, tick, draw) once the renders are in;
/// gives the screen's text.
fn draw(app: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    app.tick().unwrap();
    while app.job.is_some() || app.history_job.is_some() {
        std::thread::sleep(Duration::from_millis(5));
        app.poll();
        app.load_history();
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
    // Wide enough for side by side: the old line next to the one replacing it.
    let wider = draw(&mut app, 220, 40);
    assert!(wider.lines().any(|l| l.contains("25 -25") && l.contains("25 +twenty-five")), "{wider}");
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
    assert_eq!(hunks.iter().map(|h| h.line).collect::<Vec<_>>(), [1, 22]);
    assert!(matches!(key(&mut app, KeyCode::Char('e')), Flow::Run(Effect::Edit(p, Some(1))) if p == "a.txt"));
    key(&mut app, KeyCode::Char('n'));
    assert_eq!(app.detail_scroll.top() as usize, hunks[1].at);
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

    // Side by side where there's room.
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

#[test]
fn the_sidebar_shows_hunks_and_history_on_wide_screens() {
    let (_s, mut app) = screen("diff-side");
    assert!(!draw(&mut app, 150, 30).contains("history"));
    let wide = draw(&mut app, 200, 30);
    assert!(app.side_open);
    assert!(wide.contains("hunks   2"), "{wide}");
    // The commit that made a.txt, by `Scratch::commit`.
    assert!(wide.lines().any(|l| l.contains("history")) && wide.contains(" a.txt"), "{wide}");
    key(&mut app, KeyCode::Char('i'));
    assert!(!draw(&mut app, 200, 30).contains("history"));
}

#[test]
fn long_lines_scroll_sideways() {
    let (s, mut app) = screen("diff-sideways");
    let text = draw(&mut app, 80, 20);
    assert!(!app.detail_scroll.wide());
    assert!(!text.contains("long lines"), "{text}");
    s.write("a.txt", &format!("{}QZ\n", "x".repeat(200)));
    app.reload().unwrap();
    let text = draw(&mut app, 160, 50);
    assert!(app.detail_scroll.wide() && text.contains("long lines") && !text.contains("QZ"), "{text}");
    // Never past the end of the longest line.
    (0..50).for_each(|_| drop(key(&mut app, KeyCode::Right)));
    let text = draw(&mut app, 160, 50);
    // The line number and `+` stay in front.
    assert!(text.lines().any(|l| l.contains("1 +x") && l.contains("xQZ")), "{text}");
}

#[test]
fn side_by_side_colours_more_on_its_own_sides() {
    let patch: Vec<PatchLine> = (1..=2000)
        .flat_map(|n| {
            let text = format!("let x = {n};");
            [PatchLine::Removed { old: n, text: text.clone() }, PatchLine::Added { new: n, text }]
        })
        .collect();
    let mut lazy = diff::Lazy::new(patch, "a.rs", true);
    lazy.near(0);
    let h = lazy.halves();
    let mut r = Rendered { lines: h.old, hunks: Vec::new(), pins: h.pins, right: h.new, lazy: Some((lazy, true)) };
    // `let` is in the text colour until coloured.
    let coloured = |l: &Line| l.spans[1].style.fg != Some(theme::TEXT);
    assert!(!coloured(&r.lines[1500]));
    // Scrolled down bit by bit: a row here holds two lines, so 1500 rows are 3000 lines.
    for bottom in (300..=1500).step_by(300) {
        r.near(bottom);
    }
    assert!(coloured(&r.lines[1500]) && coloured(&r.right[1500]));
    // Removed lines stay left, added ones right.
    assert!(r.lines[1500].to_string().contains('-') && r.right[1500].to_string().contains('+'));
}

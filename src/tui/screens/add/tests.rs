//! Keys give effects, and `apply` does them, on a scratch repo.

use super::*;
use crate::git::run::Scratch;
use crate::tui::app::{press, type_in};

/// a.txt changed, b.txt clean, new.txt untracked; the cursor on a.txt.
fn scratch(name: &str) -> Scratch {
    let s = Scratch::new(name);
    for (k, v) in [("user.name", "t"), ("user.email", "t@t"), ("commit.gpgsign", "false")] {
        s.git(&["config", k, v]);
    }
    s.commit("a.txt", "one\n");
    s.commit("b.txt", "b\n");
    s.write("a.txt", "two\n");
    s.write("new.txt", "new\n");
    s
}

fn key(app: &mut App, code: KeyCode) -> Flow<(), Effect> {
    app.key(press(code))
}

fn select(app: &mut App, path: &str) {
    app.selected = app.entries.iter().position(|e| e.path == path).unwrap();
}

fn group_of(app: &App, path: &str) -> Group {
    app.entries.iter().find(|e| e.path == path).unwrap().group
}

#[test]
fn space_stages_the_file() {
    let s = scratch("add-space");
    let mut app = start(s.repo()).unwrap();
    assert_eq!(app.current().unwrap().path, "a.txt");
    let Flow::Run(e @ Effect::File { op: FileOp::Add, .. }) = key(&mut app, KeyCode::Char(' ')) else {
        panic!("space should stage")
    };
    app.apply(e).unwrap();
    assert_eq!(group_of(&app, "a.txt"), Group::Staged);
    assert_eq!(app.toast.as_ref().unwrap().text(), "Staged a.txt");
}

#[test]
fn discard_asks_and_the_dialog_holds_the_write() {
    let s = scratch("add-discard");
    let mut app = start(s.repo()).unwrap();
    assert!(matches!(key(&mut app, KeyCode::Char('d')), Flow::Stay));
    assert!(matches!(app.confirm, Some((_, Effect::File { op: FileOp::Discard, .. }))));
    // No: nothing happens.
    assert!(matches!(key(&mut app, KeyCode::Char('n')), Flow::Stay));
    assert!(app.confirm.is_none());
    assert_eq!(std::fs::read_to_string(s.0.join("a.txt")).unwrap(), "two\n");
    // Yes: the dialog's effect runs.
    key(&mut app, KeyCode::Char('d'));
    let Flow::Run(e) = key(&mut app, KeyCode::Char('y')) else { panic!("yes should run the discard") };
    app.apply(e).unwrap();
    assert_eq!(std::fs::read_to_string(s.0.join("a.txt")).unwrap(), "one\n");
}

#[test]
fn hunks_need_the_view_built_by_tick() {
    let s = scratch("add-hunk");
    let mut app = start(s.repo()).unwrap();
    app.tick().unwrap();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::Hunks);
    let Flow::Run(e @ Effect::Hunk { action: stage::HunkAction::Stage, .. }) = key(&mut app, KeyCode::Char(' ')) else {
        panic!("space in the diff should stage the hunk")
    };
    app.apply(e).unwrap();
    assert_eq!(group_of(&app, "a.txt"), Group::Staged);
}

#[test]
fn commit_box_commits_what_is_staged() {
    let s = scratch("add-commit");
    let mut app = start(s.repo()).unwrap();
    // Nothing staged: a toast, no box.
    assert!(matches!(key(&mut app, KeyCode::Char('c')), Flow::Stay));
    assert!(app.commit.is_none() && app.toast.is_some());

    let Flow::Run(e) = key(&mut app, KeyCode::Char(' ')) else { panic!() };
    app.apply(e).unwrap();
    let Flow::Run(e @ Effect::OpenCommit { amend: false }) = key(&mut app, KeyCode::Char('c')) else {
        panic!("c should open the box")
    };
    app.apply(e).unwrap();
    assert!(app.commit.is_some());
    // An empty summary is refused, and the box stays.
    assert!(matches!(key(&mut app, KeyCode::Enter), Flow::Stay));
    assert!(app.commit.is_some());

    type_in(&mut app, "msg");
    let Flow::Run(e @ Effect::Commit { amend: false, edit: false, .. }) = key(&mut app, KeyCode::Enter) else {
        panic!("enter should commit")
    };
    assert!(matches!(app.apply(e).unwrap(), Flow::Stay));
    assert!(app.commit.is_none());
    assert_eq!(crate::git::commit::last_message(&s.repo()).unwrap().trim(), "msg");
    assert!(app.entries.iter().all(|e| e.group != Group::Staged));
}

#[test]
fn editing_a_folder_stays_on_the_screen() {
    let s = scratch("add-edit");
    std::fs::create_dir(s.0.join("dir")).unwrap();
    s.write("dir/f.txt", "f\n");
    let mut app = start(s.repo()).unwrap();
    select(&mut app, "dir/");
    let Flow::Run(e @ Effect::Edit(_)) = key(&mut app, KeyCode::Char('e')) else { panic!("e should edit") };
    // Not a file: no trip to the terminal.
    assert!(matches!(app.apply(e).unwrap(), Flow::Stay));
    assert_eq!(app.toast.as_ref().unwrap().text(), "That's a folder");
    assert!(App::in_terminal(&Effect::EditOnTerminal("a.txt".into())));
}

#[test]
fn a_conflict_with_markers_asks_before_staging() {
    let s = scratch("add-conflict");
    s.git(&["stash", "-q", "-u"]);
    s.git(&["checkout", "-q", "-b", "side"]);
    s.commit("a.txt", "theirs\n");
    s.git(&["checkout", "-q", "main"]);
    s.commit("a.txt", "ours\n");
    // Fails on purpose: that's the conflict.
    let _ = crate::process::run("git", &crate::git::run::args(&["merge", "-q", "side"]), Some(&s.0));

    let mut app = start(s.repo()).unwrap();
    select(&mut app, "a.txt");
    assert!(app.current().unwrap().markers);
    assert!(matches!(key(&mut app, KeyCode::Char(' ')), Flow::Stay));
    assert!(matches!(app.confirm, Some((_, Effect::File { op: FileOp::Add, .. }))));
    key(&mut app, KeyCode::Esc);

    // Fixed by hand: space stages it straight away.
    s.write("a.txt", "both\n");
    app.apply(Effect::Reload).unwrap();
    select(&mut app, "a.txt");
    assert!(matches!(key(&mut app, KeyCode::Char(' ')), Flow::Run(Effect::File { op: FileOp::Add, .. })));
}

#[test]
fn a_line_pick_grows_with_shift_j_and_ends_on_reload() {
    let s = scratch("add-pick");
    s.commit("c.txt", "1\n2\n3\n");
    s.write("c.txt", "a\nb\nc\n");
    let mut app = start(s.repo()).unwrap();
    select(&mut app, "c.txt");
    app.tick().unwrap();
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('v'));
    assert_eq!(app.pick().map(|p| p.range().count()), Some(1));
    // J grows the pick; it doesn't scroll the diff.
    key(&mut app, KeyCode::Char('J'));
    assert_eq!(app.pick().map(|p| p.range().count()), Some(2));

    // After a reload the entry indexes may mean other files: the pick is gone.
    app.apply(Effect::Reload).unwrap();
    assert!(app.pick.is_none());
}

#[test]
fn a_paste_only_types_into_an_open_field() {
    let s = scratch("add-paste");
    let mut app = start(s.repo()).unwrap();
    // On the list a paste is not typed: "dy" would discard a file.
    assert!(!app.typing());
    let Flow::Run(e) = key(&mut app, KeyCode::Char(' ')) else { panic!() };
    app.apply(e).unwrap();
    let Flow::Run(e) = key(&mut app, KeyCode::Char('c')) else { panic!() };
    app.apply(e).unwrap();
    assert!(app.typing());
    // A line break in the paste must not press Enter (that would commit).
    crate::tui::app::paste(&mut app, "fix\nday");
    assert_eq!(app.commit.as_ref().unwrap().message(), "fix day");
}

#[test]
fn unstaging_a_rename_takes_back_both_halves() {
    let s = scratch("add-rename");
    s.git(&["mv", "b.txt", "c.txt"]);
    let mut app = start(s.repo()).unwrap();
    select(&mut app, "c.txt");
    let Flow::Run(e) = key(&mut app, KeyCode::Char(' ')) else { panic!("space should unstage") };
    app.apply(e).unwrap();
    // b.txt is back in the index: nothing is staged, c.txt is new and untracked.
    assert!(app.entries.iter().all(|e| e.group != Group::Staged));
    assert_eq!(group_of(&app, "c.txt"), Group::Untracked);
}

#[test]
fn the_list_is_as_wide_as_its_rows() {
    let s = scratch("add-width");
    let mut app = start(s.repo()).unwrap();
    let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 30)).unwrap();
    t.draw(|f| app.draw(f)).unwrap();
    // Short paths: the minimum, not two fifths of the screen.
    assert_eq!(app.list_area.width, 28 - 3);
}

#[test]
fn the_sidebar_shows_hunks_and_the_next_commit_on_wide_screens() {
    let s = scratch("add-side");
    let mut app = start(s.repo()).unwrap();
    let screen = |app: &mut App, w: u16| {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, 24)).unwrap();
        app.tick().unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        let cells: Vec<&str> = t.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        cells.concat()
    };
    let wide = screen(&mut app, 200);
    for want in ["hunks   1", "top of the file", "next commit", "nothing staged yet", "last commit", "b.txt"] {
        assert!(wide.contains(want), "{want}: {wide}");
    }
    assert!(!screen(&mut app, 150).contains("next commit"));
    key(&mut app, KeyCode::Char('i'));
    assert!(!screen(&mut app, 200).contains("next commit"));
}

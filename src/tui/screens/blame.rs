//! `glance blame <path>`: the file's lines, each with the commit that last changed it.
//! Enter opens that commit on the `glance show` screen.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use crate::fmt::{now, relative_time};
use crate::git::blame::{self, Blame};
use crate::git::commit::{self, Details};
use crate::tui::app::{self, Flow, Screen};
use crate::tui::kit::sidebar::{self, Section};
use crate::tui::kit::{self, Toast, footer, list};
use crate::tui::panels::changes;
use crate::tui::theme;

struct App {
    repo: gix::Repository,
    path: String,
    /// The blame running in the background, until it's in.
    job: Option<mpsc::Receiver<Result<Blame, String>>>,
    blame: Option<Result<Blame, String>>,
    selected: usize,
    scroll: usize,
    /// Each commit read so far, for the sidebar; `None` if it couldn't be read.
    details: HashMap<String, Option<Details>>,
    side: bool,
    side_open: bool,
    list_area: Rect,
    toast: Option<Toast>,
}

enum Effect {
    Copy(String),
    Show(String),
}

pub fn run(repo: gix::Repository, path: &str) -> Result<()> {
    let mut app = start(repo, None, path);
    app.side = crate::tui::state::sidebar();
    app::run(&mut app)?;
    crate::tui::state::set_sidebar(app.side);
    Ok(())
}

/// The screen as it opens: the blame starts on another thread, as big files take a while.
/// It runs in `dir` (`None`: here).
fn start(repo: gix::Repository, dir: Option<PathBuf>, path: &str) -> App {
    let (tx, rx) = mpsc::channel();
    let p = path.to_string();
    std::thread::spawn(move || tx.send(blame::read(dir.as_deref(), &p)));
    App {
        repo,
        path: path.to_string(),
        job: Some(rx),
        blame: None,
        selected: 0,
        scroll: 0,
        details: HashMap::new(),
        side: true,
        side_open: false,
        list_area: Rect::default(),
        toast: None,
    }
}

impl Screen for App {
    type Out = ();
    type Effect = Effect;

    fn draw(&mut self, f: &mut Frame) {
        draw(f, self)
    }
    fn key(&mut self, k: KeyEvent) -> Flow<(), Effect> {
        self.on_key(k)
    }
    fn mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollDown => self.step(3),
            MouseEventKind::ScrollUp => self.step(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(i) = list::row_at(self.list_area, self.scroll, &m) {
                    self.selected = i.min(self.len().saturating_sub(1));
                }
            }
            _ => {}
        }
    }
    fn apply(&mut self, e: Effect) -> Result<Flow<(), Effect>> {
        match e {
            Effect::Copy(id) => {
                crate::clipboard::copy(&id);
                self.toast = Some(Toast::new(format!("Copied {id}"), theme::ACCENT));
            }
            Effect::Show(id) => {
                let target = crate::git::compare::commit(&self.repo, &id)?;
                // On this file, as named in that commit (the path given may be from a subfolder).
                let on = match &self.blame {
                    Some(Ok(b)) => b.commits.get(&id).map(|w| w.path.as_str()),
                    _ => None,
                };
                super::diff::run(self.repo.clone(), target, on)?;
            }
        }
        Ok(Flow::Stay)
    }
    fn in_terminal(e: &Effect) -> bool {
        // Another full screen: this one steps aside while it's up.
        matches!(e, Effect::Show(_))
    }
    fn tick(&mut self) -> Result<()> {
        if let Some(rx) = &self.job
            && let Ok(b) = rx.try_recv()
        {
            self.blame = Some(b);
            self.job = None;
        }
        // The sidebar's commit, read once.
        if self.side_open
            && let Some(id) = self.id().filter(|id| !blame::uncommitted(id)).map(String::from)
            && !self.details.contains_key(&id)
        {
            let d = commit::read(&self.repo, &id).ok();
            self.details.insert(id, d);
        }
        Ok(())
    }
    fn toast(&mut self) -> &mut Option<Toast> {
        &mut self.toast
    }
}

impl App {
    fn lines(&self) -> &[blame::Blamed] {
        match &self.blame {
            Some(Ok(b)) => &b.lines,
            _ => &[],
        }
    }

    fn len(&self) -> usize {
        self.lines().len()
    }

    /// The selected line's commit.
    fn id(&self) -> Option<&str> {
        self.lines().get(self.selected).map(|l| l.id.as_str())
    }

    fn step(&mut self, n: isize) {
        let last = self.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + n).clamp(0, last.max(0)) as usize;
    }

    /// The first line of the next run of another commit (or of the run before).
    fn next_change(&mut self, forward: bool) {
        let lines = self.lines();
        let Some(id) = lines.get(self.selected).map(|l| &l.id) else { return };
        let at = if forward {
            lines[self.selected..].iter().position(|l| &l.id != id).map(|i| self.selected + i)
        } else {
            // Back past this run, then to the top of the one before it.
            let start = lines[..self.selected].iter().rposition(|l| &l.id != id);
            start.map(|s| lines[..s].iter().rposition(|l| l.id != lines[s].id).map_or(0, |i| i + 1))
        };
        if let Some(at) = at {
            self.selected = at;
        }
    }

    fn on_key(&mut self, k: KeyEvent) -> Flow<(), Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.list_area.height.max(2) as isize - 1;
        match k.code {
            KeyCode::Char('c') if ctrl => return Flow::Done(()),
            KeyCode::Char('d') if ctrl => self.step(page / 2),
            KeyCode::Char('u') if ctrl => self.step(-page / 2),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::PageDown => self.step(page),
            KeyCode::PageUp => self.step(-page),
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.step(isize::MAX / 2),
            KeyCode::Char('n') => self.next_change(true),
            KeyCode::Char('N') => self.next_change(false),
            KeyCode::Char('i') => self.side = !self.side,
            KeyCode::Char('y') | KeyCode::Enter => {
                let Some(id) = self.id().map(String::from) else { return Flow::Stay };
                if blame::uncommitted(&id) {
                    self.toast = Some(Toast::new("Not committed yet", theme::YELLOW));
                } else if k.code == KeyCode::Enter {
                    return Flow::Run(Effect::Show(id));
                } else {
                    return Flow::Run(Effect::Copy(id[..7].to_string()));
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => return Flow::Done(()),
            _ => {}
        }
        Flow::Stay
    }
}

// ---------------------------------------------------------------- drawing

fn draw(f: &mut Frame, app: &mut App) {
    let full = f.area();
    f.render_widget(Block::new().style(Style::new().bg(theme::BG)), full);
    let area = Rect { x: full.x + 1, width: full.width.saturating_sub(2), ..full };
    let [header, _, body, _, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let accent = Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD);
    let title = Line::from(vec![
        Span::styled("blame", accent),
        Span::raw("  "),
        Span::styled(crate::git::repo_name(&app.repo), theme::bold()),
        Span::raw("  "),
        Span::styled(app.path.clone(), theme::muted()),
    ]);
    f.render_widget(Paragraph::new(title), header);

    let panes = kit::panes(body, None, 0, app.side);
    app.side_open = panes.side.is_some();
    draw_lines(f, app, panes.main);
    if let Some(side) = panes.side {
        draw_side(f, app, side);
    }

    let left = match &app.blame {
        Some(Ok(b)) => {
            let commits = b.commits.keys().filter(|id| !blame::uncommitted(id)).count();
            Line::styled(
                format!("{} lines · {commits} commit{}", b.lines.len(), if commits == 1 { "" } else { "s" }),
                theme::muted(),
            )
        }
        _ => Line::raw(""),
    };
    let mut pairs = vec![("enter", "show commit"), ("n N", "next commit"), ("y", "copy id"), ("q", "quit")];
    footer::sidebar(&mut pairs, footer.width);
    footer::draw(f, footer, left, pairs);
}

/// Each line: on the first of a run from one commit, its id, author and age; then the
/// line number and the text. A bar marks the selected line's run.
fn draw_lines(f: &mut Frame, app: &mut App, area: Rect) {
    let block = kit::panel(theme::PANEL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;
    let lines = match &app.blame {
        None => return f.render_widget(Paragraph::new(Span::styled("blaming…", theme::faint())), inner),
        Some(Err(e)) => return f.render_widget(Paragraph::new(Span::styled(e.clone(), theme::muted())), inner),
        Some(Ok(b)) => b,
    };
    let height = inner.height as usize;
    app.scroll = list::follow(app.scroll, app.selected, app.selected, height);
    let now = now();
    let selected_id = lines.lines.get(app.selected).map(|l| l.id.as_str());
    let number_width = lines.lines.len().to_string().len();
    let rows: Vec<Line> = (app.scroll..lines.lines.len().min(app.scroll + height))
        .map(|i| {
            let l = &lines.lines[i];
            let first = i == 0 || lines.lines[i - 1].id != l.id;
            let gutter = match (first, blame::uncommitted(&l.id)) {
                (false, _) => " ".repeat(GUTTER),
                (true, true) => format!("{:<GUTTER$}", "not committed yet"),
                (true, false) => {
                    let who = lines.commits.get(&l.id).cloned().unwrap_or_default();
                    let author = who.author.split_whitespace().next().unwrap_or("").to_string();
                    let author: String = author.chars().take(10).collect();
                    format!("{}  {author:<10}  {:>8}", &l.id[..7], relative_time(who.time, now))
                }
            };
            let on = Some(l.id.as_str()) == selected_id;
            let bar = if on { Span::styled("┃ ", Style::new().fg(theme::ACCENT)) } else { Span::raw("  ") };
            let row = Line::from(vec![
                bar,
                Span::styled(gutter, if on { theme::text() } else { theme::faint() }),
                Span::styled(format!("  {:>number_width$}  ", i + 1), theme::faint()),
                Span::styled(l.text.clone(), theme::text()),
            ]);
            let row = if i == app.selected { row.style(Style::new().bg(theme::ELEMENT)) } else { row };
            kit::truncate(row, inner.width)
        })
        .collect();
    f.render_widget(Paragraph::new(rows), inner);
}

/// Columns for `abc1234  author      3 days ago`.
const GUTTER: usize = 7 + 2 + 10 + 2 + 8;

/// The sidebar: the selected line's commit, its message and files.
fn draw_side(f: &mut Frame, app: &App, area: Rect) {
    let w = (area.width as usize).saturating_sub(4);
    let section = |lines| vec![Section { title: "", count: None, lines }];
    let Some(id) = app.id() else { return drop(sidebar::draw(f, area, section(Vec::new()))) };
    if blame::uncommitted(id) {
        let lines =
            vec![Line::styled("Not committed yet: the line is only in the working tree or the index.", theme::muted())];
        return drop(sidebar::draw(f, area, section(lines)));
    }
    let Some(d) = app.details.get(id) else { return drop(sidebar::draw(f, area, section(Vec::new()))) };
    let Some(d) = d else {
        let lines = vec![Line::styled("Couldn't read this commit.", theme::muted())];
        return drop(sidebar::draw(f, area, section(lines)));
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(d.id.clone(), Style::new().fg(theme::ACCENT)),
            Span::styled(format!("  {}", d.author), theme::muted()),
        ]),
        Line::styled(format!("{} · {}", d.email, relative_time(d.time, now())), theme::faint()),
        Line::raw(""),
    ];
    lines.extend(kit::wrap(&d.subject, w).into_iter().map(|l| Line::styled(l, theme::bold())));
    if !d.body.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(kit::reflow(&d.body, w).into_iter().map(|l| Line::styled(l, theme::muted())));
    }
    let files = d.files.iter().map(|f| changes::Item::stat(f).row()).collect();
    let sections = vec![
        Section { title: "", count: None, lines },
        Section { title: "files", count: Some(d.files.len()), lines: files },
    ];
    sidebar::draw(f, area, sections);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::run::Scratch;
    use crate::tui::app::press;

    /// a.txt: two lines from the first commit, one from the second, one not committed.
    fn screen(name: &str) -> (Scratch, App) {
        let s = Scratch::new(name);
        s.commit("a.txt", "one\ntwo\nthree\n");
        s.commit("a.txt", "one\ntwo\n3\n");
        s.write("a.txt", "one\ntwo\n3\nfour\n");
        let mut app = start(s.repo(), Some(s.0.clone()), "a.txt");
        while app.job.is_some() {
            std::thread::sleep(std::time::Duration::from_millis(5));
            app.tick().unwrap();
        }
        (s, app)
    }

    fn text(app: &mut App, w: u16) -> String {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, 20)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        app.tick().unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        let cells: Vec<&str> = t.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        cells.chunks(w as usize).map(|r| r.concat()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn n_goes_from_commit_to_commit_and_enter_shows_it() {
        let (_s, mut app) = screen("blame-keys");
        assert_eq!(app.len(), 4);
        app.key(press(KeyCode::Char('n')));
        assert_eq!(app.selected, 2);
        app.key(press(KeyCode::Char('n')));
        assert_eq!(app.selected, 3);
        // Not committed: nothing to show.
        assert!(matches!(app.key(press(KeyCode::Enter)), Flow::Stay));
        assert!(app.toast.is_some());
        app.key(press(KeyCode::Char('N')));
        assert_eq!(app.selected, 2);
        app.key(press(KeyCode::Char('N')));
        assert_eq!(app.selected, 0);
        let id = app.id().unwrap().to_string();
        assert!(matches!(app.key(press(KeyCode::Enter)), Flow::Run(Effect::Show(s)) if s == id));
        assert!(matches!(app.key(press(KeyCode::Char('y'))), Flow::Run(Effect::Copy(s)) if id.starts_with(&s)));
        assert!(App::in_terminal(&Effect::Show(id)));
    }

    #[test]
    fn draws_runs_and_the_commit_on_the_side() {
        let (_s, mut app) = screen("blame-draw");
        let narrow = text(&mut app, 100);
        // The first commit's id once, on the first of its two lines.
        let id = &app.id().unwrap()[..7];
        assert_eq!(narrow.matches(id).count(), 1, "{narrow}");
        assert!(narrow.contains("not committed yet") && narrow.contains("four"), "{narrow}");
        assert!(!app.side_open);
        let wide = text(&mut app, 200);
        assert!(app.side_open && wide.contains("files   1"), "{wide}");
    }
}

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState},
};

use crate::core::log::Commit;
use crate::render::relative_time;

/// Scrollable commit list. j/k or arrows to move, q or Esc to quit.
pub fn run(commits: &[Commit]) -> Result<()> {
    let now = crate::core::now();
    let items: Vec<ListItem> = commits
        .iter()
        .map(|c| {
            ListItem::new(Line::from(vec![
                Span::styled(c.id.clone(), Style::new().fg(Color::Yellow)),
                Span::raw("  "),
                Span::styled(format!("{:>8}", relative_time(c.time, now)), Style::new().add_modifier(Modifier::DIM)),
                Span::raw("  "),
                Span::raw(c.summary.clone()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::bordered().title(" log · j/k move · q quit "))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(Some(0));

    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|f| f.render_stateful_widget(&list, f.area(), &mut state))?;
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('j') | KeyCode::Down => state.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => state.select_previous(),
                    KeyCode::Char('g') | KeyCode::Home => state.select_first(),
                    KeyCode::Char('G') | KeyCode::End => state.select_last(),
                    _ => {}
                }
            }
        }
    })();
    ratatui::restore();
    result
}

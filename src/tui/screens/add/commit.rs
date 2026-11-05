//! The commit box: a summary, a description, and committing (or amending) what's staged.

use ratatui::widgets::Clear;

use super::*;
use crate::git::commit;
use crate::tui::kit::TextArea;

pub(super) struct CommitBox {
    summary: TextArea,
    body: TextArea,
    on_body: bool,
    amend: bool,
}

/// A commit that runs with the terminal: signing may prompt, or the editor opens.
pub(super) struct Pending {
    message: String,
    amend: bool,
    edit: bool,
}

/// Past these, a summary gets long (git's own tools cut at 50 and 72).
const SUMMARY_SOFT: usize = 50;
const SUMMARY_HARD: usize = 72;

impl CommitBox {
    fn new(message: &str, amend: bool) -> Self {
        let (summary, body) = message.split_once('\n').unwrap_or((message, ""));
        CommitBox {
            summary: TextArea::new(summary.trim(), false),
            body: TextArea::new(body.trim_matches('\n'), true),
            // A prepared description is likelier to need the edit.
            on_body: false,
            amend,
        }
    }

    /// Summary, a blank line, description; trailing space trimmed.
    fn message(&self) -> String {
        let summary = self.summary.text().trim().to_string();
        let body = self.body.text().trim_end().to_string();
        if body.trim().is_empty() { summary } else { format!("{summary}\n\n{body}") }
    }
}

impl App {
    /// `c` (or `C` to amend): opens the box with the draft, git's merge message, or
    /// HEAD's message.
    pub(super) fn open_commit(&mut self, amend: bool) {
        let staged = self.entries.iter().any(|e| e.group == Group::Staged);
        if !amend && !staged {
            self.toast = Some(Toast::new("Nothing staged yet: space stages a file", theme::YELLOW));
            return;
        }
        if self.entries.iter().any(|e| e.group == Group::Conflicts) {
            self.toast = Some(Toast::new("Resolve the conflicts first", theme::YELLOW));
            return;
        }
        let message = if amend {
            match commit::last_message(&self.repo) {
                Some(m) => m,
                None => {
                    self.toast = Some(Toast::new("Nothing to amend: no commits yet", theme::YELLOW));
                    return;
                }
            }
        } else {
            self.draft.clone().or_else(|| commit::prepared_message(&self.repo)).unwrap_or_default()
        };
        self.commit = Some(CommitBox::new(&message, amend));
    }

    pub(super) fn commit_key(&mut self, k: KeyEvent) -> Result<()> {
        let Some(b) = &mut self.commit else { return Ok(()) };
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Esc => {
                // Kept for next time (an amend starts from HEAD's message anyway).
                if !b.amend {
                    self.draft = Some(b.message()).filter(|m| !m.is_empty());
                }
                self.commit = None;
            }
            KeyCode::Tab | KeyCode::BackTab => b.on_body = !b.on_body,
            KeyCode::Down if !b.on_body => b.on_body = true,
            KeyCode::Enter if !b.on_body && !ctrl && !alt => self.submit(false)?,
            KeyCode::Enter if alt || ctrl => self.submit(false)?,
            KeyCode::Char('s') if ctrl => self.submit(false)?,
            KeyCode::Char('o') if ctrl => self.submit(true)?,
            _ => {
                let field = if b.on_body { &mut b.body } else { &mut b.summary };
                field.key(k);
            }
        }
        Ok(())
    }

    /// Commits now, or (with `edit`, or when commits are signed) once the screen has
    /// stepped aside.
    fn submit(&mut self, edit: bool) -> Result<()> {
        let Some(b) = &self.commit else { return Ok(()) };
        if !edit && b.summary.is_empty() {
            self.toast = Some(Toast::new("Write a summary first (or ctrl+o for your editor)", theme::YELLOW));
            return Ok(());
        }
        let (message, amend) = (b.message(), b.amend);
        if edit || commit::signs(&self.repo) {
            self.draft = (!amend).then(|| message.clone());
            self.commit = None;
            self.pending = Some(Pending { message, amend, edit });
            return Ok(());
        }
        let (ok, out) = commit::create(&self.repo, &message, amend);
        if ok {
            self.commit = None;
            self.committed(amend)
        } else {
            // The box stays open with the message, so a hook's complaint can be fixed.
            self.toast = Some(Toast::new(first_lines(&out), theme::RED));
            Ok(())
        }
    }

    /// Runs a pending commit on the terminal (from `Screen::outside`).
    pub(super) fn commit_outside(&mut self) -> Result<()> {
        let Some(p) = self.pending.take() else { return Ok(()) };
        let (ok, out) = commit::create_on_terminal(&self.repo, &p.message, p.amend, p.edit);
        if ok {
            return self.committed(p.amend);
        }
        let why = if out.is_empty() { "The commit was cancelled".to_string() } else { first_lines(&out) };
        self.toast = Some(Toast::new(why, theme::RED));
        self.reload()
    }

    /// After a commit: the new HEAD in a toast, the draft gone, the list fresh.
    fn committed(&mut self, amend: bool) -> Result<()> {
        self.draft = None;
        self.focus = Focus::Files;
        self.reload()?;
        self.selected = self.entries.iter().position(|e| e.group != Group::Staged).unwrap_or(0);
        let head = self.repo.head_commit().ok();
        let id = head.as_ref().map(|c| c.id().shorten_or_id().to_string()).unwrap_or_default();
        let subject = head.as_ref().and_then(|c| c.message().ok().map(|m| m.summary().to_string())).unwrap_or_default();
        let done = if amend { "Amended" } else { "Committed" };
        self.toast = Some(Toast::new(format!("{done} {id}  {subject}"), theme::GREEN));
        Ok(())
    }

    pub(super) fn draw_commit(&mut self, f: &mut Frame, full: Rect) {
        let staged: Vec<changes::Item> =
            self.entries.iter().filter(|e| e.group == Group::Staged).map(|e| e.item()).collect();
        let n = staged.len();
        let s = if n == 1 { "" } else { "s" };
        let label = if self.commit.as_ref().is_some_and(|b| b.amend) {
            format!("Adds {n} staged file{s}")
        } else {
            format!("{n} file{s}")
        };
        let files = changes::header(label, &staged);
        let branch = self.st.branch.clone();
        // HEAD is on its upstream already: amending it means a force push.
        let pushed =
            self.st.upstream.as_ref().filter(|u| u.ahead == 0 && !u.name.ends_with("(gone)")).map(|u| u.name.clone());
        let head = self.repo.head_id().map(|h| h.shorten_or_id().to_string()).unwrap_or_default();
        let Some(b) = &mut self.commit else { return };

        let body_rows = full.height.saturating_sub(16).clamp(2, 8);
        let warn = b.amend && pushed.is_some();
        let w = full.width.saturating_sub(4).min(78);
        let h = 11 + body_rows + u16::from(warn);
        let r = kit::centered(full, w, h);
        f.render_widget(Clear, r);
        let block = kit::panel(theme::ELEMENT);
        let inner = block.inner(r);
        f.render_widget(block, r);

        let mut rows = vec![Constraint::Length(1), Constraint::Length(1)];
        if warn {
            rows.push(Constraint::Length(1));
        }
        rows.extend([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(body_rows),
            Constraint::Length(1),
            Constraint::Length(1),
        ]);
        let areas = Layout::vertical(rows).split(inner);
        let mut at = areas.iter().copied();
        let mut next = || at.next().unwrap_or_default();

        let title = if b.amend { format!("Amend {head}") } else { "Commit".to_string() };
        let mut spans = vec![Span::styled(title, theme::bold())];
        if let Some(br) = &branch {
            spans.push(Span::styled(format!(" on {br}"), theme::muted()));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), next());
        let info = if b.amend && staged.is_empty() {
            Line::styled("Only the message changes.", theme::muted())
        } else {
            files
        };
        f.render_widget(Paragraph::new(info), next());
        if warn {
            let up = pushed.unwrap_or_default();
            let text = format!("Already on {up}: after amending, pushing needs --force-with-lease.");
            f.render_widget(Paragraph::new(Span::styled(text, Style::new().fg(theme::YELLOW))), next());
        }
        next();

        // "summary" and its length, which turns yellow, then red, as it grows.
        let n = b.summary.first_len();
        let color = if n > SUMMARY_HARD {
            theme::RED
        } else if n > SUMMARY_SOFT {
            theme::YELLOW
        } else {
            theme::FAINT
        };
        let label = |on: bool, t: &str| Span::styled(t.to_string(), if on { theme::bold() } else { theme::faint() });
        let row = next();
        f.render_widget(Paragraph::new(label(!b.on_body, "summary")), row);
        f.render_widget(
            Paragraph::new(Span::styled(n.to_string(), Style::new().fg(color)))
                .alignment(ratatui::layout::Alignment::Right),
            row,
        );
        b.summary.draw(f, next(), !b.on_body, "what this commit does");
        next();
        f.render_widget(Paragraph::new(label(b.on_body, "description")), next());
        b.body.draw(f, next(), b.on_body, "why, if it's not obvious (optional)");
        next();

        let go = if b.amend { "Amend" } else { "Commit" };
        let first = if b.on_body { format!("ctrl+s  {go}") } else { format!("enter  {go}") };
        let switch = if b.on_body { "tab  summary" } else { "tab  description" };
        let pills = Line::from(vec![
            kit::pill(&first, theme::ACCENT, theme::BG),
            Span::raw("  "),
            kit::pill(switch, theme::BORDER, theme::TEXT),
            Span::raw("  "),
            kit::pill("ctrl+o  editor", theme::BORDER, theme::TEXT),
            Span::raw("  "),
            kit::pill("esc  later", theme::BORDER, theme::TEXT),
        ]);
        f.render_widget(Paragraph::new(kit::truncate(pills, inner.width)), next());
    }
}

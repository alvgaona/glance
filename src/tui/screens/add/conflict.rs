//! Conflicted files: what kind of conflict, the file with its markers in color, and
//! resolving it (keep the file, take the deletion, or mark the fixed file resolved).

use super::*;
use crate::git::stage::Conflict;

/// A few words for the list and the diff's title.
pub(super) fn what(c: Option<Conflict>) -> &'static str {
    match c {
        Some(c) if c.in_content() && !c.base => "added on both sides",
        Some(c) if c.in_content() => "changed on both sides",
        Some(c) if c.ours => "deleted by the incoming side",
        Some(c) if c.theirs => "deleted in HEAD",
        Some(_) => "deleted on both sides",
        None => "conflicted",
    }
}

/// The diff pane's body for a conflicted file: what to do, then the file as it is.
pub(super) fn body(repo: &gix::Repository, e: &Entry) -> Vec<Line<'static>> {
    let c = e.conflict;
    let advice = match c {
        Some(c) if !c.ours && !c.theirs => "Both sides deleted it: space removes it.",
        Some(c) if !c.in_content() && c.ours => {
            "HEAD changed it, the incoming side deleted it.\nspace keeps it as it is below, d deletes it."
        }
        Some(c) if !c.in_content() => {
            "HEAD deleted it, the incoming side changed it.\nspace keeps their version below, d deletes it."
        }
        _ => "",
    };
    let mut out: Vec<Line> = advice.lines().map(|l| Line::styled(l.to_string(), theme::muted())).collect();
    let Some(dir) = repo.workdir() else { return out };
    let Ok(bytes) = std::fs::read(dir.join(&e.path)) else { return out };
    if bytes.contains(&0) {
        out.push(Line::styled("Binary file: keep one side with git checkout --ours or --theirs.", theme::muted()));
        return out;
    }
    let text = String::from_utf8_lossy(&bytes);
    let n = text.lines().filter(|l| l.starts_with("<<<<<<<")).count();
    if n > 0 {
        let s = if n == 1 { "" } else { "s" };
        out.push(Line::styled(
            format!("{n} conflict{s}: fix the file (e opens it), then space marks it resolved."),
            theme::muted(),
        ));
    } else if c.is_some_and(|c| c.in_content()) {
        out.push(Line::styled("No markers left: space marks it resolved.", Style::new().fg(theme::GREEN)));
    }
    if !out.is_empty() {
        out.push(Line::raw(""));
    }
    out.extend(colored(&text));
    out
}

/// The file with line numbers; inside a conflict, HEAD's lines green, the base's faint,
/// the incoming lines blue, and the markers bold yellow.
fn colored(text: &str) -> Vec<Line<'static>> {
    #[derive(Clone, Copy)]
    enum Part {
        Outside,
        Ours,
        Base,
        Theirs,
    }
    let marker = Style::new().fg(theme::YELLOW).add_modifier(Modifier::BOLD);
    let mut part = Part::Outside;
    let width = text.lines().count().to_string().len().max(3);
    text.lines()
        .enumerate()
        .map(|(i, l)| {
            let (next, style) = match l {
                _ if l.starts_with("<<<<<<<") => (Part::Ours, marker),
                _ if l.starts_with("|||||||") => (Part::Base, marker),
                _ if l.starts_with("=======") && !matches!(part, Part::Outside) => (Part::Theirs, marker),
                _ if l.starts_with(">>>>>>>") => (Part::Outside, marker),
                _ => (
                    part,
                    match part {
                        Part::Outside => theme::text(),
                        Part::Ours => Style::new().fg(theme::GREEN),
                        Part::Base => theme::faint(),
                        Part::Theirs => Style::new().fg(theme::BLUE),
                    },
                ),
            };
            part = next;
            Line::from(vec![
                Span::styled(format!("{:>width$}  ", i + 1), theme::faint()),
                Span::styled(l.replace('\t', "    ").trim_end_matches('\r').to_string(), style),
            ])
        })
        .collect()
}

impl App {
    /// Space on a conflict: keep the file (or remove it if both sides deleted it); a
    /// file with markers still in it asks first.
    pub(super) fn resolve(&mut self) -> Result<()> {
        let Some(e) = self.current() else { return Ok(()) };
        let (path, c) = (e.path.clone(), e.conflict);
        let paths = [path.clone()];
        match c {
            Some(c) if !c.ours && !c.theirs => {
                let r = stage::remove(&self.repo, &paths);
                self.done(r, format!("Removed {path}"))
            }
            Some(c) if !c.in_content() => {
                let r = stage::add(&self.repo, &paths);
                self.done(r, format!("Kept {path}"))
            }
            _ if has_markers(&self.repo, &path) => {
                self.confirm = Some((
                    Confirm {
                        title: format!("Mark {path} resolved?"),
                        body: "It still has conflict markers (<<<<<<<).\nStaged as it is, they'd be committed too."
                            .into(),
                        yes: "Stage anyway".into(),
                        no: "Cancel".into(),
                        danger: true,
                    },
                    Ask::Resolve(path),
                ));
                Ok(())
            }
            _ => {
                let r = stage::add(&self.repo, &paths);
                self.done(r, format!("Marked {path} resolved"))
            }
        }
    }

    /// d on a conflict: take the deletion, after asking. Content conflicts are fixed in
    /// the file instead.
    pub(super) fn ask_remove(&mut self) {
        let Some(e) = self.current() else { return };
        if e.conflict.is_none_or(|c| c.in_content()) {
            self.toast = Some(Toast::new("Fix the conflict in the file: e opens it", theme::YELLOW));
            return;
        }
        let path = e.path.clone();
        let confirm = Confirm {
            title: format!("Delete {path}?"),
            body: "Takes the side that deleted it.\nThe other side's changes to it are dropped.".into(),
            yes: "Delete".into(),
            no: "Cancel".into(),
            danger: true,
        };
        self.confirm = Some((confirm, Ask::Remove(path)));
    }

    /// `e`: the file in the editor, once the screen has stepped aside.
    pub(super) fn ask_edit(&mut self) {
        let Some(e) = self.current() else { return };
        let on_disk = self.repo.workdir().is_some_and(|d| d.join(&e.path).is_file());
        if !on_disk {
            let why = if e.is_folder() { "That's a folder" } else { "It's not in the working tree" };
            self.toast = Some(Toast::new(why, theme::YELLOW));
            return;
        }
        self.editing = Some(e.path.clone());
    }

    /// Runs the editor (from `Screen::outside`), then shows the file as it is now.
    pub(super) fn edit_outside(&mut self) -> Result<()> {
        let Some(path) = self.editing.take() else { return Ok(()) };
        let (ok, out) = crate::git::run::edit(&self.repo, &path);
        let place = self.place();
        match place {
            Some((g, pos)) => self.reload_at(g, pos)?,
            None => self.reload()?,
        }
        self.focus = Focus::Files;
        let conflict = self.current().filter(|e| e.path == path).and_then(|e| e.conflict);
        self.toast = Some(if !ok {
            Toast::new(if out.is_empty() { "The editor failed".into() } else { first_lines(&out) }, theme::RED)
        } else if conflict.is_some_and(|c| c.in_content()) && !has_markers(&self.repo, &path) {
            Toast::new("No markers left: space marks it resolved", theme::GREEN)
        } else {
            Toast::new(format!("Edited {path}"), theme::ACCENT)
        });
        Ok(())
    }
}

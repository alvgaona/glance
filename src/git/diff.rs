use gix::diff::blob::{Algorithm, Diff, InternedInput, sources::byte_lines};

/// How much a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Lines { added: u32, removed: u32 },
    Binary,
}

/// Skip line counting for very large files; it's not worth the wait.
const MAX_BYTES: usize = 8 << 20;

pub fn count(old: &[u8], new: &[u8]) -> Option<Size> {
    if old.len() > MAX_BYTES || new.len() > MAX_BYTES {
        return None;
    }
    if old.contains(&0) || new.contains(&0) {
        return Some(Size::Binary);
    }
    let input = InternedInput::new(byte_lines(old), byte_lines(new));
    let diff = Diff::compute(Algorithm::Histogram, &input);
    Some(Size::Lines { added: diff.count_additions(), removed: diff.count_removals() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(lines: &[PatchLine]) -> String {
        lines
            .iter()
            .map(|l| match l {
                PatchLine::Hunk { old_start, new_start } => format!("@@ {old_start} {new_start}"),
                PatchLine::Context { text, .. } => format!(" {text}"),
                PatchLine::Added { text, .. } => format!("+{text}"),
                PatchLine::Removed { text, .. } => format!("-{text}"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn unified_with_context() {
        let old: String = (1..=20).map(|i| format!("{i}\n")).collect();
        let new = old.replace("10\n", "ten\n").replace("19\n", "");
        let p = patch(old.as_bytes(), new.as_bytes(), 2).unwrap();
        // Two separate hunks (lines 10 and 19 are far apart), each with 2 lines of context.
        assert_eq!(render(&p), "@@ 8 8\n 8\n 9\n-10\n+ten\n 11\n 12\n@@ 17 17\n 17\n 18\n-19\n 20");
    }

    #[test]
    fn close_changes_share_a_hunk() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "A\nb\nc\nD\ne\n";
        let p = patch(old.as_bytes(), new.as_bytes(), 3).unwrap();
        assert_eq!(render(&p), "@@ 1 1\n-a\n+A\n b\n c\n-d\n+D\n e");
    }
}

/// One line of a unified diff. Old-side line numbers aren't drawn yet (the view shows new ones).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PatchLine {
    /// `@@ -12,7 +12,9 @@`
    Hunk {
        old_start: u32,
        new_start: u32,
    },
    Context {
        old: u32,
        new: u32,
        text: String,
    },
    Added {
        new: u32,
        text: String,
    },
    Removed {
        old: u32,
        text: String,
    },
}

/// Unified diff with `context` lines around each change, for drawing: text with tabs
/// expanded and line endings taken off. `None` for binary or huge files.
pub fn patch(old: &[u8], new: &[u8], context: u32) -> Option<Vec<PatchLine>> {
    Some(hunks(old, new, context)?.iter().flat_map(Hunk::patch_lines).collect())
}

/// One hunk of a unified diff, byte for byte, so it can be applied again.
#[derive(Debug, Clone)]
pub struct Hunk {
    /// First line on each side, counting from 1.
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Context,
    Removed,
    Added,
}

#[derive(Debug, Clone)]
pub struct HunkLine {
    pub op: Op,
    /// Line numbers on each side (the side a line isn't on keeps the last one seen).
    pub old: u32,
    pub new: u32,
    /// The line as it is in the file, ending included (none on a last line without one).
    pub bytes: Vec<u8>,
}

impl Hunk {
    /// How many lines it covers on the old and on the new side.
    pub fn lens(&self) -> (u32, u32) {
        let old = self.lines.iter().filter(|l| l.op != Op::Added).count() as u32;
        let new = self.lines.iter().filter(|l| l.op != Op::Removed).count() as u32;
        (old, new)
    }

    /// For drawing: its `@@` line, then its lines as text (tabs expanded, no endings).
    pub fn patch_lines(&self) -> Vec<PatchLine> {
        let text = |b: &[u8]| String::from_utf8_lossy(b).trim_end_matches(['\n', '\r']).replace('\t', "    ");
        let mut out = vec![PatchLine::Hunk { old_start: self.old_start, new_start: self.new_start }];
        out.extend(self.lines.iter().map(|l| match l.op {
            Op::Context => PatchLine::Context { old: l.old, new: l.new, text: text(&l.bytes) },
            Op::Removed => PatchLine::Removed { old: l.old, text: text(&l.bytes) },
            Op::Added => PatchLine::Added { new: l.new, text: text(&l.bytes) },
        }));
        out
    }
}

/// The hunks between `old` and `new`, with `context` lines around each change; changes
/// whose context would touch share a hunk. `None` for binary or huge files.
pub fn hunks(old: &[u8], new: &[u8], context: u32) -> Option<Vec<Hunk>> {
    if old.len() > MAX_BYTES || new.len() > MAX_BYTES || old.contains(&0) || new.contains(&0) {
        return None;
    }
    let input = InternedInput::new(byte_lines(old), byte_lines(new));
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    let bytes = |t: gix::diff::blob::Token| input.interner[t].to_vec();
    let (n_old, n_new) = (input.before.len() as u32, input.after.len() as u32);

    let mut groups: Vec<Vec<gix::diff::blob::Hunk>> = Vec::new();
    for h in diff.hunks() {
        match groups.last_mut() {
            Some(g) if h.before.start <= g.last().unwrap().before.end + 2 * context => g.push(h),
            _ => groups.push(vec![h]),
        }
    }

    let mut out = Vec::new();
    for g in groups {
        let first = &g[0];
        let lead = first.before.start.min(context);
        let (mut o, mut n) = (first.before.start - lead, first.after.start - lead);
        let mut hunk = Hunk { old_start: o + 1, new_start: n + 1, lines: Vec::new() };
        let mut push = |op, old, new, b| hunk.lines.push(HunkLine { op, old, new, bytes: b });
        for h in &g {
            while o < h.before.start {
                push(Op::Context, o + 1, n + 1, bytes(input.before[o as usize]));
                o += 1;
                n += 1;
            }
            for i in h.before.clone() {
                push(Op::Removed, i + 1, n, bytes(input.before[i as usize]));
            }
            for i in h.after.clone() {
                push(Op::Added, h.before.end, i + 1, bytes(input.after[i as usize]));
            }
            o = h.before.end;
            n = h.after.end;
        }
        let end = (o + context).min(n_old);
        while o < end && n < n_new {
            push(Op::Context, o + 1, n + 1, bytes(input.before[o as usize]));
            o += 1;
            n += 1;
        }
        out.push(hunk);
    }
    Some(out)
}

/// Contents of a blob, or empty if it can't be read.
pub fn blob(repo: &gix::Repository, id: impl Into<gix::ObjectId>) -> Vec<u8> {
    let id = id.into();
    if id.is_null() {
        return Vec::new();
    }
    repo.find_object(id).map(|o| o.detach().data).unwrap_or_default()
}

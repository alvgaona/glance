use gix::diff::blob::{Algorithm, Diff, InternedInput, sources::byte_lines};

/// How much a file changed.
#[derive(Debug, Clone, Copy)]
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
    Some(Size::Lines {
        added: diff.count_additions(),
        removed: diff.count_removals(),
    })
}

/// Contents of a blob, or empty if it can't be read.
pub fn blob(repo: &gix::Repository, id: impl Into<gix::ObjectId>) -> Vec<u8> {
    let id = id.into();
    if id.is_null() {
        return Vec::new();
    }
    repo.find_object(id).map(|o| o.detach().data).unwrap_or_default()
}

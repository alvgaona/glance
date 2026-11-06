//! delta draws the diffs in `glance diff`, and in the log and stash details, when it's
//! installed.

use crate::process::{run, run_with_input, which};

pub fn available() -> bool {
    which("delta")
}

/// Put in front of each hunk header (delta's `hunk-label`) to find them in the output,
/// then drawn as the `•` delta's own `navigate` puts there. Private use: never in a diff.
const MARK: char = '\u{E000}';
/// The same for file headers, put before delta's usual labels and then taken out.
const FILE_MARK: char = '\u{E001}';

/// What delta drew: text with colour codes, and the lines its hunk and file headers
/// are on.
pub struct Drawn {
    pub text: String,
    pub hunks: Vec<usize>,
    pub files: Vec<usize>,
}

/// `patch` drawn by delta at `width` columns. The user's delta settings (theme, styles)
/// still apply; `set` overrides some, as `("side-by-side", "false")`.
pub fn render(patch: &str, width: u16, set: &[(&str, &str)]) -> Result<Drawn, String> {
    let mark = MARK.to_string();
    let label = |text: &str| format!("{FILE_MARK}{text}");
    let (added, removed, renamed, copied) = (label("added:"), label("removed:"), label("renamed:"), label("copied:"));
    let modified = FILE_MARK.to_string();
    let fixed = [
        ("hunk-label", mark.as_str()),
        ("file-modified-label", modified.as_str()),
        ("file-added-label", added.as_str()),
        ("file-removed-label", removed.as_str()),
        ("file-renamed-label", renamed.as_str()),
        ("file-copied-label", copied.as_str()),
        ("hyperlinks", "false"),
    ];
    // Kept: a `git -c` from the user still counts.
    let params = std::iter::once(std::env::var("GIT_CONFIG_PARAMETERS").unwrap_or_default())
        .chain(fixed.iter().chain(set).map(|(k, v)| format!("'delta.{k}'='{v}'")))
        .collect::<Vec<_>>()
        .join(" ");
    let args: Vec<String> = ["--paging=never", "--line-fill-method=spaces", &format!("--width={width}")]
        .into_iter()
        .map(String::from)
        .collect();
    let env = [("GIT_CONFIG_PARAMETERS", params.trim())];
    let out = match run_with_input("delta", &args, None, patch.as_bytes(), &env) {
        Ok(out) if out.ok() => out.stdout,
        Ok(out) => return Err(out.stderr.trim().to_string()),
        Err(e) => return Err(format!("could not run delta: {e}")),
    };
    let at = |mark: char| out.lines().enumerate().filter(|(_, l)| l.contains(mark)).map(|(i, _)| i).collect();
    let (hunks, files) = (at(MARK), at(FILE_MARK));
    // A modified file's label is only the mark: take its space too, as delta's own empty label has none.
    let text = out.replace(MARK, "•").replace(&format!("{FILE_MARK} "), "").replace(FILE_MARK, "");
    Ok(Drawn { text, hunks, files })
}

/// A delta switch from git config (`delta.side-by-side`), when it's set.
pub fn setting(key: &str) -> Option<bool> {
    let args = ["config", "--type=bool", "--get", &format!("delta.{key}")].map(String::from);
    let out = run("git", &args, None).ok()?;
    match out.stdout.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

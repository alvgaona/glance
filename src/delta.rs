//! delta draws the diffs in `glance diff`, and in the log and stash details, when it's
//! installed.

use crate::process::{run, run_with_input, which};

pub fn available() -> bool {
    which("delta")
}

/// Put in front of each hunk header (delta's `hunk-label`) to find them in the output,
/// then drawn as the `•` delta's own `navigate` puts there. Private use: never in a diff.
const MARK: char = '\u{E000}';

/// What delta drew: text with colour codes, and the lines its hunk headers are on.
pub struct Drawn {
    pub text: String,
    pub hunks: Vec<usize>,
}

/// `patch` drawn by delta at `width` columns. The user's delta settings (theme, styles)
/// still apply; `set` overrides some, as `("side-by-side", "false")`.
pub fn render(patch: &str, width: u16, set: &[(&str, &str)]) -> Result<Drawn, String> {
    let mark = MARK.to_string();
    let fixed = [("hunk-label", mark.as_str()), ("hyperlinks", "false")];
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
    let hunks = out.lines().enumerate().filter(|(_, l)| l.contains(MARK)).map(|(i, _)| i).collect();
    Ok(Drawn { text: out.replace(MARK, "•"), hunks })
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

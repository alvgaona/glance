//! Commit picker in fzf, for builds without the full-screen views.
//! Same look and keys as the `glog` shell function.

use anyhow::{Context, Result, bail};

use crate::process::{run_with_input, which};

/// Finds the first commit hash in the fzf line (works even if a graph prefix is added later).
const HASH: &str = r#"h=$(grep -oE "[a-f0-9]{7,}" <<< {} | head -1); [ -z "$h" ] && exit 0"#;

const SHOW: &str = r#"git show --color=always --stat --patch --date=format:"%a %d %b %Y %H:%M" --format="%C(yellow bold)%h%C(reset)%C(auto)%d%C(reset)%n%C(blue)%aN%C(reset) %C(dim)<%aE>%C(reset)%n%C(green)%ar%C(reset) %C(dim)· %ad%C(reset)%n%n%C(bold)%s%C(reset)%n%n%b" "$h""#;

/// Same palette as the full-screen views (src/tui/theme.rs).
const COLORS: &str = "fg:#eeeeee,fg+:#eeeeee,bg+:#202020,hl:#e5c07b:bold,hl+:#e5c07b:bold,\
border:#303030,label:#fab283:bold,prompt:#fab283,pointer:#fab283,marker:#fab283,\
info:#808080,header:#808080,spinner:#fab283,gutter:-1,preview-border:#303030,preview-label:#808080,\
separator:#303030,scrollbar:#303030,query:#eeeeee:regular,ghost:#505050";

pub fn available() -> bool {
    which("fzf")
}

/// Shows `lines` in fzf. Returns the picked commit id, or `None` if the user quit.
/// `enter` says what Enter does, for the key hints.
pub fn pick_commit(lines: &[String], label: &str, enter: &str) -> Result<Option<String>> {
    let (unified, split) = if which("delta") {
        (
            format!("{HASH}; {SHOW} | delta --no-gitconfig --line-numbers --paging=never --width=$FZF_PREVIEW_COLUMNS"),
            format!("{HASH}; {SHOW} | delta --paging=never --side-by-side --width=$FZF_PREVIEW_COLUMNS"),
        )
    } else {
        let plain = format!("{HASH}; {SHOW}");
        (plain.clone(), plain)
    };

    let args = [
        "--ansi".to_string(),
        "--no-sort".into(),
        "--layout=reverse".into(),
        "--height=100%".into(),
        "--border=rounded".into(),
        format!("--border-label= {label} "),
        "--prompt=  ".into(),
        "--pointer=┃".into(),
        "--info=inline-right".into(),
        "--ghost=search commits".into(),
        "--highlight-line".into(),
        format!("--color={COLORS}"),
        format!("--preview={unified}"),
        "--preview-window=right,60%,border-left,<140(down,60%,border-top)".into(),
        "--preview-label= diff ".into(),
        format!("--header=enter {enter} · ctrl-s side-by-side · ctrl-u unified · ctrl-/ hide preview"),
        format!("--bind=ctrl-s:change-preview-label( side-by-side )+change-preview:{split}"),
        format!("--bind=ctrl-u:change-preview-label( diff )+change-preview:{unified}"),
        "--bind=ctrl-/:toggle-preview".into(),
        "--bind=shift-down:preview-half-page-down,shift-up:preview-half-page-up".into(),
    ];
    // fzf draws on /dev/tty, so its stdin and stdout are free for the list and the pick.
    // Preview commands use bash syntax (<<<), so don't depend on the user's $SHELL.
    // fzf closing its input early (the user quit) is fine: the writer just stops.
    let input = lines.iter().map(|l| format!("{l}\n")).collect::<String>();
    let out =
        run_with_input("fzf", &args, None, input.as_bytes(), &[("SHELL", "bash")]).context("could not start fzf")?;
    match out.code {
        Some(0) => {}
        Some(1) | Some(130) => return Ok(None), // no match / Esc or ctrl-c
        code => bail!("fzf exited with {code:?}: {}", out.stderr.trim()),
    }
    Ok(first_hash(&out.stdout).map(str::to_owned))
}

/// First word of 7+ hex characters, skipping the graph in front of it.
/// Connector lines of the graph have none.
fn first_hash(line: &str) -> Option<&str> {
    line.split(|c: char| !c.is_ascii_hexdigit())
        .find(|w| w.len() >= 7 && w.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

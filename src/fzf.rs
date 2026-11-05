//! `gg log -i`: commit picker in fzf, same look and keys as the `glog` shell function.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// Finds the first commit hash in the fzf line (works even if a graph prefix is added later).
const HASH: &str = r#"h=$(grep -oE "[a-f0-9]{7,}" <<< {} | head -1); [ -z "$h" ] && exit 0"#;

const SHOW: &str = r#"git show --color=always --stat --patch --date=format:"%a %d %b %Y %H:%M" --format="%C(yellow bold)%h%C(reset)%C(auto)%d%C(reset)%n%C(blue)%aN%C(reset) %C(dim)<%aE>%C(reset)%n%C(green)%ar%C(reset) %C(dim)· %ad%C(reset)%n%n%C(bold)%s%C(reset)%n%n%b" "$h""#;

pub fn available() -> bool {
    which("fzf")
}

/// Shows `lines` in fzf. Returns the picked commit id, or `None` if the user quit.
pub fn pick_commit(lines: &[String], label: &str) -> Result<Option<String>> {
    let (unified, split) = if which("delta") {
        (
            format!("{HASH}; {SHOW} | delta --no-gitconfig --line-numbers --paging=never --width=$FZF_PREVIEW_COLUMNS"),
            format!("{HASH}; {SHOW} | delta --paging=never --side-by-side --width=$FZF_PREVIEW_COLUMNS"),
        )
    } else {
        let plain = format!("{HASH}; {SHOW}");
        (plain.clone(), plain)
    };

    let mut child = Command::new("fzf")
        .args([
            "--ansi",
            "--no-sort",
            "--layout=reverse",
            "--height=100%",
            "--border=rounded",
            &format!("--border-label= {label} "),
            "--prompt=  ",
            "--pointer=▌",
            "--info=inline-right",
            "--ghost=search commits",
            "--highlight-line",
            "--color=border:8,label:4:bold,prompt:4,pointer:4,hl:3:bold,hl+:3:bold,bg+:236,gutter:-1,info:8,header:8,preview-border:8,preview-label:8",
            &format!("--preview={unified}"),
            "--preview-window=right,60%,border-left,<140(down,60%,border-top)",
            "--preview-label= diff ",
            "--header=enter copy · ctrl-s side-by-side · ctrl-u unified · ctrl-/ hide preview",
            &format!("--bind=ctrl-s:change-preview-label( side-by-side )+change-preview:{split}"),
            &format!("--bind=ctrl-u:change-preview-label( diff )+change-preview:{unified}"),
            "--bind=ctrl-/:toggle-preview",
            "--bind=shift-down:preview-half-page-down,shift-up:preview-half-page-up",
        ])
        // Preview commands use bash syntax (<<<), so don't depend on the user's $SHELL.
        .env("SHELL", "bash")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("could not start fzf")?;

    {
        let mut stdin = child.stdin.take().expect("stdin is piped");
        for line in lines {
            // fzf closes its input early if the user quits; that's fine.
            if writeln!(stdin, "{line}").is_err() {
                break;
            }
        }
    }

    let out = child.wait_with_output()?;
    match out.status.code() {
        Some(0) => {}
        Some(1) | Some(130) => return Ok(None), // no match / Esc or ctrl-c
        _ => bail!("fzf exited with {}", out.status),
    }
    let picked = String::from_utf8_lossy(&out.stdout);
    Ok(first_hash(&picked).map(str::to_owned))
}

/// First word of 7+ hex characters, skipping the graph in front of it.
/// Connector lines of the graph have none.
fn first_hash(line: &str) -> Option<&str> {
    line.split(|c: char| !c.is_ascii_hexdigit())
        .find(|w| w.len() >= 7 && w.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// Puts `text` on the macOS clipboard. Silently does nothing elsewhere.
pub fn copy(text: &str) {
    if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

pub fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

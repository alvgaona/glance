<div align="center">

# glance

**A nicer view of git status, log and friends.**

[![CI](https://github.com/alvgaona/glance/actions/workflows/ci.yaml/badge.svg)](https://github.com/alvgaona/glance/actions/workflows/ci.yaml)
[![crates.io](https://img.shields.io/crates/v/glance-git.svg)](https://crates.io/crates/glance-git)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

</div>

---

glance is a friendlier face for the git commands you run all day. In a terminal, each
one opens a full screen you can move around in: the file list next to its diff, the
branch list with its pull requests, the stash next to what's in it. Pipe it, and you
get clean printed output instead.

It doesn't replace git, it sits on top. Reads go through
[gix](https://github.com/GitoxideLabs/gitoxide), so screens open fast even in big
repositories. Every change runs the `git` binary, so your hooks, signing, config and
aliases behave exactly as they always do. Nothing glance does is hidden from plain git.

The goal: the everyday things that are awkward in plain git (staging part of a file,
cleaning up merged branches, finding a stash, resolving a conflict, an interactive
rebase) should be obvious, quick, and hard to get wrong.

## Install

```sh
cargo install glance-git          # builds from source, the command is `glance`
cargo binstall glance-git         # downloads a release binary instead
```

Or download a binary for macOS or Linux from the
[releases](https://github.com/alvgaona/glance/releases).

### Dependencies

| Tool | | Used for |
| --- | --- | --- |
| [`git`](https://git-scm.com) | required | every change glance makes (stage, commit, fetch, rebase…) |
| [`gh`](https://cli.github.com) | optional | pull request status and links in `glance branches` |
| [`fzf`](https://github.com/junegunn/fzf) | optional | picking where `glance rebase -p` starts |
| [`delta`](https://github.com/dandavison/delta) | optional | the diffs in `glance diff`, and nicer `fzf` previews |

Without an optional tool, only the feature that uses it is missing. Copying with `y`
works on macOS only for now.

Like a shorter name? `alias gg=glance`.

## Commands

| Command | Alias | What it does |
| --- | --- | --- |
| `glance status` | `st` | Branch, upstream and changed files, grouped. Printed, like `git status` but easier to scan. |
| `glance add` | | Stage on a full screen, with each file next to its diff. Stage files, hunks or single lines, commit and amend, resolve conflicts. `glance add <path>…` stages directly. |
| `glance diff` | `d` | Changed files on the left, the picked one's diff on the right, drawn by delta when it's installed. Switch between unstaged, staged and since HEAD, and change the layout on the screen. Takes any `git diff` arguments; piped, it is plain `git diff`. |
| `glance log` | `l` | Commit history with details and refs. Full screen in a terminal, a compact list when piped. |
| `glance branches` | `br` | Local and remote branches by last commit, with ahead/behind, merged or gone, and pull request status. |
| `glance stash` | `sh` | Browse stashes with their changes. Apply, pop, drop, turn one into a branch, or stash just the files you pick. |
| `glance worktrees` | `wt` | Every worktree at once. Add, remove, prune, lock. `cd "$(glance wt)"` jumps to one. |
| `glance remote` | | Remotes and where they point. Add, rename, remove, set URLs, fetch. |
| `glance repos` | | Every repository in a folder, one line each: branch, last commit, merges or rebases in progress, changes, upstream, stashes. |
| `glance rebase` | `rb` | Interactive rebase with a visual todo editor instead of a text file. |

Every screen works the same way: `j`/`k` or arrows to move, `ctrl+p` (or `:`) for every
command, `/` to search where there's a list, `q` to quit. Anything that can't be undone
asks first.

<details>
<summary><b>Keys on each screen</b></summary>

**`glance add`**

| Key | |
| --- | --- |
| `space` | stage or unstage the file (mark a conflict resolved) |
| `enter` | work hunk by hunk: `space` stages a hunk, `d` discards it |
| `v` | in a hunk: pick lines (`j`/`k` move, `J`/`K` extend), then `space` or `d` |
| `c` / `C` | commit / amend (`ctrl+o` writes the message in your editor) |
| `a` / `u` | stage / unstage everything |
| `d` | discard the file's changes (delete, if untracked) |
| `e` | open the file in your editor |

**`glance diff`**

| Key | |
| --- | --- |
| `1` / `2` / `3` | unstaged / staged / everything since HEAD |
| `s` / `l` | side by side / line numbers, on or off |
| `w` | ignore whitespace |
| `+` / `-` / `f` | more / less context, or the whole file |
| `z` | hide the file list |
| `/` | filter files |
| `e` / `y` | open the file in your editor / copy its path |

**`glance log`**

| Key | |
| --- | --- |
| `/` | search commits |
| `tab` | show or hide details |
| `y` | copy the commit id |
| `enter` | copy the commit id and quit |

**`glance branches`**

| Key | |
| --- | --- |
| `enter` | check out |
| `r` | rename |
| `d` / `D` | delete locally / on the remote |
| `space` | select a branch |
| `m` | select every merged or gone branch |
| `f` | fetch all remotes and prune |
| `a` | show remote branches |
| `o` | open the pull request |

**`glance stash`**

| Key | |
| --- | --- |
| `enter` / `p` | apply / pop |
| `d` | drop |
| `b` | new branch from the stash |
| `n` / `s` / `u` | stash picked files / only staged / only unstaged |

**`glance worktrees`**

| Key | |
| --- | --- |
| `enter` | print the folder (for `cd "$(glance wt)"`) |
| `n` / `d` | new / remove |
| `p` | prune worktrees whose folder is gone |
| `l` | lock or unlock |

**`glance remote`**

| Key | |
| --- | --- |
| `f` / `F` | fetch this remote / all |
| `a` / `r` / `d` | add / rename / remove |
| `u` / `U` | set the fetch / push URL |
| `h` | ask the remote for its default branch |
| `o` | open in the browser |

**`glance rebase`**

| Key | |
| --- | --- |
| `p` `r` `e` `s` `f` `d` | pick, reword, edit, squash, fixup, drop |
| `shift+↑` / `shift+↓` | move a commit |
| `o` | newest or oldest first (`git config glance.rebaseOrder oldest`) |
| `ctrl+s` | run the rebase |
| `esc` | abort |

</details>

## Roadmap

What's planned is tracked in [#1](https://github.com/alvgaona/glance/issues/1). Ideas
welcome: open an issue.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, how the code is laid out, and how
commits and releases work.

## License

[Apache-2.0](LICENSE)

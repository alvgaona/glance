use std::path::{Path, PathBuf};

use anyhow::Result;

use super::status::{self, Status};

pub struct Repo {
    pub name: String,
    /// `Err` holds a short reason the repo couldn't be read.
    pub state: Result<RepoState, String>,
}

pub struct RepoState {
    pub status: Status,
    /// Time of the HEAD commit.
    pub last_commit: Option<i64>,
}

/// Every git repo directly inside `dir`, sorted by name, read a few at a time in parallel.
pub fn read(dir: &Path) -> Result<Vec<Repo>> {
    let mut paths: Vec<PathBuf> =
        std::fs::read_dir(dir)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join(".git").exists()).collect();
    paths.sort();

    // A few at a time: a folder of 500 repos shouldn't read 500 indexes at once.
    let n = std::thread::available_parallelism().map_or(4, |n| n.get());
    let repos = paths
        .chunks(n)
        .flat_map(|chunk| {
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk.iter().map(|path| scope.spawn(move || read_one(path))).collect();
                handles
                    .into_iter()
                    .zip(chunk)
                    .map(|(h, path)| {
                        h.join().unwrap_or_else(|_| Repo {
                            name: file_name(path),
                            state: Err("crashed while reading".into()),
                        })
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    Ok(repos)
}

fn read_one(path: &Path) -> Repo {
    let state = (|| -> Result<RepoState> {
        let repo = gix::open(path)?;
        let status = status::read(&repo, false)?;
        let last_commit = repo.head_commit().ok().and_then(|c| c.time().ok()).map(|t| t.seconds);
        Ok(RepoState { status, last_commit })
    })()
    .map_err(|e| first_line(&e.to_string()));
    Repo { name: file_name(path), state }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

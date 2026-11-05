//! Remote URLs, short enough for a list, and as a page to open.

/// `git@github.com:a/b.git` and `https://github.com/a/b` → `github.com/a/b`.
/// Local paths are shortened like folders: `~/git/x`, `…/git/x`.
pub fn short(url: &str) -> String {
    match host_path(url) {
        Some((host, path)) => format!("{host}/{path}"),
        None => {
            let path = url.strip_prefix("file://").unwrap_or(url);
            if path.starts_with('/') { super::paths::short(std::path::Path::new(path)) } else { path.to_string() }
        }
    }
}

/// The web page of a remote: `git@github.com:a/b.git` → `https://github.com/a/b`.
/// `None` for local paths and other transports.
pub fn web(url: &str) -> Option<String> {
    let (host, path) = host_path(url)?;
    // An http(s) URL is the page's own address, port and all; an ssh port isn't.
    let (scheme, port) = match url.trim().split_once("://") {
        Some((s @ ("http" | "https"), rest)) => {
            let authority = rest.split('/').next().unwrap_or_default();
            let port = authority.rsplit('@').next().and_then(|h| h.split_once(':')).map(|(_, p)| format!(":{p}"));
            (s, port)
        }
        _ => ("https", None),
    };
    Some(format!("{scheme}://{host}{}/{path}", port.unwrap_or_default()))
}

/// Host (no user, no port) and path (no `.git`) of an ssh, scp-like or http(s) URL.
fn host_path(url: &str) -> Option<(String, String)> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let (authority, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            if !matches!(scheme, "http" | "https" | "ssh" | "git" | "git+ssh") {
                return None;
            }
            rest.split_once('/')?
        }
        // scp-like `user@host:path`; a `/` before the `:` makes it a local path.
        None => {
            let (a, p) = url.split_once(':')?;
            if a.contains('/') || a.len() == 1 {
                return None;
            }
            (a, p)
        }
    };
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| (host.to_string(), path.trim_start_matches('/').to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_web() {
        assert_eq!(short("git@github.com:alvgaona/glance.git"), "github.com/alvgaona/glance");
        assert_eq!(short("https://me@github.com/a/b.git/"), "github.com/a/b");
        assert_eq!(short("ssh://git@host:2222/a/b"), "host/a/b");
        assert_eq!(short("../up"), "../up");
        assert_eq!(short("/srv/git/x.git"), "/srv/git/x.git");
        assert_eq!(short("file:///srv/git/x.git"), "/srv/git/x.git");
        assert_eq!(web("git@github.com:a/b.git").as_deref(), Some("https://github.com/a/b"));
        assert_eq!(web("../up"), None);
        assert_eq!(
            web("https://git.example.com:8443/team/repo.git").as_deref(),
            Some("https://git.example.com:8443/team/repo")
        );
        assert_eq!(web("ssh://git@host:2222/a/b").as_deref(), Some("https://host/a/b"));
        // A Windows drive letter is a path, not a host.
        assert_eq!(web("C:/repos/x"), None);
    }
}

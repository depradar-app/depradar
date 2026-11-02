//! Fetches manifest and lockfile content from a GitHub repository without
//! cloning it. Manifest/lockfile *discovery* uses the Git Trees API (one
//! recursive call lists every path in the repo); file *content* comes from
//! `raw.githubusercontent.com`, which has no 1 MB cap (unlike the Contents
//! API, which returns an empty body for larger files) and isn't counted
//! against the REST API rate limit.

use reqwest::{Client, RequestBuilder, Response};
use serde::Deserialize;

use super::{is_ignored_path, FileSource};
use crate::error::AppError;
use crate::registry::Endpoints;

fn valid_owner(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn valid_repo(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Parses `owner/repo`, an https/ssh GitHub URL (also a browser URL such as
/// `https://github.com/owner/repo/tree/main/src?tab=x`, with or without
/// `www.` / `.git`) into (owner, repo).
pub fn parse_repo(input: &str) -> Option<(String, String)> {
    let mut s = input.trim();
    s = s.split(['?', '#']).next().unwrap_or(s);

    if let Some(rest) = s.strip_prefix("git@github.com:") {
        s = rest;
    } else if let Some(idx) = s.find("://") {
        let rest = &s[idx + 3..];
        let rest = rest.strip_prefix("git@").unwrap_or(rest);
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        s = rest.strip_prefix("github.com/")?;
    } else {
        let rest = s.strip_prefix("www.").unwrap_or(s);
        s = rest.strip_prefix("github.com/").unwrap_or(s);
    }

    let mut segments = s.split('/').filter(|p| !p.is_empty());
    let owner = segments.next()?.trim();
    let repo = segments.next()?.trim();
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if !valid_owner(owner) || !valid_repo(repo) {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// Percent-encodes a path (or branch name), keeping `/` unless `keep_slash`
/// is false.
fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric()
            || matches!(c, '-' | '_' | '.' | '~')
            || (keep_slash && c == '/')
        {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn with_auth(mut req: RequestBuilder, token: Option<&str>) -> RequestBuilder {
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    req
}

fn api_get(client: &Client, url: &str, token: Option<&str>) -> RequestBuilder {
    with_auth(
        client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28"),
        token,
    )
}

/// Turns a non-success GitHub response into the matching structured error.
/// A 403 is only a rate limit when GitHub says so (exhausted quota or a
/// `Retry-After`); otherwise it's a permissions problem (a token without
/// access, an SSO-protected organisation...).
fn status_error(resp: &Response, subject: &str) -> AppError {
    let status = resp.status().as_u16();
    let headers = resp.headers();
    let quota_exhausted = headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        == Some("0")
        || headers.contains_key("retry-after");
    match status {
        404 => AppError::with_detail("github_repo_not_found", subject),
        401 => AppError::new("github_unauthorized"),
        429 => AppError::new("github_rate_limited"),
        403 if quota_exhausted => AppError::new("github_rate_limited"),
        403 => AppError::new("github_forbidden"),
        other => AppError::with_detail("github_http_error", other.to_string()),
    }
}

#[derive(Deserialize)]
struct RepoMetadata {
    default_branch: String,
}

/// Fetches the repository's default branch name. This also serves as an
/// early existence/rate-limit check before any tree listing is attempted.
pub async fn default_branch(
    client: &Client,
    ep: &Endpoints,
    owner: &str,
    repo: &str,
    token: Option<&str>,
) -> Result<String, AppError> {
    let url = format!("{}/repos/{owner}/{repo}", ep.github_api);
    let resp = api_get(client, &url, token)
        .send()
        .await
        .map_err(|e| AppError::with_detail("network_error", e.to_string()))?;

    if resp.status().as_u16() == 200 {
        let meta: RepoMetadata = resp
            .json()
            .await
            .map_err(|e| AppError::with_detail("network_error", e.to_string()))?;
        return Ok(meta.default_branch);
    }
    Err(status_error(&resp, &format!("{owner}/{repo}")))
}

#[derive(Deserialize)]
struct TreeResponse {
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
}

/// The result of recursively listing a repository's file tree.
#[derive(Debug)]
pub struct TreeListing {
    /// Every blob path in the repo, minus anything under an ignored
    /// directory (see `source::IGNORE_DIRS`) — not filtered to manifest or
    /// lockfile names, callers do that themselves.
    pub paths: Vec<String>,
    /// True if GitHub's response was too large to return in full — some
    /// deeply-nested files may be missing from `paths`.
    pub truncated: bool,
}

fn matching_blob_paths(entries: Vec<TreeEntry>) -> Vec<String> {
    entries
        .into_iter()
        .filter(|entry| entry.entry_type == "blob" && !is_ignored_path(&entry.path))
        .map(|entry| entry.path)
        .collect()
}

/// Recursively lists every file in `branch` in a single call, so manifests
/// nested in a monorepo (not just at the repo root) can be discovered.
pub async fn list_tree(
    client: &Client,
    ep: &Endpoints,
    owner: &str,
    repo: &str,
    branch: &str,
    token: Option<&str>,
) -> Result<TreeListing, AppError> {
    let url = format!(
        "{}/repos/{owner}/{repo}/git/trees/{}?recursive=1",
        ep.github_api,
        percent_encode(branch, false)
    );
    let resp = api_get(client, &url, token)
        .send()
        .await
        .map_err(|e| AppError::with_detail("network_error", e.to_string()))?;

    if resp.status().as_u16() == 200 {
        let body: TreeResponse = resp
            .json()
            .await
            .map_err(|e| AppError::with_detail("network_error", e.to_string()))?;
        return Ok(TreeListing {
            paths: matching_blob_paths(body.tree),
            truncated: body.truncated,
        });
    }
    Err(status_error(&resp, &format!("{owner}/{repo}")))
}

/// Fetches the content of a single file at `path`. `Ok(None)` means the
/// file doesn't exist (404): not an error, just an absent manifest/lockfile.
pub async fn fetch_file(
    client: &Client,
    ep: &Endpoints,
    (owner, repo, branch): (&str, &str, &str),
    path: &str,
    token: Option<&str>,
) -> Result<Option<String>, AppError> {
    let url = format!(
        "{}/{owner}/{repo}/{}/{}",
        ep.github_raw,
        percent_encode(branch, true),
        percent_encode(path, true)
    );
    let resp = with_auth(client.get(&url), token)
        .send()
        .await
        .map_err(|e| AppError::with_detail("network_error", e.to_string()))?;

    match resp.status().as_u16() {
        200 => {
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AppError::with_detail("network_error", e.to_string()))?;
            Ok(Some(String::from_utf8_lossy(&bytes).to_string()))
        }
        404 => Ok(None),
        _ => Err(status_error(&resp, path)),
    }
}

/// Reads files of one repository at one branch.
pub struct GithubSource<'a> {
    pub client: &'a Client,
    pub ep: &'a Endpoints,
    pub owner: &'a str,
    pub repo: &'a str,
    pub branch: &'a str,
    pub token: Option<&'a str>,
}

impl FileSource for GithubSource<'_> {
    async fn read(&self, rel: &str) -> Result<Option<String>, AppError> {
        fetch_file(
            self.client,
            self.ep,
            (self.owner, self.repo, self.branch),
            rel,
            self.token,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn pair(a: &str, b: &str) -> Option<(String, String)> {
        Some((a.to_string(), b.to_string()))
    }

    #[test]
    fn parses_owner_repo_shorthand() {
        assert_eq!(parse_repo("rust-lang/rust"), pair("rust-lang", "rust"));
    }

    #[test]
    fn parses_full_urls_with_trailing_slash_or_git_suffix() {
        assert_eq!(
            parse_repo("https://github.com/rust-lang/rust/"),
            pair("rust-lang", "rust")
        );
        assert_eq!(
            parse_repo("https://github.com/rust-lang/rust.git"),
            pair("rust-lang", "rust")
        );
        assert_eq!(
            parse_repo("github.com/rust-lang/rust"),
            pair("rust-lang", "rust")
        );
    }

    #[test]
    fn parses_browser_ssh_and_www_urls() {
        for input in [
            "https://github.com/rust-lang/rust/tree/master/src",
            "git@github.com:rust-lang/rust.git",
            "https://www.github.com/rust-lang/rust",
            "https://github.com/rust-lang/rust?tab=readme",
            "https://github.com/rust-lang/rust#readme",
            "http://github.com/rust-lang/rust",
            "ssh://git@github.com/rust-lang/rust.git",
            "www.github.com/rust-lang/rust",
            "  rust-lang/rust/  ",
        ] {
            assert_eq!(parse_repo(input), pair("rust-lang", "rust"), "{input}");
        }
    }

    #[test]
    fn rejects_invalid_input() {
        for input in [
            "not-a-repo",
            "",
            "https://gitlab.com/a/b",
            "a b/c",
            "../x",
            "a/..",
            "-/",
            "o/r$",
        ] {
            assert_eq!(parse_repo(input), None, "{input}");
        }
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(percent_encode("feature/x y", false), "feature%2Fx%20y");
        assert_eq!(percent_encode("a b/c#d", true), "a%20b/c%23d");
        assert_eq!(percent_encode("é", true), "%C3%A9");
    }

    #[test]
    fn matching_blob_paths_filters_trees_and_ignored_dirs() {
        let entries = vec![
            TreeEntry {
                path: "Cargo.toml".to_string(),
                entry_type: "blob".to_string(),
            },
            TreeEntry {
                path: "packages/api/Cargo.toml".to_string(),
                entry_type: "blob".to_string(),
            },
            TreeEntry {
                path: "node_modules/foo/package.json".to_string(),
                entry_type: "blob".to_string(),
            },
            TreeEntry {
                path: "src".to_string(),
                entry_type: "tree".to_string(),
            },
        ];

        assert_eq!(
            matching_blob_paths(entries),
            vec![
                "Cargo.toml".to_string(),
                "packages/api/Cargo.toml".to_string(),
            ]
        );
    }

    async fn setup() -> (MockServer, Endpoints, Client) {
        let server = MockServer::start().await;
        let ep = Endpoints::all_at(&server.uri());
        (server, ep, Client::new())
    }

    #[tokio::test]
    async fn default_branch_reads_the_repository_metadata() {
        let (server, ep, client) = setup().await;
        Mock::given(method("GET"))
            .and(path("/gh/repos/o/r"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"default_branch": "dev"})),
            )
            .mount(&server)
            .await;
        assert_eq!(
            default_branch(&client, &ep, "o", "r", Some("tok"))
                .await
                .unwrap(),
            "dev"
        );
    }

    #[tokio::test]
    async fn http_statuses_map_to_precise_error_codes() {
        let (server, ep, client) = setup().await;
        type Case<'a> = (&'a str, u16, Option<(&'a str, &'a str)>, &'a str);
        let cases: [Case; 8] = [
            ("nf", 404, None, "github_repo_not_found"),
            ("unauth", 401, None, "github_unauthorized"),
            ("rate429", 429, None, "github_rate_limited"),
            (
                "rate403",
                403,
                Some(("x-ratelimit-remaining", "0")),
                "github_rate_limited",
            ),
            (
                "retry403",
                403,
                Some(("retry-after", "30")),
                "github_rate_limited",
            ),
            (
                "forbidden",
                403,
                Some(("x-ratelimit-remaining", "42")),
                "github_forbidden",
            ),
            ("plain403", 403, None, "github_forbidden"),
            ("boom", 500, None, "github_http_error"),
        ];
        for (repo, status, hdr, code) in cases {
            let mut t = ResponseTemplate::new(status);
            if let Some((k, v)) = hdr {
                t = t.insert_header(k, v);
            }
            Mock::given(method("GET"))
                .and(path(format!("/gh/repos/o/{repo}")))
                .respond_with(t)
                .mount(&server)
                .await;
            let err = default_branch(&client, &ep, "o", repo, None)
                .await
                .unwrap_err();
            assert_eq!(err.code, code, "{repo}");
        }
        let err = default_branch(&client, &ep, "o", "boom", None)
            .await
            .unwrap_err();
        assert_eq!(err.detail.as_deref(), Some("500"));
    }

    #[tokio::test]
    async fn network_failures_are_reported_as_such() {
        let ep = Endpoints::all_at("http://127.0.0.1:1");
        let client = Client::new();
        assert_eq!(
            default_branch(&client, &ep, "o", "r", None)
                .await
                .unwrap_err()
                .code,
            "network_error"
        );
        assert_eq!(
            list_tree(&client, &ep, "o", "r", "main", None)
                .await
                .unwrap_err()
                .code,
            "network_error"
        );
        assert_eq!(
            fetch_file(&client, &ep, ("o", "r", "main"), "x", None)
                .await
                .unwrap_err()
                .code,
            "network_error"
        );
    }

    #[tokio::test]
    async fn malformed_json_is_a_network_error() {
        let (server, ep, client) = setup().await;
        Mock::given(method("GET"))
            .and(path("/gh/repos/o/r"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/gh/repos/o/r/git/trees/main"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;
        assert_eq!(
            default_branch(&client, &ep, "o", "r", None)
                .await
                .unwrap_err()
                .code,
            "network_error"
        );
        assert_eq!(
            list_tree(&client, &ep, "o", "r", "main", None)
                .await
                .unwrap_err()
                .code,
            "network_error"
        );
    }

    #[tokio::test]
    async fn list_tree_encodes_the_branch_and_reports_truncation() {
        let (server, ep, client) = setup().await;
        Mock::given(method("GET"))
            .and(path("/gh/repos/o/r/git/trees/feature%2Fx"))
            .and(query_param("recursive", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "truncated": true,
                "tree": [
                    {"path": "Cargo.toml", "type": "blob"},
                    {"path": "src", "type": "tree"}
                ]
            })))
            .mount(&server)
            .await;
        let listing = list_tree(&client, &ep, "o", "r", "feature/x", None)
            .await
            .unwrap();
        assert_eq!(listing.paths, vec!["Cargo.toml".to_string()]);
        assert!(listing.truncated);

        let err = list_tree(&client, &ep, "o", "r", "gone", None)
            .await
            .unwrap_err();
        assert_eq!(err.code, "github_repo_not_found");
    }

    #[tokio::test]
    async fn fetch_file_returns_the_full_body_even_for_large_files() {
        let (server, ep, client) = setup().await;
        let big = "x".repeat(2_000_000);
        Mock::given(method("GET"))
            .and(path("/raw/o/r/main/dir/package-lock.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(big.clone()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/raw/o/r/main/forbidden.txt"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let src = GithubSource {
            client: &client,
            ep: &ep,
            owner: "o",
            repo: "r",
            branch: "main",
            token: None,
        };
        assert_eq!(src.read("dir/package-lock.json").await.unwrap(), Some(big));
        assert_eq!(src.read("absent.json").await.unwrap(), None);
        assert_eq!(
            src.read("forbidden.txt").await.unwrap_err().code,
            "github_forbidden"
        );
    }
}

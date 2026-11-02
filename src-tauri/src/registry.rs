//! Minimal clients for querying the latest *stable* published version of a
//! package on each of the supported registries.
//!
//! Every lookup distinguishes "the registry doesn't know this package"
//! (`Lookup::NotFound`) from "the registry could not be reached / answered
//! with an error" (`Lookup::Failed`), so a flaky network is never silently
//! reported as an unknown package.

use std::cmp::Ordering;
use std::time::Duration;

use reqwest::Client;

use crate::version::is_prerelease;

/// Base URLs of every external service, overridable so tests can point the
/// whole scanner at a local mock server.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub crates_index: String,
    pub npm: String,
    pub pypi: String,
    pub go_proxy: String,
    pub packagist: String,
    pub rubygems: String,
    pub maven: String,
    pub osv: String,
    pub github_api: String,
    pub github_raw: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            crates_index: "https://index.crates.io".into(),
            npm: "https://registry.npmjs.org".into(),
            pypi: "https://pypi.org".into(),
            go_proxy: "https://proxy.golang.org".into(),
            packagist: "https://repo.packagist.org".into(),
            rubygems: "https://rubygems.org".into(),
            maven: "https://repo1.maven.org/maven2".into(),
            osv: "https://api.osv.dev".into(),
            github_api: "https://api.github.com".into(),
            github_raw: "https://raw.githubusercontent.com".into(),
        }
    }
}

#[cfg(test)]
impl Endpoints {
    /// Points every service at the same base URL (used by tests).
    pub fn all_at(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            crates_index: format!("{base}/crates"),
            npm: format!("{base}/npm"),
            pypi: format!("{base}/pypi"),
            go_proxy: format!("{base}/go"),
            packagist: format!("{base}/packagist"),
            rubygems: format!("{base}/rubygems"),
            maven: format!("{base}/maven"),
            osv: format!("{base}/osv"),
            github_api: format!("{base}/gh"),
            github_raw: format!("{base}/raw"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Found(String),
    NotFound,
    Failed,
}

const RETRY_DELAY: Duration = Duration::from_millis(150);

/// GET with one retry on a network error / 429 / 5xx. `Ok(None)` is a 404/410.
async fn get_text(client: &Client, url: &str) -> Result<Option<String>, ()> {
    for attempt in 0..2 {
        if let Ok(resp) = client.get(url).send().await {
            let status = resp.status();
            if status.is_success() {
                return resp.text().await.map(Some).map_err(|_| ());
            }
            if status.as_u16() == 404 || status.as_u16() == 410 {
                return Ok(None);
            }
            if !(status.as_u16() == 429 || status.is_server_error()) {
                return Err(());
            }
        }
        if attempt == 0 {
            tokio::time::sleep(RETRY_DELAY).await;
        }
    }
    Err(())
}

async fn get_json(client: &Client, url: &str) -> Result<Option<serde_json::Value>, ()> {
    match get_text(client, url).await? {
        None => Ok(None),
        Some(body) => serde_json::from_str(&body).map(Some).map_err(|_| ()),
    }
}

fn json_lookup(
    result: Result<Option<serde_json::Value>, ()>,
    extract: impl FnOnce(&serde_json::Value) -> Option<String>,
) -> Lookup {
    match result {
        Err(()) => Lookup::Failed,
        Ok(None) => Lookup::NotFound,
        Ok(Some(json)) => extract(&json).map_or(Lookup::NotFound, Lookup::Found),
    }
}

// ---------------------------------------------------------------- crates.io

/// Path of a crate in the sparse index (`https://index.crates.io`).
fn crates_index_path(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    match n.len() {
        1 => format!("1/{n}"),
        2 => format!("2/{n}"),
        3 => format!("3/{}/{n}", &n[..1]),
        _ => format!("{}/{}/{n}", &n[..2], &n[2..4]),
    }
}

/// Highest non-yanked version in a sparse-index file (one JSON object per
/// line), preferring stable releases.
fn pick_crates_version(body: &str) -> Option<String> {
    let mut stable: Option<semver::Version> = None;
    let mut any: Option<semver::Version> = None;
    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if entry.get("yanked").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        let Some(v) = entry
            .get("vers")
            .and_then(|v| v.as_str())
            .and_then(|v| semver::Version::parse(v).ok())
        else {
            continue;
        };
        if v.pre.is_empty() && stable.as_ref().is_none_or(|s| v > *s) {
            stable = Some(v.clone());
        }
        if any.as_ref().is_none_or(|s| v > *s) {
            any = Some(v);
        }
    }
    stable.or(any).map(|v| v.to_string())
}

pub async fn latest_crates_io(client: &Client, ep: &Endpoints, name: &str) -> Lookup {
    let url = format!("{}/{}", ep.crates_index, crates_index_path(name));
    match get_text(client, &url).await {
        Err(()) => Lookup::Failed,
        Ok(None) => Lookup::NotFound,
        Ok(Some(body)) => pick_crates_version(&body).map_or(Lookup::NotFound, Lookup::Found),
    }
}

// --------------------------------------------------------------------- npm

fn npm_path_segment(name: &str) -> String {
    // Scoped packages ("@scope/name") need their '/' encoded in the npm
    // registry URL path.
    if let Some(rest) = name.strip_prefix('@') {
        format!("@{}", rest.replacen('/', "%2f", 1))
    } else {
        name.to_string()
    }
}

pub async fn latest_npm(client: &Client, ep: &Endpoints, name: &str) -> Lookup {
    let url = format!("{}/{}/latest", ep.npm, npm_path_segment(name));
    json_lookup(get_json(client, &url).await, |j| {
        j.get("version").and_then(|v| v.as_str()).map(String::from)
    })
}

// -------------------------------------------------------------------- PyPI

pub async fn latest_pypi(client: &Client, ep: &Endpoints, name: &str) -> Lookup {
    let url = format!("{}/pypi/{name}/json", ep.pypi);
    json_lookup(get_json(client, &url).await, |j| {
        j.get("info")?
            .get("version")
            .and_then(|v| v.as_str())
            .map(String::from)
    })
}

// ---------------------------------------------------------------------- Go

/// The Go module proxy protocol requires each uppercase letter in a module
/// path to be escaped as `!` followed by its lowercase form.
fn encode_go_module_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_ascii_uppercase() {
            out.push('!');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `github.com/a/b/v3` → (`github.com/a/b`, 3); no suffix → (path, 1).
fn split_go_major(module: &str) -> (&str, u32) {
    if let Some((base, last)) = module.rsplit_once('/') {
        if let Some(n) = last.strip_prefix('v').and_then(|n| n.parse::<u32>().ok()) {
            if n >= 2 {
                return (base, n);
            }
        }
    }
    (module, 1)
}

async fn go_latest_of(client: &Client, ep: &Endpoints, module: &str) -> Lookup {
    let url = format!("{}/{}/@latest", ep.go_proxy, encode_go_module_path(module));
    json_lookup(get_json(client, &url).await, |j| {
        j.get("Version").and_then(|v| v.as_str()).map(String::from)
    })
}

/// Latest version of a Go module, *including* newer major versions: a Go
/// major bump lives under a different module path (`.../v3`), which
/// `@latest` of the current path never reveals, so the next majors are probed.
pub async fn latest_go(client: &Client, ep: &Endpoints, module: &str) -> Lookup {
    let first = go_latest_of(client, ep, module).await;
    let Lookup::Found(mut best) = first else {
        return first;
    };
    if module.starts_with("gopkg.in/") {
        return Lookup::Found(best);
    }
    let (base, major) = split_go_major(module);
    for next in (major + 1)..=(major + 5) {
        match go_latest_of(client, ep, &format!("{base}/v{next}")).await {
            Lookup::Found(v) => best = v,
            _ => break,
        }
    }
    Lookup::Found(best)
}

// ---------------------------------------------------------------- Packagist

pub async fn latest_packagist(client: &Client, ep: &Endpoints, name: &str) -> Lookup {
    let name = name.to_ascii_lowercase();
    let url = format!("{}/p2/{name}.json", ep.packagist);
    json_lookup(get_json(client, &url).await, |j| {
        let versions: Vec<&str> = j
            .get("packages")?
            .get(name.as_str())?
            .as_array()?
            .iter()
            .filter_map(|v| v.get("version").and_then(|v| v.as_str()))
            .collect();
        // The p2 endpoint lists releases newest-first.
        versions
            .iter()
            .find(|v| !is_prerelease(v))
            .or_else(|| versions.iter().find(|v| !v.starts_with("dev-")))
            .map(|v| v.to_string())
    })
}

// ----------------------------------------------------------------- RubyGems

pub async fn latest_rubygems(client: &Client, ep: &Endpoints, name: &str) -> Lookup {
    let url = format!("{}/api/v1/gems/{name}.json", ep.rubygems);
    json_lookup(get_json(client, &url).await, |j| {
        j.get("version").and_then(|v| v.as_str()).map(String::from)
    })
}

// -------------------------------------------------------------------- Maven

fn numeric_prefix(v: &str) -> Vec<u64> {
    v.split(['.', '-'])
        .map_while(|p| p.parse::<u64>().ok())
        .collect()
}

fn compare_maven_versions(a: &str, b: &str) -> Ordering {
    let (pa, pb) = (numeric_prefix(a), numeric_prefix(b));
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let (x, y) = (
            pa.get(i).copied().unwrap_or(0),
            pb.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x.cmp(&y);
        }
    }
    Ordering::Equal
}

/// Highest stable version listed in a `maven-metadata.xml`. Falls back to the
/// highest version of any kind when only pre-releases exist.
fn pick_maven_version(xml: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let versions: Vec<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("versions"))
        .flat_map(|n| n.children().filter(|c| c.has_tag_name("version")))
        .filter_map(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let pick = |candidates: Vec<&String>| -> Option<String> {
        let mut best: Option<&String> = None;
        for v in candidates {
            // `>=`: among equal numeric prefixes, the later listing wins.
            if best.is_none_or(|b| compare_maven_versions(v, b) != Ordering::Less) {
                best = Some(v);
            }
        }
        best.cloned()
    };

    pick(versions.iter().filter(|v| !is_prerelease(v)).collect())
        .or_else(|| pick(versions.iter().collect()))
}

/// `coordinate` is `"groupId:artifactId"`.
pub async fn latest_maven(client: &Client, ep: &Endpoints, coordinate: &str) -> Lookup {
    let Some((group, artifact)) = coordinate.split_once(':') else {
        return Lookup::NotFound;
    };
    let url = format!(
        "{}/{}/{artifact}/maven-metadata.xml",
        ep.maven,
        group.replace('.', "/")
    );
    match get_text(client, &url).await {
        Err(()) => Lookup::Failed,
        Ok(None) => Lookup::NotFound,
        Ok(Some(xml)) => pick_maven_version(&xml).map_or(Lookup::NotFound, Lookup::Found),
    }
}

/// Fetches a `.pom` from Maven Central (used to resolve imported BOMs).
pub async fn fetch_maven_pom(
    client: &Client,
    ep: &Endpoints,
    group: &str,
    artifact: &str,
    version: &str,
) -> Option<String> {
    let url = format!(
        "{}/{}/{artifact}/{version}/{artifact}-{version}.pom",
        ep.maven,
        group.replace('.', "/")
    );
    get_text(client, &url).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client() -> Client {
        Client::new()
    }

    async fn serve(server: &MockServer, p: &str, template: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(template)
            .mount(server)
            .await;
    }

    #[test]
    fn npm_path_segment_leaves_unscoped_names_unchanged() {
        assert_eq!(npm_path_segment("lodash"), "lodash");
    }

    #[test]
    fn npm_path_segment_encodes_the_slash_in_scoped_names() {
        assert_eq!(npm_path_segment("@scope/name"), "@scope%2fname");
    }

    #[test]
    fn encode_go_module_path_leaves_lowercase_paths_unchanged() {
        assert_eq!(
            encode_go_module_path("github.com/spf13/cobra"),
            "github.com/spf13/cobra"
        );
    }

    #[test]
    fn encode_go_module_path_escapes_uppercase_letters() {
        assert_eq!(
            encode_go_module_path("github.com/BurntSushi/toml"),
            "github.com/!burnt!sushi/toml"
        );
    }

    #[test]
    fn splits_go_major_suffix() {
        assert_eq!(split_go_major("github.com/a/b"), ("github.com/a/b", 1));
        assert_eq!(split_go_major("github.com/a/b/v3"), ("github.com/a/b", 3));
        assert_eq!(split_go_major("github.com/a/v1"), ("github.com/a/v1", 1));
        assert_eq!(split_go_major("github.com/a/vx"), ("github.com/a/vx", 1));
    }

    #[test]
    fn crates_index_paths_follow_the_sparse_layout() {
        assert_eq!(crates_index_path("a"), "1/a");
        assert_eq!(crates_index_path("ab"), "2/ab");
        assert_eq!(crates_index_path("abc"), "3/a/abc");
        assert_eq!(crates_index_path("Serde"), "se/rd/serde");
    }

    #[test]
    fn crates_version_skips_yanked_and_prefers_stable() {
        let body = r#"{"vers":"1.0.0","yanked":false}
{"vers":"1.2.0","yanked":false}
{"vers":"1.3.0","yanked":true}
{"vers":"2.0.0-rc.1","yanked":false}

not json
{"vers":"bogus","yanked":false}
{"yanked":false}
"#;
        assert_eq!(pick_crates_version(body), Some("1.2.0".to_string()));
    }

    #[test]
    fn crates_version_falls_back_to_a_prerelease() {
        let body = r#"{"vers":"0.1.0-alpha.1","yanked":false}
{"vers":"0.1.0-alpha.2","yanked":false}"#;
        assert_eq!(pick_crates_version(body), Some("0.1.0-alpha.2".to_string()));
        assert_eq!(pick_crates_version(""), None);
    }

    #[test]
    fn maven_picks_highest_stable_version() {
        let xml = r#"<metadata><versioning>
            <release>7.1.0-RC1</release>
            <versions>
              <version>6.1.3</version>
              <version>7.0.0</version>
              <version>7.1.0-RC1</version>
              <version>6.9.9</version>
            </versions></versioning></metadata>"#;
        assert_eq!(pick_maven_version(xml), Some("7.0.0".to_string()));
    }

    #[test]
    fn maven_falls_back_to_prerelease_when_nothing_is_stable() {
        let xml = "<metadata><versioning><versions>\
            <version>1.0.0-M1</version><version>1.0.0-M2</version>\
            </versions></versioning></metadata>";
        assert_eq!(pick_maven_version(xml), Some("1.0.0-M2".to_string()));
        assert_eq!(pick_maven_version("not xml <<<"), None);
        assert_eq!(pick_maven_version("<metadata/>"), None);
    }

    #[test]
    fn maven_version_comparison_is_numeric() {
        assert_eq!(compare_maven_versions("2.10.0", "2.9.0"), Ordering::Greater);
        assert_eq!(compare_maven_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_maven_versions("1.0.0", "1.0.1"), Ordering::Less);
    }

    #[tokio::test]
    async fn crates_io_lookup_reads_the_sparse_index() {
        let server = MockServer::start().await;
        let ep = Endpoints::all_at(&server.uri());
        serve(
            &server,
            "/crates/se/rd/serde",
            ResponseTemplate::new(200).set_body_string(
                "{\"vers\":\"1.0.0\",\"yanked\":false}\n{\"vers\":\"1.0.5\",\"yanked\":false}\n",
            ),
        )
        .await;
        assert_eq!(
            latest_crates_io(&client(), &ep, "serde").await,
            Lookup::Found("1.0.5".into())
        );
        assert_eq!(
            latest_crates_io(&client(), &ep, "missing-crate").await,
            Lookup::NotFound
        );
    }

    #[tokio::test]
    async fn npm_lookup_handles_found_missing_and_scoped() {
        let server = MockServer::start().await;
        let ep = Endpoints::all_at(&server.uri());
        serve(
            &server,
            "/npm/lodash/latest",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"version": "4.17.21"})),
        )
        .await;
        serve(
            &server,
            "/npm/@scope%2fpkg/latest",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"version": "2.0.0"})),
        )
        .await;
        assert_eq!(
            latest_npm(&client(), &ep, "lodash").await,
            Lookup::Found("4.17.21".into())
        );
        assert_eq!(
            latest_npm(&client(), &ep, "@scope/pkg").await,
            Lookup::Found("2.0.0".into())
        );
        assert_eq!(latest_npm(&client(), &ep, "nope").await, Lookup::NotFound);
    }

    #[tokio::test]
    async fn server_errors_are_failures_not_unknown_packages() {
        let server = MockServer::start().await;
        let ep = Endpoints::all_at(&server.uri());
        serve(&server, "/npm/flaky/latest", ResponseTemplate::new(503)).await;
        serve(&server, "/npm/limited/latest", ResponseTemplate::new(429)).await;
        serve(&server, "/npm/denied/latest", ResponseTemplate::new(403)).await;
        serve(
            &server,
            "/npm/garbled/latest",
            ResponseTemplate::new(200).set_body_string("<html>"),
        )
        .await;
        assert_eq!(latest_npm(&client(), &ep, "flaky").await, Lookup::Failed);
        assert_eq!(latest_npm(&client(), &ep, "limited").await, Lookup::Failed);
        assert_eq!(latest_npm(&client(), &ep, "denied").await, Lookup::Failed);
        assert_eq!(latest_npm(&client(), &ep, "garbled").await, Lookup::Failed);
    }

    #[tokio::test]
    async fn unreachable_registry_is_a_failure() {
        let ep = Endpoints::all_at("http://127.0.0.1:1");
        assert_eq!(latest_npm(&client(), &ep, "x").await, Lookup::Failed);
    }

    #[tokio::test]
    async fn a_transient_error_is_retried_once() {
        let server = MockServer::start().await;
        let ep = Endpoints::all_at(&server.uri());
        Mock::given(method("GET"))
            .and(path("/npm/retry/latest"))
            .respond_with(ResponseTemplate::new(502))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        serve(
            &server,
            "/npm/retry/latest",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"version": "1.0.0"})),
        )
        .await;
        assert_eq!(
            latest_npm(&client(), &ep, "retry").await,
            Lookup::Found("1.0.0".into())
        );
    }

    #[tokio::test]
    async fn pypi_rubygems_lookups() {
        let server = MockServer::start().await;
        let ep = Endpoints::all_at(&server.uri());
        serve(
            &server,
            "/pypi/pypi/requests/json",
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"info": {"version": "2.32.3"}})),
        )
        .await;
        serve(
            &server,
            "/rubygems/api/v1/gems/rails.json",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"version": "7.1.0"})),
        )
        .await;
        serve(
            &server,
            "/pypi/pypi/odd/json",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"info": {}})),
        )
        .await;
        assert_eq!(
            latest_pypi(&client(), &ep, "requests").await,
            Lookup::Found("2.32.3".into())
        );
        assert_eq!(latest_pypi(&client(), &ep, "odd").await, Lookup::NotFound);
        assert_eq!(
            latest_rubygems(&client(), &ep, "rails").await,
            Lookup::Found("7.1.0".into())
        );
        assert_eq!(
            latest_rubygems(&client(), &ep, "nope").await,
            Lookup::NotFound
        );
    }

    #[tokio::test]
    async fn packagist_skips_dev_and_prerelease_versions() {
        let server = MockServer::start().await;
        let ep = Endpoints::all_at(&server.uri());
        serve(
            &server,
            "/packagist/p2/vendor/pkg.json",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "packages": {"vendor/pkg": [
                    {"version": "dev-main"},
                    {"version": "4.0.0-RC1"},
                    {"version": "3.5.0"},
                    {"version": "3.4.0"}
                ]}
            })),
        )
        .await;
        serve(
            &server,
            "/packagist/p2/vendor/onlybeta.json",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "packages": {"vendor/onlybeta": [
                    {"version": "dev-main"},
                    {"version": "1.0.0-beta1"}
                ]}
            })),
        )
        .await;
        assert_eq!(
            latest_packagist(&client(), &ep, "Vendor/Pkg").await,
            Lookup::Found("3.5.0".into())
        );
        assert_eq!(
            latest_packagist(&client(), &ep, "vendor/onlybeta").await,
            Lookup::Found("1.0.0-beta1".into())
        );
        assert_eq!(
            latest_packagist(&client(), &ep, "vendor/none").await,
            Lookup::NotFound
        );
    }

    #[tokio::test]
    async fn go_lookup_probes_newer_major_versions() {
        let server = MockServer::start().await;
        let ep = Endpoints::all_at(&server.uri());
        serve(
            &server,
            "/go/github.com/a/b/@latest",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"Version": "v1.9.0"})),
        )
        .await;
        serve(
            &server,
            "/go/github.com/a/b/v2/@latest",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"Version": "v2.1.0"})),
        )
        .await;
        serve(
            &server,
            "/go/github.com/a/b/v3/@latest",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"Version": "v3.0.1"})),
        )
        .await;
        assert_eq!(
            latest_go(&client(), &ep, "github.com/a/b").await,
            Lookup::Found("v3.0.1".into())
        );
        assert_eq!(
            latest_go(&client(), &ep, "github.com/a/missing").await,
            Lookup::NotFound
        );
        // A module that exists but whose probe is not needed (gopkg.in).
        serve(
            &server,
            "/go/gopkg.in/yaml.v3/@latest",
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"Version": "v3.0.1"})),
        )
        .await;
        assert_eq!(
            latest_go(&client(), &ep, "gopkg.in/yaml.v3").await,
            Lookup::Found("v3.0.1".into())
        );
    }

    #[tokio::test]
    async fn maven_lookup_reads_metadata_and_pom() {
        let server = MockServer::start().await;
        let ep = Endpoints::all_at(&server.uri());
        serve(
            &server,
            "/maven/org/junit/jupiter/junit-jupiter/maven-metadata.xml",
            ResponseTemplate::new(200).set_body_string(
                "<metadata><versioning><versions><version>5.13.0-M3</version>\
                 <version>6.1.3</version></versions></versioning></metadata>",
            ),
        )
        .await;
        serve(
            &server,
            "/maven/g/a/1.0/a-1.0.pom",
            ResponseTemplate::new(200).set_body_string("<project/>"),
        )
        .await;
        assert_eq!(
            latest_maven(&client(), &ep, "org.junit.jupiter:junit-jupiter").await,
            Lookup::Found("6.1.3".into())
        );
        assert_eq!(
            latest_maven(&client(), &ep, "org.nope:nope").await,
            Lookup::NotFound
        );
        assert_eq!(
            latest_maven(&client(), &ep, "no-colon").await,
            Lookup::NotFound
        );
        assert_eq!(
            fetch_maven_pom(&client(), &ep, "g", "a", "1.0").await,
            Some("<project/>".to_string())
        );
        assert_eq!(fetch_maven_pom(&client(), &ep, "g", "a", "9").await, None);
    }

    #[test]
    fn default_endpoints_point_at_the_real_services() {
        let ep = Endpoints::default();
        assert!(ep.osv.starts_with("https://api.osv.dev"));
        assert!(ep.maven.contains("repo1.maven.org"));
    }
}

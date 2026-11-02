//! The scan pipeline shared by local folders and GitHub repositories: find
//! manifests, parse them, read installed versions from lockfiles, look up
//! the latest versions, then check everything against OSV in one pass.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::future::join_all;
use reqwest::Client;
use tokio::sync::Semaphore;

use crate::ecosystem::{go_eco, java_eco, npm_eco, php_eco, python_eco, ruby_eco, rust_eco};
use crate::error::AppError;
use crate::models::{reason, DepStatus, DependencyInfo, ManifestScan, ScanReport, Warning};
use crate::osv::{self, VulnQuery};
use crate::registry::{self, Endpoints, Lookup};
use crate::source::github::{self, GithubSource};
use crate::source::local::{self, LocalSource};
use crate::source::{
    basename, dirname, ecosystem_for, join_path, manifest_lockfiles, within_depth, FileSource,
    LOCKFILE_FILES, MANIFEST_FILES,
};
use crate::version::{self, Ecosystem};

/// Maximum simultaneous registry requests across the whole scan.
const MAX_CONCURRENT_LOOKUPS: usize = 8;
/// Manifests processed at the same time.
const MANIFEST_CONCURRENCY: usize = 4;
/// How long a successful "latest version" answer is reused across scans.
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
/// Levels of `<parent>` poms followed when resolving a Maven version.
const MAX_POM_ANCESTORS: usize = 6;

fn build_http_client() -> Client {
    Client::builder()
        .user_agent(concat!(
            "DepRadar/",
            env!("CARGO_PKG_VERSION"),
            " (dependency update checker, desktop app)"
        ))
        .timeout(Duration::from_secs(15))
        .build()
        .expect("failed to build http client")
}

type LookupKey = (Ecosystem, String);

/// Everything a scan needs: one shared HTTP client (connection pooling), the
/// service endpoints, a request limiter and a short-lived result cache.
pub struct ScanCtx {
    pub client: Client,
    pub ep: Endpoints,
    cache: Mutex<HashMap<LookupKey, (Instant, String)>>,
    /// One async lock per package being fetched, so concurrent lookups of the
    /// same package (several manifests of a monorepo) share a single request.
    in_flight: Mutex<HashMap<LookupKey, Arc<tokio::sync::Mutex<()>>>>,
    limiter: Semaphore,
}

impl ScanCtx {
    pub fn new(ep: Endpoints) -> Self {
        Self {
            client: build_http_client(),
            ep,
            cache: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
            limiter: Semaphore::new(MAX_CONCURRENT_LOOKUPS),
        }
    }

    fn cached(&self, key: &LookupKey) -> Option<String> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .get(key)
            .filter(|(at, _)| at.elapsed() < CACHE_TTL)
            .map(|(_, v)| v.clone())
    }

    fn store(&self, key: LookupKey, latest: String) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(key, (Instant::now(), latest));
    }
}

impl Default for ScanCtx {
    fn default() -> Self {
        Self::new(Endpoints::default())
    }
}

async fn lookup_latest(ctx: &ScanCtx, eco: Ecosystem, name: &str) -> Lookup {
    let key = (eco, name.to_string());
    if let Some(v) = ctx.cached(&key) {
        return Lookup::Found(v);
    }
    let gate = {
        let mut map = ctx.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(key.clone()).or_default().clone()
    };
    let _gate = gate.lock().await;
    // Another task may have fetched it while this one waited.
    if let Some(v) = ctx.cached(&key) {
        return Lookup::Found(v);
    }
    let _permit = ctx.limiter.acquire().await.expect("limiter closed");
    let (c, ep) = (&ctx.client, &ctx.ep);
    let result = match eco {
        Ecosystem::Rust => registry::latest_crates_io(c, ep, name).await,
        Ecosystem::Npm => registry::latest_npm(c, ep, name).await,
        Ecosystem::Python => registry::latest_pypi(c, ep, name).await,
        Ecosystem::Go => registry::latest_go(c, ep, name).await,
        Ecosystem::Php => registry::latest_packagist(c, ep, name).await,
        Ecosystem::Ruby => registry::latest_rubygems(c, ep, name).await,
        Ecosystem::Java => registry::latest_maven(c, ep, name).await,
    };
    if let Lookup::Found(v) = &result {
        ctx.store(key, v.clone());
    }
    result
}

/// Name under which a lockfile stores `name`.
fn canonical_name(eco: Ecosystem, name: &str) -> String {
    match eco {
        Ecosystem::Python => python_eco::normalize_name(name),
        _ => name.to_string(),
    }
}

/// Normalises a '/'-separated relative path, resolving `.` and `..`. `None`
/// if it would climb out of the scan root.
fn normalize_rel(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = vec![];
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// `dir` and each of its ancestors, nearest first, ending with `""`.
fn ancestor_dirs(dir: &str) -> Vec<String> {
    let mut out = vec![];
    let mut current = dir.to_string();
    loop {
        out.push(current.clone());
        if current.is_empty() {
            break;
        }
        current = dirname(&current).to_string();
    }
    out
}

fn warning_from(path: &str, err: AppError) -> Warning {
    Warning {
        code: err.code,
        path: Some(path.to_string()),
        detail: err.detail,
    }
}

fn dedupe(deps: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    deps.into_iter()
        .filter(|(name, _)| seen.insert(name.clone()))
        .collect()
}

// ------------------------------------------------------------- parsing

async fn cargo_workspace_deps<S: FileSource>(
    src: &S,
    paths: &HashSet<String>,
    manifest_path: &str,
) -> Option<rust_eco::WorkspaceDeps> {
    for dir in ancestor_dirs(dirname(manifest_path)).into_iter().skip(1) {
        let candidate = join_path(&dir, "Cargo.toml");
        if !paths.contains(&candidate) {
            continue;
        }
        if let Ok(Some(content)) = src.read(&candidate).await {
            let ws = rust_eco::parse_workspace_deps(&content);
            if !ws.is_empty() {
                return Some(ws);
            }
        }
    }
    None
}

/// Properties and managed versions a pom inherits from its `<parent>` chain
/// and from imported BOMs.
async fn pom_inherited_context<S: FileSource>(
    ctx: &ScanCtx,
    src: &S,
    paths: &HashSet<String>,
    pom_path: &str,
    content: &str,
) -> java_eco::PomContext {
    // Ancestors, nearest first, as (path, content).
    let mut chain: Vec<String> = vec![];
    let mut current_path = pom_path.to_string();
    let mut current = content.to_string();
    for _ in 0..MAX_POM_ANCESTORS {
        let Some(parent) = java_eco::pom_parent(&current) else {
            break;
        };
        let rel = join_path(dirname(&current_path), &parent.relative_path);
        let rel = if rel.ends_with(".xml") {
            rel
        } else {
            join_path(&rel, "pom.xml")
        };
        let Some(parent_path) = normalize_rel(&rel).filter(|p| paths.contains(p)) else {
            break;
        };
        let Ok(Some(parent_content)) = src.read(&parent_path).await else {
            break;
        };
        chain.push(parent_content.clone());
        current_path = parent_path;
        current = parent_content;
    }

    let mut inherited = java_eco::PomContext::default();
    for ancestor in chain.iter().rev() {
        inherited = java_eco::pom_context(ancestor, &inherited);
    }

    // Imported BOMs (own pom and ancestors), lowest priority.
    let own = java_eco::pom_context(content, &inherited);
    let mut boms = java_eco::pom_bom_imports(content, &own);
    for ancestor in &chain {
        boms.extend(java_eco::pom_bom_imports(ancestor, &own));
    }
    let mut seen = HashSet::new();
    for (g, a, v) in boms {
        if !seen.insert((g.clone(), a.clone(), v.clone())) {
            continue;
        }
        if let Some(bom) = registry::fetch_maven_pom(&ctx.client, &ctx.ep, &g, &a, &v).await {
            let bom_ctx = java_eco::pom_context(&bom, &java_eco::PomContext::default());
            for (key, version) in bom_ctx.managed {
                inherited.managed.entry(key).or_insert(version);
            }
        }
    }
    inherited
}

async fn parse_manifest<S: FileSource>(
    ctx: &ScanCtx,
    src: &S,
    paths: &HashSet<String>,
    manifest_path: &str,
    content: &str,
) -> Vec<(String, String)> {
    let deps = match basename(manifest_path) {
        "Cargo.toml" => {
            let ws = if rust_eco::uses_workspace_deps(content) {
                cargo_workspace_deps(src, paths, manifest_path).await
            } else {
                None
            };
            rust_eco::parse_cargo_toml(content, ws.as_ref())
        }
        "package.json" => npm_eco::parse_package_json(content),
        "requirements.txt" => python_eco::parse_requirements_txt(content),
        "pyproject.toml" => python_eco::parse_pyproject_toml(content),
        "go.mod" => go_eco::parse_go_mod(content),
        "composer.json" => php_eco::parse_composer_json(content),
        "Gemfile" => ruby_eco::parse_gemfile(content),
        "pom.xml" => {
            let inherited = pom_inherited_context(ctx, src, paths, manifest_path, content).await;
            java_eco::parse_pom_xml(content, &inherited)
        }
        "build.gradle" | "build.gradle.kts" => java_eco::parse_gradle(content),
        "libs.versions.toml" => java_eco::parse_version_catalog(content),
        _ => vec![],
    };
    dedupe(deps)
}

// ----------------------------------------------------------- lockfiles

/// Whether the lockfile found in an ancestor folder really covers a project
/// nested below it (a workspace), rather than being an unrelated project's.
async fn ancestor_lock_applies<S: FileSource>(
    src: &S,
    paths: &HashSet<String>,
    lock_name: &str,
    lock_dir: &str,
) -> bool {
    match lock_name {
        "Cargo.lock" => match src.read(&join_path(lock_dir, "Cargo.toml")).await {
            Ok(Some(c)) => c.contains("[workspace]"),
            _ => false,
        },
        "package-lock.json" | "yarn.lock" | "bun.lock" => {
            match src.read(&join_path(lock_dir, "package.json")).await {
                Ok(Some(c)) => c.contains("\"workspaces\""),
                _ => false,
            }
        }
        "pnpm-lock.yaml" => paths.contains(&join_path(lock_dir, "pnpm-workspace.yaml")),
        "uv.lock" => match src.read(&join_path(lock_dir, "pyproject.toml")).await {
            Ok(Some(c)) => c.contains("[tool.uv.workspace]"),
            _ => false,
        },
        _ => false,
    }
}

/// The lockfile covering `manifest_path`: one next to it, or (for
/// workspaces) in an ancestor folder.
async fn find_lockfile<S: FileSource>(
    src: &S,
    paths: &HashSet<String>,
    manifest_path: &str,
) -> Option<String> {
    let candidates = manifest_lockfiles(basename(manifest_path));
    for (depth, dir) in ancestor_dirs(dirname(manifest_path)).iter().enumerate() {
        for name in candidates {
            let path = join_path(dir, name);
            if !paths.contains(&path) {
                continue;
            }
            if depth == 0 || ancestor_lock_applies(src, paths, name, dir).await {
                return Some(path);
            }
        }
    }
    None
}

/// Reads the lockfile and returns, per dependency (keyed by
/// [`canonical_name`]), the version it pins.
fn installed_versions(
    eco: Ecosystem,
    lock_path: &str,
    content: &str,
    manifest_dir: &str,
    deps: &[(String, String)],
    warnings: &mut Vec<Warning>,
) -> HashMap<String, String> {
    let mut installed = HashMap::new();
    if content.trim().is_empty() {
        warnings.push(Warning::at("lockfile_empty", lock_path));
        return installed;
    }
    let lock_dir = dirname(lock_path);
    let member_dir = manifest_dir
        .strip_prefix(lock_dir)
        .map(|r| r.trim_start_matches('/'))
        .unwrap_or("");

    match basename(lock_path) {
        "Cargo.lock" => {
            let versions = rust_eco::parse_cargo_lock(content);
            for (name, declared) in deps {
                if let Some(candidates) = versions.get(name) {
                    if let Some(v) = rust_eco::resolve_installed_version(candidates, declared) {
                        installed.insert(name.clone(), v.to_string());
                    }
                }
            }
        }
        "package-lock.json" => match npm_eco::parse_package_lock(content) {
            Some(npm_eco::PackageLock::Packages(packages)) => {
                for (name, _) in deps {
                    if let Some(v) = npm_eco::installed_from_lock(&packages, member_dir, name) {
                        installed.insert(name.clone(), v);
                    }
                }
            }
            Some(npm_eco::PackageLock::UnsupportedV1) => {
                warnings.push(Warning::at("lockfile_v1_unsupported", lock_path));
            }
            None => warnings.push(Warning::at("lockfile_unparseable", lock_path)),
        },
        "yarn.lock" => {
            let map = npm_eco::parse_yarn_lock(content);
            for (name, declared) in deps {
                let hit = map
                    .get(&format!("{name}@{declared}"))
                    .or_else(|| map.get(name));
                if let Some(v) = hit {
                    installed.insert(name.clone(), v.clone());
                }
            }
        }
        "bun.lock" => {
            let map = npm_eco::parse_bun_lock(content);
            for (name, _) in deps {
                if let Some(v) = map.get(name) {
                    installed.insert(name.clone(), v.clone());
                }
            }
        }
        "pnpm-lock.yaml" => {
            let map = npm_eco::parse_pnpm_lock(content);
            for (name, _) in deps {
                if let Some(v) = map.get(name) {
                    installed.insert(name.clone(), v.clone());
                }
            }
        }
        "poetry.lock" | "uv.lock" | "pdm.lock" => {
            let map = python_eco::parse_poetry_lock(content);
            for (name, _) in deps {
                if let Some(v) = map.get(&canonical_name(eco, name)) {
                    installed.insert(canonical_name(eco, name), v.clone());
                }
            }
        }
        "composer.lock" => installed = php_eco::parse_composer_lock(content),
        "Gemfile.lock" => installed = ruby_eco::parse_gemfile_lock(content),
        _ => {}
    }
    installed
}

// ---------------------------------------------------------- resolution

/// The version of `declared`/`installed` to submit to OSV, and where it
/// comes from. A lockfile entry always wins; without one, only an *exact*
/// pin in the manifest (Go, Maven, `==x.y.z`) is a version that's really in
/// use.
fn osv_version(
    eco: Ecosystem,
    declared: &str,
    installed: Option<&str>,
) -> Option<(String, &'static str)> {
    if let Some(i) = installed {
        return Some((i.to_string(), "installed"));
    }
    let d = declared.trim();
    let starts_with_digit = |s: &str| s.starts_with(|c: char| c.is_ascii_digit());
    let pinned = match eco {
        Ecosystem::Go => d.strip_prefix('v').is_some_and(starts_with_digit),
        Ecosystem::Java => {
            starts_with_digit(d)
                && d.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        }
        Ecosystem::Python => {
            let exact = d.strip_prefix("===").or_else(|| d.strip_prefix("=="));
            return exact
                .map(str::trim)
                .filter(|v| starts_with_digit(v) && !v.contains(['*', ',', ' ']))
                .map(|v| (v.to_string(), "declared"));
        }
        _ => false,
    };
    pinned.then(|| (d.to_string(), "declared"))
}

async fn resolve_dep(
    ctx: &ScanCtx,
    eco: Ecosystem,
    name: String,
    declared: String,
    installed: Option<String>,
) -> DependencyInfo {
    // An npm "alias" ("npm:<real-name>@<range>") installs a *different*
    // package under this name — the alias key itself was never published, so
    // both the registry lookup and the range comparison target the real one.
    let alias = (eco == Ecosystem::Npm)
        .then(|| npm_eco::parse_npm_alias(&declared))
        .flatten();
    let (lookup_name, effective): (String, String) = match alias {
        Some((real, range)) => (real.to_string(), range.to_string()),
        None => (name.clone(), declared.clone()),
    };
    let package = alias.map(|_| lookup_name.clone());

    let mut info = DependencyInfo {
        name,
        package,
        declared,
        installed,
        latest: None,
        status: DepStatus::Unknown,
        reason: None,
        vulnerabilities: vec![],
        vuln_basis: None,
    };

    // Never ask a public registry about a local path / git URL / workspace
    // member: nothing to find, and it would leak a private package name.
    if version::is_non_registry(&effective) {
        info.reason = Some(reason::NON_REGISTRY.into());
        return info;
    }

    match lookup_latest(ctx, eco, &lookup_name).await {
        Lookup::Found(latest) => {
            info.status = version::classify(eco, &effective, info.installed.as_deref(), &latest);
            if info.status == DepStatus::Unknown {
                info.reason = Some(reason::UNPARSEABLE.into());
            }
            info.latest = Some(latest);
        }
        Lookup::NotFound => info.reason = Some(reason::NOT_FOUND.into()),
        Lookup::Failed => info.reason = Some(reason::NETWORK_ERROR.into()),
    }
    info
}

struct ManifestOutcome {
    scan: Option<ManifestScan>,
    warnings: Vec<Warning>,
}

async fn process_manifest<S: FileSource>(
    ctx: &ScanCtx,
    src: &S,
    paths: &HashSet<String>,
    manifest_path: &str,
) -> Option<ManifestOutcome> {
    let filename = basename(manifest_path);
    let eco = ecosystem_for(filename)?;
    let mut warnings = vec![];

    let content = match src.read(manifest_path).await {
        Ok(Some(c)) => c,
        Ok(None) => return None,
        Err(e) => {
            // No card to show for an unreadable manifest, only a warning.
            return Some(ManifestOutcome {
                scan: None,
                warnings: vec![warning_from(manifest_path, e)],
            });
        }
    };

    let deps = parse_manifest(ctx, src, paths, manifest_path, &content).await;
    if deps.is_empty() {
        return None;
    }

    let mut lockfile_path = None;
    let mut installed = HashMap::new();
    if let Some(lock_path) = find_lockfile(src, paths, manifest_path).await {
        match src.read(&lock_path).await {
            Ok(Some(lock_content)) => {
                installed = installed_versions(
                    eco,
                    &lock_path,
                    &lock_content,
                    dirname(manifest_path),
                    &deps,
                    &mut warnings,
                );
                lockfile_path = Some(lock_path);
            }
            Ok(None) => {}
            Err(e) => warnings.push(warning_from(&lock_path, e)),
        }
    }

    // The shared limiter bounds the actual registry traffic, so every
    // lookup can be started at once.
    let mut pending = Vec::with_capacity(deps.len());
    for (name, declared) in deps {
        let inst = installed.get(&canonical_name(eco, &name)).cloned();
        pending.push(resolve_dep(ctx, eco, name, declared, inst));
    }
    let mut dependencies: Vec<DependencyInfo> = join_all(pending).await;
    dependencies.sort_by(|a, b| a.name.cmp(&b.name));

    let unreachable = dependencies
        .iter()
        .filter(|d| d.reason.as_deref() == Some(reason::NETWORK_ERROR))
        .count();
    if unreachable > 0 {
        warnings.push(
            Warning::at("registry_unreachable", manifest_path).with_detail(unreachable.to_string()),
        );
    }

    Some(ManifestOutcome {
        scan: Some(ManifestScan {
            ecosystem: eco.as_str().to_string(),
            manifest_path: manifest_path.to_string(),
            lockfile_path,
            dependencies,
        }),
        warnings,
    })
}

/// Checks every dependency of every manifest against OSV in as few requests
/// as possible, and records which version each check used.
async fn attach_vulnerabilities(
    ctx: &ScanCtx,
    manifests: &mut [ManifestScan],
    warnings: &mut Vec<Warning>,
) {
    let mut queries: Vec<VulnQuery> = vec![];
    let mut index: HashMap<VulnQuery, usize> = HashMap::new();
    // (manifest idx, dep idx, query idx, basis)
    let mut targets: Vec<(usize, usize, usize, &'static str)> = vec![];

    for (mi, manifest) in manifests.iter().enumerate() {
        let eco = ecosystem_for(basename(&manifest.manifest_path)).unwrap_or(Ecosystem::Npm);
        for (di, dep) in manifest.dependencies.iter().enumerate() {
            if dep.reason.as_deref() == Some(reason::NON_REGISTRY) {
                continue;
            }
            let effective = match (eco, npm_eco::parse_npm_alias(&dep.declared)) {
                (Ecosystem::Npm, Some((_, range))) => range,
                _ => dep.declared.as_str(),
            };
            let Some((version, basis)) = osv_version(eco, effective, dep.installed.as_deref())
            else {
                continue;
            };
            let query = VulnQuery {
                ecosystem: eco,
                name: dep.package.clone().unwrap_or_else(|| dep.name.clone()),
                version,
            };
            let qi = *index.entry(query.clone()).or_insert_with(|| {
                queries.push(query);
                queries.len() - 1
            });
            targets.push((mi, di, qi, basis));
        }
    }
    if queries.is_empty() {
        return;
    }

    let answers = osv::check_vulnerabilities(&ctx.client, &ctx.ep, &queries).await;
    let failed = answers.iter().filter(|a| a.is_none()).count();
    if failed > 0 {
        warnings.push(Warning::new("osv_unavailable").with_detail(failed.to_string()));
    }
    for (mi, di, qi, basis) in targets {
        if let Some(ids) = &answers[qi] {
            let dep = &mut manifests[mi].dependencies[di];
            dep.vulnerabilities = ids.clone();
            dep.vuln_basis = Some(basis.to_string());
        }
    }
}

/// Runs the whole pipeline over the manifests listed in `paths`.
pub async fn scan_source<S: FileSource>(
    ctx: &ScanCtx,
    src: &S,
    paths: &HashSet<String>,
    source_name: String,
    mut warnings: Vec<Warning>,
) -> ScanReport {
    let mut manifest_paths: Vec<&String> = paths
        .iter()
        .filter(|p| MANIFEST_FILES.contains(&basename(p)) && within_depth(p))
        .collect();
    manifest_paths.sort();

    let mut outcomes: Vec<ManifestOutcome> = vec![];
    for chunk in manifest_paths.chunks(MANIFEST_CONCURRENCY) {
        let mut pending = Vec::with_capacity(chunk.len());
        for path in chunk {
            pending.push(process_manifest(ctx, src, paths, path));
        }
        outcomes.extend(join_all(pending).await.into_iter().flatten());
    }

    let mut manifests = vec![];
    for outcome in outcomes {
        warnings.extend(outcome.warnings);
        manifests.extend(outcome.scan);
    }
    manifests.sort_by(|a, b| a.manifest_path.cmp(&b.manifest_path));

    attach_vulnerabilities(ctx, &mut manifests, &mut warnings).await;

    ScanReport {
        source: source_name,
        manifests,
        warnings,
    }
}

// ------------------------------------------------------------ entry points

fn is_scannable(path: &str) -> bool {
    let name = basename(path);
    (MANIFEST_FILES.contains(&name) || LOCKFILE_FILES.contains(&name)) && within_depth(path)
}

pub async fn scan_local_path(ctx: &ScanCtx, path: &str) -> Result<ScanReport, AppError> {
    let root = PathBuf::from(path);
    if !root.is_dir() {
        return Err(AppError::with_detail("local_dir_not_found", path));
    }

    let walk_root = root.clone();
    let files = tokio::task::spawn_blocking(move || local::find_files(&walk_root))
        .await
        .map_err(|e| AppError::with_detail("read_failed", e.to_string()))?;
    let paths: HashSet<String> = files.into_iter().collect();
    if !paths.iter().any(|p| MANIFEST_FILES.contains(&basename(p))) {
        return Err(AppError::new("no_manifests_found"));
    }

    let src = LocalSource { root };
    Ok(scan_source(ctx, &src, &paths, path.to_string(), vec![]).await)
}

pub async fn scan_github_repo(
    ctx: &ScanCtx,
    repo: &str,
    token: Option<&str>,
) -> Result<ScanReport, AppError> {
    let (owner, name) = github::parse_repo(repo)
        .ok_or_else(|| AppError::with_detail("invalid_repo_format", repo))?;

    let branch = github::default_branch(&ctx.client, &ctx.ep, &owner, &name, token).await?;
    let listing = github::list_tree(&ctx.client, &ctx.ep, &owner, &name, &branch, token).await?;

    let paths: HashSet<String> = listing
        .paths
        .into_iter()
        .filter(|p| is_scannable(p))
        .collect();
    if !paths.iter().any(|p| MANIFEST_FILES.contains(&basename(p))) {
        return Err(AppError::new(if listing.truncated {
            "github_tree_truncated"
        } else {
            "no_manifests_found_github"
        }));
    }

    let mut warnings = vec![];
    if listing.truncated {
        warnings.push(Warning::new("tree_truncated"));
    }

    let src = GithubSource {
        client: &ctx.client,
        ep: &ctx.ep,
        owner: &owner,
        repo: &name,
        branch: &branch,
        token,
    };
    Ok(scan_source(ctx, &src, &paths, format!("{owner}/{name}"), warnings).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    // ----------------------------------------------------------- helpers

    async fn server() -> (MockServer, ScanCtx) {
        let server = MockServer::start().await;
        let ctx = ScanCtx::new(Endpoints::all_at(&server.uri()));
        (server, ctx)
    }

    async fn mount_get(server: &MockServer, p: &str, status: u16, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(server)
            .await;
    }

    async fn mount_text(server: &MockServer, p: &str, body: &str) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_string(body.to_string()))
            .mount(server)
            .await;
    }

    async fn npm_latest(server: &MockServer, name: &str, version: &str) {
        mount_get(
            server,
            &format!("/npm/{name}/latest"),
            200,
            serde_json::json!({ "version": version }),
        )
        .await;
    }

    /// OSV answers: for each query, the ids registered for `"<name>@<version>"`.
    async fn mount_osv(server: &MockServer, known: Vec<(&'static str, Vec<&'static str>)>) {
        Mock::given(method("POST"))
            .and(path("/osv/v1/querybatch"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let results: Vec<serde_json::Value> = body["queries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|q| {
                        let key = format!(
                            "{}@{}",
                            q["package"]["name"].as_str().unwrap(),
                            q["version"].as_str().unwrap()
                        );
                        match known.iter().find(|(k, _)| *k == key) {
                            Some((_, ids)) => serde_json::json!({
                                "vulns": ids.iter().map(|id| serde_json::json!({"id": id})).collect::<Vec<_>>()
                            }),
                            None => serde_json::json!({}),
                        }
                    })
                    .collect();
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "results": results }))
            })
            .mount(server)
            .await;
    }

    fn write(dir: &std::path::Path, rel: &str, content: &str) {
        let p = rel.split('/').fold(dir.to_path_buf(), |a, s| a.join(s));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    async fn scan_dir(ctx: &ScanCtx, dir: &std::path::Path) -> ScanReport {
        scan_local_path(ctx, &dir.to_string_lossy()).await.unwrap()
    }

    fn dep<'a>(report: &'a ScanReport, manifest: &str, name: &str) -> &'a DependencyInfo {
        report
            .manifests
            .iter()
            .find(|m| m.manifest_path == manifest)
            .unwrap_or_else(|| panic!("no manifest {manifest}"))
            .dependencies
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("no dep {name}"))
    }

    fn codes(report: &ScanReport) -> Vec<String> {
        report.warnings.iter().map(|w| w.code.clone()).collect()
    }

    // ------------------------------------------------------- pure helpers

    #[test]
    fn osv_version_prefers_the_lockfile_and_accepts_exact_pins() {
        use Ecosystem::*;
        assert_eq!(
            osv_version(Npm, "^1.0.0", Some("1.2.3")),
            Some(("1.2.3".into(), "installed"))
        );
        assert_eq!(osv_version(Npm, "1.2.3", None), None);
        assert_eq!(
            osv_version(Go, "v1.2.3", None),
            Some(("v1.2.3".into(), "declared"))
        );
        assert_eq!(osv_version(Go, "latest", None), None);
        assert_eq!(
            osv_version(Java, "2.14.1", None),
            Some(("2.14.1".into(), "declared"))
        );
        assert_eq!(osv_version(Java, "[1.0,2.0)", None), None);
        assert_eq!(osv_version(Java, "${v}", None), None);
        assert_eq!(
            osv_version(Python, "==2.28.1", None),
            Some(("2.28.1".into(), "declared"))
        );
        assert_eq!(
            osv_version(Python, "===2.28.1", None),
            Some(("2.28.1".into(), "declared"))
        );
        assert_eq!(osv_version(Python, "==2.*", None), None);
        assert_eq!(osv_version(Python, "==1.0,<2", None), None);
        assert_eq!(osv_version(Python, ">=2.0", None), None);
    }

    #[test]
    fn path_utilities() {
        assert_eq!(normalize_rel("a/b/../c/./d"), Some("a/c/d".into()));
        assert_eq!(normalize_rel("../x"), None);
        assert_eq!(normalize_rel(""), Some(String::new()));
        assert_eq!(
            ancestor_dirs("a/b"),
            vec!["a/b".to_string(), "a".to_string(), String::new()]
        );
        assert_eq!(ancestor_dirs(""), vec![String::new()]);
        assert_eq!(
            canonical_name(Ecosystem::Python, "Typing_Extensions"),
            "typing-extensions"
        );
        assert_eq!(canonical_name(Ecosystem::Npm, "Foo"), "Foo");
        assert!(is_scannable("a/package.json"));
        assert!(is_scannable("yarn.lock"));
        assert!(!is_scannable("README.md"));
        assert!(!is_scannable("a/b/c/d/e/f/g/h/package.json"));
        assert_eq!(
            dedupe(vec![("a".into(), "1".into()), ("a".into(), "2".into())]).len(),
            1
        );
    }

    #[tokio::test]
    async fn the_cache_ignores_expired_entries() {
        let ctx = ScanCtx::default();
        let key = (Ecosystem::Npm, "x".to_string());
        assert_eq!(ctx.cached(&key), None);
        ctx.store(key.clone(), "1.0.0".into());
        assert_eq!(ctx.cached(&key), Some("1.0.0".into()));
        ctx.cache.lock().unwrap().insert(
            key.clone(),
            (
                Instant::now() - CACHE_TTL - Duration::from_secs(1),
                "old".into(),
            ),
        );
        assert_eq!(ctx.cached(&key), None);
    }

    // ------------------------------------------- real-world manifests

    fn sorted(mut deps: Vec<(String, String)>) -> Vec<(String, String)> {
        deps.sort();
        deps
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn a_laravel_style_composer_json_is_understood() {
        use crate::version::{classify, normalize_composer};
        let deps =
            php_eco::parse_composer_json(include_str!("../tests/fixtures/laravel-composer.json"));
        let by_name: HashMap<_, _> = deps.iter().cloned().collect();
        assert_eq!(by_name.len(), 6, "php and ext-json are platform packages");
        assert_eq!(
            normalize_composer(&by_name["laravel/framework"]),
            "^11.0 || ^12.0"
        );
        assert_eq!(
            classify(
                Ecosystem::Php,
                &by_name["laravel/framework"],
                None,
                "12.4.0"
            ),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Php, &by_name["guzzlehttp/guzzle"], None, "7.9.2"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Php, &by_name["mockery/mockery"], None, "1.7.0"),
            DepStatus::Outdated
        );
        assert_eq!(
            classify(Ecosystem::Php, &by_name["symfony/console"], None, "7.2.0"),
            DepStatus::UpToDate
        );
        assert!(version::is_non_registry(&by_name["acme/internal"]));
    }

    #[test]
    fn a_poetry_pyproject_is_understood() {
        use crate::version::classify;
        let deps = python_eco::parse_pyproject_toml(include_str!(
            "../tests/fixtures/poetry-pyproject.toml"
        ));
        let by_name: HashMap<_, _> = deps.iter().cloned().collect();
        assert!(!by_name.contains_key("python"));
        assert_eq!(
            classify(Ecosystem::Python, &by_name["requests"], None, "2.32.3"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Python, &by_name["Django"], None, "5.1.0"),
            DepStatus::Major
        );
        assert_eq!(
            classify(
                Ecosystem::Python,
                &by_name["typing_extensions"],
                None,
                "4.9.4"
            ),
            DepStatus::UpToDate
        );
        assert_eq!(
            // ^0.110 means <0.111: a 0.x minor bump is a breaking change.
            classify(Ecosystem::Python, &by_name["fastapi"], None, "0.111.0"),
            DepStatus::Major
        );
        assert!(version::is_non_registry(&by_name["internal"]));
        assert!(by_name.contains_key("pytest"));
    }

    #[test]
    fn a_pip_compile_requirements_file_is_understood() {
        let deps = python_eco::parse_requirements_txt(include_str!(
            "../tests/fixtures/pip-compile-requirements.txt"
        ));
        assert_eq!(
            sorted(deps),
            vec![
                pair("certifi", "==2024.2.2"),
                pair("requests", "==2.31.0"),
                pair("urllib3", ">=1.21.1,<3"),
            ]
        );
    }

    #[test]
    fn a_cargo_workspace_is_understood() {
        let ws = rust_eco::parse_workspace_deps(include_str!(
            "../tests/fixtures/cargo-workspace-root.toml"
        ));
        assert_eq!(ws.len(), 3, "the path dependency has no registry version");
        let member = include_str!("../tests/fixtures/cargo-workspace-member.toml");
        assert!(rust_eco::uses_workspace_deps(member));
        assert_eq!(
            sorted(rust_eco::parse_cargo_toml(member, Some(&ws))),
            vec![
                pair("anyhow", "1"),
                pair("regex", "1.10"),
                pair("serde", "1.0"),
                pair("tempfile", "3"),
                pair("tokio", "1.38"),
                pair("winapi", "0.3"),
            ]
        );
    }

    // ------------------------------------------------------- local scans

    #[tokio::test]
    async fn local_scan_resolves_versions_vulnerabilities_and_non_registry_deps() {
        let (server, ctx) = server().await;
        npm_latest(&server, "lodash", "4.17.21").await;
        npm_latest(&server, "react", "18.3.0").await;
        mount_get(&server, "/npm/ghost/latest", 404, serde_json::json!({})).await;
        mount_osv(&server, vec![("lodash@4.17.19", vec!["GHSA-1", "CVE-2"])]).await;

        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            r#"{"dependencies": {"lodash": "^4.17.0", "react": "^18.0.0", "ghost": "^1.0.0",
                 "internal": "workspace:*", "aliased": "npm:lodash@^4.0.0"},
                "devDependencies": {"react": "^18.0.0"}}"#,
        );
        write(
            dir.path(),
            "package-lock.json",
            r#"{"lockfileVersion": 3, "packages": {
                "node_modules/lodash": {"version": "4.17.19"},
                "node_modules/aliased": {"version": "4.17.19"},
                "node_modules/react": {"version": "18.3.0"}}}"#,
        );

        let report = scan_dir(&ctx, dir.path()).await;
        assert_eq!(report.manifests.len(), 1);
        let m = &report.manifests[0];
        assert_eq!(m.ecosystem, "npm");
        assert_eq!(m.lockfile_path.as_deref(), Some("package-lock.json"));
        // Sorted by name, `react` listed once despite dep + devDep.
        let names: Vec<_> = m.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["aliased", "ghost", "internal", "lodash", "react"]);

        let lodash = dep(&report, "package.json", "lodash");
        assert_eq!(lodash.installed.as_deref(), Some("4.17.19"));
        assert_eq!(lodash.latest.as_deref(), Some("4.17.21"));
        assert_eq!(lodash.status, DepStatus::Outdated);
        assert_eq!(lodash.vulnerabilities, ["GHSA-1", "CVE-2"]);
        assert_eq!(lodash.vuln_basis.as_deref(), Some("installed"));

        assert_eq!(
            dep(&report, "package.json", "react").status,
            DepStatus::UpToDate
        );

        let aliased = dep(&report, "package.json", "aliased");
        assert_eq!(aliased.package.as_deref(), Some("lodash"));
        assert_eq!(aliased.status, DepStatus::Outdated);
        // The alias is checked under the REAL package name.
        assert_eq!(aliased.vulnerabilities, ["GHSA-1", "CVE-2"]);

        let ghost = dep(&report, "package.json", "ghost");
        assert_eq!(ghost.status, DepStatus::Unknown);
        assert_eq!(ghost.reason.as_deref(), Some(reason::NOT_FOUND));

        let internal = dep(&report, "package.json", "internal");
        assert_eq!(internal.status, DepStatus::Unknown);
        assert_eq!(internal.reason.as_deref(), Some(reason::NON_REGISTRY));
        assert_eq!(internal.vuln_basis, None);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        // The private/workspace name never reached the registry.
        let hit_internal = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().contains("internal"));
        assert!(!hit_internal);
    }

    #[tokio::test]
    async fn unreachable_registries_and_osv_are_reported_not_hidden() {
        let (server, ctx) = server().await;
        Mock::given(method("GET"))
            .and(path("/npm/flaky/latest"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/osv/v1/querybatch"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            r#"{"dependencies": {"flaky": "^1.0.0"}}"#,
        );
        write(
            dir.path(),
            "package-lock.json",
            r#"{"lockfileVersion": 3, "packages": {"node_modules/flaky": {"version": "1.0.0"}}}"#,
        );
        let report = scan_dir(&ctx, dir.path()).await;
        let flaky = dep(&report, "package.json", "flaky");
        assert_eq!(flaky.reason.as_deref(), Some(reason::NETWORK_ERROR));
        assert!(flaky.vulnerabilities.is_empty());
        assert_eq!(flaky.vuln_basis, None);
        let c = codes(&report);
        assert!(c.contains(&"registry_unreachable".to_string()), "{c:?}");
        assert!(c.contains(&"osv_unavailable".to_string()), "{c:?}");
    }

    #[tokio::test]
    async fn exact_pins_in_go_maven_and_requirements_are_checked_against_osv() {
        let (server, ctx) = server().await;
        mount_get(
            &server,
            "/go/golang.org/x/net/@latest",
            200,
            serde_json::json!({"Version": "v0.30.0"}),
        )
        .await;
        mount_text(
            &server,
            "/maven/org/apache/logging/log4j/log4j-core/maven-metadata.xml",
            "<metadata><versioning><versions><version>2.14.1</version><version>2.24.0</version></versions></versioning></metadata>",
        )
        .await;
        mount_get(
            &server,
            "/pypi/pypi/django/json",
            200,
            serde_json::json!({"info": {"version": "5.1.0"}}),
        )
        .await;
        mount_osv(
            &server,
            vec![
                ("golang.org/x/net@v0.10.0", vec!["GO-1"]),
                (
                    "org.apache.logging.log4j:log4j-core@2.14.1",
                    vec!["GHSA-jfh8"],
                ),
                ("django@3.2.0", vec!["PYSEC-1"]),
            ],
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "go.mod",
            "module m\n\nrequire golang.org/x/net v0.10.0\n",
        );
        write(
            dir.path(),
            "pom.xml",
            "<project><dependencies><dependency><groupId>org.apache.logging.log4j</groupId>\
             <artifactId>log4j-core</artifactId><version>2.14.1</version></dependency></dependencies></project>",
        );
        write(
            dir.path(),
            "requirements.txt",
            "django==3.2.0\nrequests>=2\n",
        );

        let report = scan_dir(&ctx, dir.path()).await;
        let go = dep(&report, "go.mod", "golang.org/x/net");
        assert_eq!(go.vulnerabilities, ["GO-1"]);
        assert_eq!(go.vuln_basis.as_deref(), Some("declared"));
        assert_eq!(go.status, DepStatus::Major);
        let java = dep(&report, "pom.xml", "org.apache.logging.log4j:log4j-core");
        assert_eq!(java.vulnerabilities, ["GHSA-jfh8"]);
        assert_eq!(java.latest.as_deref(), Some("2.24.0"));
        let py = dep(&report, "requirements.txt", "django");
        assert_eq!(py.vulnerabilities, ["PYSEC-1"]);
        // `>=2` is a range, not a version in use: not submitted to OSV.
        assert_eq!(
            dep(&report, "requirements.txt", "requests").vuln_basis,
            None
        );
    }

    #[tokio::test]
    async fn latest_versions_are_cached_across_manifests_and_scans() {
        let (server, ctx) = server().await;
        Mock::given(method("GET"))
            .and(path("/npm/shared/latest"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"version": "2.0.0"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a/package.json",
            r#"{"dependencies": {"shared": "^1.0.0"}}"#,
        );
        write(
            dir.path(),
            "b/package.json",
            r#"{"dependencies": {"shared": "^2.0.0"}}"#,
        );
        let first = scan_dir(&ctx, dir.path()).await;
        let second = scan_dir(&ctx, dir.path()).await;
        assert_eq!(
            dep(&first, "a/package.json", "shared").status,
            DepStatus::Major
        );
        assert_eq!(
            dep(&second, "b/package.json", "shared").status,
            DepStatus::UpToDate
        );
        // `expect(1)` is verified when the mock server is dropped.
    }

    #[tokio::test]
    async fn cargo_workspace_members_inherit_versions_and_use_the_root_lockfile() {
        let (server, ctx) = server().await;
        mount_text(
            &server,
            "/crates/to/ki/tokio",
            "{\"vers\":\"1.40.0\",\"yanked\":false}\n",
        )
        .await;
        mount_text(
            &server,
            "/crates/se/rd/serde",
            "{\"vers\":\"1.0.210\",\"yanked\":false}\n",
        )
        .await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/a\"]\n[workspace.dependencies]\ntokio = \"1\"\nserde = \"1.0\"\n",
        );
        write(
            dir.path(),
            "Cargo.lock",
            "version = 3\n[[package]]\nname = \"tokio\"\nversion = \"1.2.0\"\n[[package]]\nname = \"serde\"\nversion = \"1.0.210\"\n",
        );
        write(
            dir.path(),
            "crates/a/Cargo.toml",
            "[package]\nname = \"a\"\n[dependencies]\ntokio = { workspace = true }\n",
        );
        let report = scan_dir(&ctx, dir.path()).await;
        let member = report
            .manifests
            .iter()
            .find(|m| m.manifest_path == "crates/a/Cargo.toml")
            .unwrap();
        assert_eq!(member.lockfile_path.as_deref(), Some("Cargo.lock"));
        let tokio = &member.dependencies[0];
        assert_eq!(tokio.name, "tokio");
        assert_eq!(tokio.installed.as_deref(), Some("1.2.0"));
        assert_eq!(tokio.status, DepStatus::Outdated);
        let root = report
            .manifests
            .iter()
            .find(|m| m.manifest_path == "Cargo.toml")
            .unwrap();
        assert_eq!(root.dependencies.len(), 2);
    }

    #[tokio::test]
    async fn npm_workspace_members_use_the_root_lockfile_and_nested_copies() {
        let (server, ctx) = server().await;
        npm_latest(&server, "left-pad", "1.3.0").await;
        npm_latest(&server, "react", "18.3.0").await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            r#"{"workspaces": ["packages/*"]}"#,
        );
        write(
            dir.path(),
            "package-lock.json",
            r#"{"lockfileVersion": 3, "packages": {
                "node_modules/react": {"version": "18.3.0"},
                "packages/app/node_modules/left-pad": {"version": "1.1.0"},
                "node_modules/@app/web": {"version": "0.0.0", "link": true}}}"#,
        );
        write(
            dir.path(),
            "packages/app/package.json",
            r#"{"dependencies": {"left-pad": "^1.0.0", "react": "^18.0.0"}}"#,
        );
        let report = scan_dir(&ctx, dir.path()).await;
        assert_eq!(report.manifests.len(), 1, "root declares no deps");
        let m = &report.manifests[0];
        assert_eq!(m.lockfile_path.as_deref(), Some("package-lock.json"));
        assert_eq!(
            dep(&report, "packages/app/package.json", "left-pad")
                .installed
                .as_deref(),
            Some("1.1.0")
        );
        assert_eq!(
            dep(&report, "packages/app/package.json", "react")
                .installed
                .as_deref(),
            Some("18.3.0")
        );
    }

    #[tokio::test]
    async fn an_unrelated_ancestor_lockfile_is_not_borrowed() {
        let (server, ctx) = server().await;
        npm_latest(&server, "react", "18.3.0").await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        // The root is NOT a workspace: its lockfile belongs to the root only.
        write(
            dir.path(),
            "package.json",
            r#"{"dependencies": {"x": "1"}}"#,
        );
        write(
            dir.path(),
            "package-lock.json",
            r#"{"lockfileVersion": 3, "packages": {"node_modules/react": {"version": "17.0.0"}}}"#,
        );
        write(
            dir.path(),
            "docs/package.json",
            r#"{"dependencies": {"react": "^18.0.0"}}"#,
        );
        let report = scan_dir(&ctx, dir.path()).await;
        let docs = dep(&report, "docs/package.json", "react");
        assert_eq!(docs.installed, None);
        assert_eq!(
            report
                .manifests
                .iter()
                .find(|m| m.manifest_path == "docs/package.json")
                .unwrap()
                .lockfile_path,
            None
        );
    }

    #[tokio::test]
    async fn yarn_pnpm_poetry_composer_and_gem_lockfiles_provide_installed_versions() {
        let (server, ctx) = server().await;
        npm_latest(&server, "lodash", "4.17.21").await;
        npm_latest(&server, "react", "18.3.0").await;
        mount_get(
            &server,
            "/pypi/pypi/typing_extensions/json",
            200,
            serde_json::json!({"info": {"version": "4.12.0"}}),
        )
        .await;
        mount_get(
            &server,
            "/packagist/p2/monolog/monolog.json",
            200,
            serde_json::json!({"packages": {"monolog/monolog": [{"version": "3.7.0"}]}}),
        )
        .await;
        mount_get(
            &server,
            "/rubygems/api/v1/gems/rails.json",
            200,
            serde_json::json!({"version": "7.2.0"}),
        )
        .await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "yarn/package.json",
            r#"{"dependencies": {"lodash": "^4.17.0"}}"#,
        );
        write(
            dir.path(),
            "yarn/yarn.lock",
            "lodash@^4.17.0:\n  version \"4.17.15\"\n",
        );
        write(
            dir.path(),
            "pnpm/package.json",
            r#"{"dependencies": {"react": "^18.0.0"}}"#,
        );
        write(
            dir.path(),
            "pnpm/pnpm-lock.yaml",
            "packages:\n  react@18.2.0:\n    resolution: {}\n",
        );
        write(
            dir.path(),
            "py/pyproject.toml",
            "[tool.poetry.dependencies]\ntyping_extensions = \"^4.0\"\n",
        );
        write(
            dir.path(),
            "py/poetry.lock",
            "[[package]]\nname = \"typing-extensions\"\nversion = \"4.5.0\"\n",
        );
        write(
            dir.path(),
            "php/composer.json",
            r#"{"require": {"monolog/monolog": "~3.0"}}"#,
        );
        write(
            dir.path(),
            "php/composer.lock",
            r#"{"packages": [{"name": "monolog/monolog", "version": "3.1.0"}]}"#,
        );
        write(dir.path(), "rb/Gemfile", "gem 'rails', '~> 7.0'\n");
        write(
            dir.path(),
            "rb/Gemfile.lock",
            "GEM\n  specs:\n    rails (7.0.4)\n",
        );

        let report = scan_dir(&ctx, dir.path()).await;
        assert_eq!(
            dep(&report, "yarn/package.json", "lodash")
                .installed
                .as_deref(),
            Some("4.17.15")
        );
        assert_eq!(
            dep(&report, "pnpm/package.json", "react")
                .installed
                .as_deref(),
            Some("18.2.0")
        );
        let te = dep(&report, "py/pyproject.toml", "typing_extensions");
        assert_eq!(te.installed.as_deref(), Some("4.5.0"));
        assert_eq!(te.status, DepStatus::Outdated);
        assert_eq!(
            dep(&report, "php/composer.json", "monolog/monolog")
                .installed
                .as_deref(),
            Some("3.1.0")
        );
        assert_eq!(
            dep(&report, "rb/Gemfile", "rails").installed.as_deref(),
            Some("7.0.4")
        );
    }

    #[tokio::test]
    async fn lockfile_problems_become_warnings() {
        let (server, ctx) = server().await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "empty/package.json",
            r#"{"dependencies": {"a": "workspace:*"}}"#,
        );
        write(dir.path(), "empty/package-lock.json", "  \n");
        write(
            dir.path(),
            "v1/package.json",
            r#"{"dependencies": {"a": "workspace:*"}}"#,
        );
        write(
            dir.path(),
            "v1/package-lock.json",
            r#"{"lockfileVersion": 1, "dependencies": {}}"#,
        );
        write(
            dir.path(),
            "bad/package.json",
            r#"{"dependencies": {"a": "workspace:*"}}"#,
        );
        write(dir.path(), "bad/package-lock.json", "not json");
        let report = scan_dir(&ctx, dir.path()).await;
        let mut found: Vec<_> = report
            .warnings
            .iter()
            .map(|w| (w.code.as_str(), w.path.as_deref().unwrap()))
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                ("lockfile_empty", "empty/package-lock.json"),
                ("lockfile_unparseable", "bad/package-lock.json"),
                ("lockfile_v1_unsupported", "v1/package-lock.json"),
            ]
        );
    }

    #[tokio::test]
    async fn unreadable_files_become_warnings_with_their_path() {
        let (server, ctx) = server().await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("package.json"), [0xff, 0xfe]).unwrap();
        write(
            dir.path(),
            "ok/package.json",
            r#"{"dependencies": {"a": "workspace:*"}}"#,
        );
        fs::write(
            dir.path().join("ok").join("package-lock.json"),
            [0xff, 0xfe],
        )
        .unwrap();
        let report = scan_dir(&ctx, dir.path()).await;
        let read_failed: Vec<_> = report
            .warnings
            .iter()
            .filter(|w| w.code == "read_failed")
            .map(|w| w.path.clone().unwrap())
            .collect();
        assert_eq!(read_failed.len(), 2, "{:?}", report.warnings);
        assert!(read_failed.contains(&"package.json".to_string()));
        assert!(read_failed.contains(&"ok/package-lock.json".to_string()));
    }

    #[tokio::test]
    async fn maven_versions_come_from_parent_poms_and_imported_boms() {
        let (server, ctx) = server().await;
        mount_text(
            &server,
            "/maven/org/bom/bom/2.0/bom-2.0.pom",
            "<project><dependencyManagement><dependencies>\
             <dependency><groupId>org.lib</groupId><artifactId>from-bom</artifactId><version>5.5.0</version></dependency>\
             </dependencies></dependencyManagement></project>",
        )
        .await;
        mount_text(&server, "/maven/junit/junit/maven-metadata.xml", "<metadata><versioning><versions><version>4.13.2</version></versions></versioning></metadata>").await;
        mount_text(&server, "/maven/org/lib/from-bom/maven-metadata.xml", "<metadata><versioning><versions><version>5.6.0</version></versions></versioning></metadata>").await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "pom.xml",
            "<project><properties><junit.version>4.12</junit.version></properties>\
             <dependencyManagement><dependencies>\
             <dependency><groupId>junit</groupId><artifactId>junit</artifactId><version>${junit.version}</version></dependency>\
             <dependency><groupId>org.bom</groupId><artifactId>bom</artifactId><version>2.0</version><type>pom</type><scope>import</scope></dependency>\
             </dependencies></dependencyManagement></project>",
        );
        write(
            dir.path(),
            "module/pom.xml",
            "<project><parent><groupId>g</groupId><artifactId>root</artifactId><version>1</version></parent>\
             <dependencies>\
             <dependency><groupId>junit</groupId><artifactId>junit</artifactId></dependency>\
             <dependency><groupId>org.lib</groupId><artifactId>from-bom</artifactId></dependency>\
             </dependencies></project>",
        );
        let report = scan_dir(&ctx, dir.path()).await;
        let junit = dep(&report, "module/pom.xml", "junit:junit");
        assert_eq!(junit.declared, "4.12");
        assert_eq!(junit.status, DepStatus::Outdated);
        assert_eq!(
            dep(&report, "module/pom.xml", "org.lib:from-bom").declared,
            "5.5.0"
        );
    }

    #[tokio::test]
    async fn gradle_scripts_and_version_catalogs_are_scanned() {
        let (server, ctx) = server().await;
        mount_text(&server, "/maven/com/google/guava/guava/maven-metadata.xml", "<metadata><versioning><versions><version>33.0.0-jre</version><version>33.3.0-jre</version></versions></versioning></metadata>").await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "build.gradle.kts",
            "dependencies {\n  implementation(\"com.google.guava:guava:33.0.0-jre\")\n}\n",
        );
        write(
            dir.path(),
            "gradle/libs.versions.toml",
            "[libraries]\nguava = \"com.google.guava:guava:33.0.0-jre\"\n",
        );
        let report = scan_dir(&ctx, dir.path()).await;
        assert_eq!(report.manifests.len(), 2);
        assert_eq!(
            dep(&report, "build.gradle.kts", "com.google.guava:guava").status,
            DepStatus::Outdated
        );
        assert_eq!(
            dep(
                &report,
                "gradle/libs.versions.toml",
                "com.google.guava:guava"
            )
            .latest
            .as_deref(),
            Some("33.3.0-jre")
        );
    }

    #[tokio::test]
    async fn a_folder_named_build_is_scanned_and_errors_are_structured() {
        let (server, ctx) = server().await;
        mount_osv(&server, vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("build");
        write(
            &root,
            "package.json",
            r#"{"dependencies": {"a": "workspace:*"}}"#,
        );
        assert_eq!(scan_dir(&ctx, &root).await.manifests.len(), 1);

        let err = scan_local_path(&ctx, "/this/path/does/not/exist/depradar-test")
            .await
            .unwrap_err();
        assert_eq!(err.code, "local_dir_not_found");

        let empty = tempfile::tempdir().unwrap();
        write(empty.path(), "README.md", "hello");
        let err = scan_local_path(&ctx, &empty.path().to_string_lossy())
            .await
            .unwrap_err();
        assert_eq!(err.code, "no_manifests_found");
    }

    // ------------------------------------------------------ GitHub scans

    async fn mount_repo(server: &MockServer, tree: serde_json::Value) {
        mount_get(
            server,
            "/gh/repos/o/r",
            200,
            serde_json::json!({"default_branch": "main"}),
        )
        .await;
        mount_get(server, "/gh/repos/o/r/git/trees/main", 200, tree).await;
    }

    #[tokio::test]
    async fn github_scan_reads_files_from_raw_content_including_large_lockfiles() {
        let (server, ctx) = server().await;
        npm_latest(&server, "lodash", "4.17.21").await;
        mount_osv(&server, vec![("lodash@4.17.19", vec!["GHSA-x"])]).await;
        mount_repo(
            &server,
            serde_json::json!({"truncated": true, "tree": [
                {"path": "package.json", "type": "blob"},
                {"path": "package-lock.json", "type": "blob"},
                {"path": "node_modules/x/package.json", "type": "blob"},
                {"path": "README.md", "type": "blob"}]}),
        )
        .await;
        mount_text(
            &server,
            "/raw/o/r/main/package.json",
            r#"{"dependencies": {"lodash": "^4.17.0"}}"#,
        )
        .await;
        // > 1 MB: the Contents API would have returned an empty body.
        let padding = "x".repeat(1_500_000);
        mount_text(
            &server,
            "/raw/o/r/main/package-lock.json",
            &format!(
                r#"{{"lockfileVersion": 3, "padding": "{padding}", "packages": {{"node_modules/lodash": {{"version": "4.17.19"}}}}}}"#
            ),
        )
        .await;
        let report = scan_github_repo(&ctx, "https://github.com/o/r/tree/main", Some("tok"))
            .await
            .unwrap();
        assert_eq!(report.source, "o/r");
        let lodash = dep(&report, "package.json", "lodash");
        assert_eq!(lodash.installed.as_deref(), Some("4.17.19"));
        assert_eq!(lodash.vulnerabilities, ["GHSA-x"]);
        assert_eq!(codes(&report), ["tree_truncated"]);
    }

    #[tokio::test]
    async fn github_errors_are_precise() {
        let (server, ctx) = server().await;
        assert_eq!(
            scan_github_repo(&ctx, "nope", None).await.unwrap_err().code,
            "invalid_repo_format"
        );

        mount_repo(
            &server,
            serde_json::json!({"tree": [{"path": "README.md", "type": "blob"}]}),
        )
        .await;
        assert_eq!(
            scan_github_repo(&ctx, "o/r", None).await.unwrap_err().code,
            "no_manifests_found_github"
        );

        let (server2, ctx2) = self::server().await;
        mount_repo(&server2, serde_json::json!({"truncated": true, "tree": []})).await;
        assert_eq!(
            scan_github_repo(&ctx2, "o/r", None).await.unwrap_err().code,
            "github_tree_truncated"
        );

        let (_server3, ctx3) = self::server().await;
        assert_eq!(
            scan_github_repo(&ctx3, "o/r", None).await.unwrap_err().code,
            "github_repo_not_found"
        );
    }

    #[tokio::test]
    async fn github_file_failures_become_warnings() {
        let (server, ctx) = server().await;
        mount_osv(&server, vec![]).await;
        mount_repo(
            &server,
            serde_json::json!({"tree": [
                {"path": "package.json", "type": "blob"},
                {"path": "a/package.json", "type": "blob"},
                {"path": "gone/package.json", "type": "blob"}]}),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/raw/o/r/main/package.json"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        mount_text(
            &server,
            "/raw/o/r/main/a/package.json",
            r#"{"dependencies": {"x": "workspace:*"}}"#,
        )
        .await;
        let report = scan_github_repo(&ctx, "o/r", None).await.unwrap();
        assert_eq!(report.manifests.len(), 1);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].code, "github_forbidden");
        assert_eq!(report.warnings[0].path.as_deref(), Some("package.json"));
    }
}

/// Real integration tests (network + public registries). Skipped by default
/// since they depend on the network; run with
/// `cargo test -- --ignored --nocapture` (CI runs them weekly, see
/// `.github/workflows/live-tests.yml`).
#[cfg(test)]
mod live_tests {
    use super::*;

    async fn live_github(repo: &str, ecosystem: &str) -> ManifestScan {
        let ctx = ScanCtx::default();
        let report = scan_github_repo(&ctx, repo, None)
            .await
            .expect("scan failed");
        let manifest = report
            .manifests
            .into_iter()
            .find(|m| m.ecosystem == ecosystem && !m.dependencies.is_empty())
            .unwrap_or_else(|| panic!("expected a non-empty {ecosystem} manifest in {repo}"));
        let resolved = manifest
            .dependencies
            .iter()
            .filter(|d| d.latest.is_some())
            .count();
        assert!(
            resolved > 0,
            "no dependency of {repo} resolved a latest version"
        );
        manifest
    }

    #[tokio::test]
    #[ignore]
    async fn github_rust() {
        live_github("BurntSushi/ripgrep", "rust").await;
    }

    #[tokio::test]
    #[ignore]
    async fn github_npm() {
        live_github("sindresorhus/execa", "npm").await;
    }

    #[tokio::test]
    #[ignore]
    async fn github_go() {
        live_github("gin-gonic/gin", "go").await;
    }

    #[tokio::test]
    #[ignore]
    async fn github_php() {
        live_github("guzzle/guzzle", "php").await;
    }

    #[tokio::test]
    #[ignore]
    async fn github_ruby() {
        live_github("sinatra/sinatra", "ruby").await;
    }

    #[tokio::test]
    #[ignore]
    async fn github_java() {
        live_github("google/gson", "java").await;
    }

    #[tokio::test]
    #[ignore]
    async fn github_python() {
        live_github("pallets/flask", "python").await;
    }

    #[tokio::test]
    #[ignore]
    async fn large_lockfile_on_github_is_read() {
        // facebook/create-react-app's package-lock.json is ~2 MB: the
        // Contents API returns an empty body for it.
        let manifest = live_github("facebook/create-react-app", "npm").await;
        assert!(manifest.lockfile_path.is_some());
        assert!(manifest.dependencies.iter().any(|d| d.installed.is_some()));
    }

    #[tokio::test]
    #[ignore]
    async fn vulnerable_pins_are_reported_for_go_and_maven() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("go.mod"),
            "module m\n\nrequire golang.org/x/net v0.10.0\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("pom.xml"),
            "<project><dependencies><dependency><groupId>org.apache.logging.log4j</groupId>\
             <artifactId>log4j-core</artifactId><version>2.14.1</version></dependency></dependencies></project>",
        )
        .unwrap();
        let ctx = ScanCtx::default();
        let report = scan_local_path(&ctx, &dir.path().to_string_lossy())
            .await
            .unwrap();
        for m in &report.manifests {
            let d = &m.dependencies[0];
            assert!(
                !d.vulnerabilities.is_empty(),
                "{} has no advisories",
                d.name
            );
            assert!(d.latest.is_some());
        }
    }

    #[tokio::test]
    #[ignore]
    async fn maven_latest_is_a_stable_release() {
        let ctx = ScanCtx::default();
        match registry::latest_maven(&ctx.client, &ctx.ep, "org.junit.jupiter:junit-jupiter").await
        {
            Lookup::Found(v) => assert!(!crate::version::is_prerelease(&v), "{v}"),
            other => panic!("{other:?}"),
        }
    }
}

pub mod github;
pub mod local;

use std::future::Future;

use crate::error::AppError;
use crate::version::Ecosystem;

/// Where manifest and lockfile contents come from (a local folder, a GitHub
/// repository...). Paths are '/'-separated and relative to the scan root.
pub trait FileSource: Sync {
    /// `Ok(None)` means the file doesn't exist: not an error, just absent.
    fn read(&self, rel: &str) -> impl Future<Output = Result<Option<String>, AppError>> + Send;
}

/// Recognized manifest filenames.
pub const MANIFEST_FILES: [&str; 11] = [
    "Cargo.toml",
    "package.json",
    "requirements.txt",
    "pyproject.toml",
    "go.mod",
    "composer.json",
    "Gemfile",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "libs.versions.toml",
];

/// Recognized lockfile filenames (plus `pnpm-workspace.yaml`, which tells
/// whether a `pnpm-lock.yaml` covers nested packages).
pub const LOCKFILE_FILES: [&str; 11] = [
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "pnpm-workspace.yaml",
    "poetry.lock",
    "uv.lock",
    "pdm.lock",
    "composer.lock",
    "Gemfile.lock",
];

/// How deep a manifest may sit: a file `a/b/c/Cargo.toml` is at depth 4.
/// The same limit applies to local folders and to GitHub repositories.
pub const MAX_DEPTH: usize = 8;

/// Directory names skipped by both the local and the GitHub scanner.
pub const IGNORE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "venv",
    ".venv",
    "__pycache__",
    "dist",
    "build",
    ".next",
    ".cache",
    // Vendored/copied-in dependency sources (Composer, Go) — each one can
    // carry its own nested manifest, exactly like node_modules.
    "vendor",
];

/// Maps a manifest filename to its ecosystem.
pub fn ecosystem_for(filename: &str) -> Option<Ecosystem> {
    match filename {
        "Cargo.toml" => Some(Ecosystem::Rust),
        "package.json" => Some(Ecosystem::Npm),
        "requirements.txt" | "pyproject.toml" => Some(Ecosystem::Python),
        "go.mod" => Some(Ecosystem::Go),
        "composer.json" => Some(Ecosystem::Php),
        "Gemfile" => Some(Ecosystem::Ruby),
        "pom.xml" | "build.gradle" | "build.gradle.kts" | "libs.versions.toml" => {
            Some(Ecosystem::Java)
        }
        _ => None,
    }
}

/// The lockfile names that can hold a manifest's installed versions, in
/// order of preference. `requirements.txt`, `go.mod` and the Java manifests
/// have no universal lockfile format usable this way and map to nothing.
pub fn manifest_lockfiles(filename: &str) -> &'static [&'static str] {
    match filename {
        "Cargo.toml" => &["Cargo.lock"],
        "package.json" => &[
            "package-lock.json",
            "yarn.lock",
            "pnpm-lock.yaml",
            "bun.lock",
        ],
        "pyproject.toml" => &["poetry.lock", "uv.lock", "pdm.lock"],
        "composer.json" => &["composer.lock"],
        "Gemfile" => &["Gemfile.lock"],
        _ => &[],
    }
}

/// True if any directory segment of `path` (a '/'-separated relative path,
/// as returned by GitHub's tree API or built from a local walk) is one of
/// `IGNORE_DIRS`. The last segment (the filename itself) is not checked.
pub fn is_ignored_path(path: &str) -> bool {
    let mut segments = path.split('/');
    segments.next_back();
    segments.any(|segment| IGNORE_DIRS.contains(&segment))
}

/// Last path segment of a '/'-separated path.
pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Directory part of a '/'-separated path (`""` at the top level).
pub fn dirname(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// Joins a directory and a file name ('/'-separated, `""` = top level).
pub fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// True if a manifest/lockfile at `path` is within [`MAX_DEPTH`].
pub fn within_depth(path: &str) -> bool {
    path.split('/').count() <= MAX_DEPTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_ignored_path_detects_any_ignored_ancestor() {
        assert!(is_ignored_path("node_modules/foo/package.json"));
        assert!(is_ignored_path(
            "packages/api/node_modules/foo/package.json"
        ));
        assert!(!is_ignored_path("packages/api/Cargo.toml"));
        assert!(!is_ignored_path("Cargo.toml"));
    }

    #[test]
    fn maps_every_manifest_to_an_ecosystem() {
        for name in MANIFEST_FILES {
            assert!(ecosystem_for(name).is_some(), "{name}");
        }
        assert_eq!(ecosystem_for("README.md"), None);
        assert_eq!(ecosystem_for("build.gradle.kts"), Some(Ecosystem::Java));
    }

    #[test]
    fn lockfile_candidates_per_manifest() {
        assert_eq!(manifest_lockfiles("Cargo.toml"), &["Cargo.lock"]);
        assert_eq!(manifest_lockfiles("package.json").len(), 4);
        assert_eq!(manifest_lockfiles("pyproject.toml").len(), 3);
        assert_eq!(manifest_lockfiles("composer.json"), &["composer.lock"]);
        assert_eq!(manifest_lockfiles("Gemfile"), &["Gemfile.lock"]);
        assert!(manifest_lockfiles("go.mod").is_empty());
        assert!(manifest_lockfiles("requirements.txt").is_empty());
        for name in MANIFEST_FILES {
            for lock in manifest_lockfiles(name) {
                assert!(LOCKFILE_FILES.contains(lock), "{lock}");
            }
        }
    }

    #[test]
    fn path_helpers() {
        assert_eq!(basename("a/b/c.txt"), "c.txt");
        assert_eq!(basename("c.txt"), "c.txt");
        assert_eq!(dirname("a/b/c.txt"), "a/b");
        assert_eq!(dirname("c.txt"), "");
        assert_eq!(join_path("", "x"), "x");
        assert_eq!(join_path("a", "x"), "a/x");
    }

    #[test]
    fn depth_limit_is_inclusive() {
        assert!(within_depth("Cargo.toml"));
        assert!(within_depth("a/b/c/d/e/f/g/Cargo.toml"));
        assert!(!within_depth("a/b/c/d/e/f/g/h/Cargo.toml"));
    }
}

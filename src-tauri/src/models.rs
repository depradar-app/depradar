use serde::Serialize;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DepStatus {
    UpToDate,
    Outdated,
    Major,
    Unknown,
}

/// Why a dependency is `Unknown` (or couldn't be fully checked). Stable codes
/// the frontend translates (`reason.<code>`).
pub mod reason {
    /// The declared constraint points at a local path, a git URL, a
    /// workspace member... — nothing to look up on a public registry.
    pub const NON_REGISTRY: &str = "non_registry";
    /// The registry answered but doesn't know this package.
    pub const NOT_FOUND: &str = "not_found";
    /// The registry could not be reached or answered with an error.
    pub const NETWORK_ERROR: &str = "network_error";
    /// The package exists but its constraint / version can't be interpreted.
    pub const UNPARSEABLE: &str = "unparseable";
}

#[derive(Serialize, Clone, Debug)]
pub struct DependencyInfo {
    /// Name as written in the manifest (for an npm alias: the alias).
    pub name: String,
    /// The real registry package when it differs from `name` (npm aliases).
    pub package: Option<String>,
    pub declared: String,
    /// The version actually pinned in a lockfile, when one was found for
    /// this manifest and contained an entry for this package.
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub status: DepStatus,
    /// Set whenever `status` is `Unknown` for a reason worth telling the user.
    pub reason: Option<String>,
    /// Known-vulnerability identifiers (GHSA/CVE/...) from OSV.dev affecting
    /// the checked version.
    pub vulnerabilities: Vec<String>,
    /// Which version the vulnerability lookup used: `"installed"` (from a
    /// lockfile) or `"declared"` (an exact pin in the manifest). `None` when
    /// the dependency couldn't be checked at all.
    pub vuln_basis: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ManifestScan {
    pub ecosystem: String,
    pub manifest_path: String,
    /// Path of the lockfile found for this manifest, if any (lets the
    /// frontend distinguish "no lockfile found" from "found but this
    /// dependency isn't in it").
    pub lockfile_path: Option<String>,
    pub dependencies: Vec<DependencyInfo>,
}

/// A non-fatal annotation on a scan. Like `AppError`, it carries a stable
/// code the frontend translates (`warning.<code>`) instead of a sentence.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub code: String,
    pub path: Option<String>,
    pub detail: Option<String>,
}

impl Warning {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_string(),
            path: None,
            detail: None,
        }
    }

    pub fn at(code: &str, path: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            path: Some(path.into()),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct ScanReport {
    pub source: String,
    pub manifests: Vec<ManifestScan>,
    pub warnings: Vec<Warning>,
}

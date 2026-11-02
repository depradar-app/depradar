//! Extracts the dependencies declared in a `Cargo.toml`.

use std::collections::HashMap;
use toml::map::Map;
use toml::Value;

const DEP_SECTIONS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// `[workspace.dependencies]` of a workspace root: dependency key →
/// (registry package name, version requirement).
pub type WorkspaceDeps = HashMap<String, (String, String)>;

/// Dependencies of a `Cargo.toml`: the plain, `dev-` and `build-` tables, the
/// same three under every `[target.<cfg>.*]`, and (for a workspace root) its
/// `[workspace.dependencies]`. A renamed dependency (`package = "serde"`) is
/// reported under its real crate name, and `{ workspace = true }` entries are
/// resolved through `workspace` (the root manifest's table).
pub fn parse_cargo_toml(content: &str, workspace: Option<&WorkspaceDeps>) -> Vec<(String, String)> {
    let Ok(doc) = content.parse::<Value>() else {
        return vec![];
    };

    let mut deps = vec![];
    for section in DEP_SECTIONS {
        if let Some(Value::Table(table)) = doc.get(section) {
            collect(table, workspace, &mut deps);
        }
    }
    if let Some(Value::Table(targets)) = doc.get("target") {
        for target in targets.values() {
            for section in DEP_SECTIONS {
                if let Some(Value::Table(table)) = target.get(section) {
                    collect(table, workspace, &mut deps);
                }
            }
        }
    }
    if let Some(Value::Table(table)) = doc.get("workspace").and_then(|w| w.get("dependencies")) {
        collect(table, None, &mut deps);
    }
    deps
}

/// True when some dependency entry is `{ workspace = true }`, i.e. its
/// version lives in an ancestor manifest.
pub fn uses_workspace_deps(content: &str) -> bool {
    let Ok(doc) = content.parse::<Value>() else {
        return false;
    };
    let inherits = |table: &Value| {
        table.as_table().is_some_and(|t| {
            t.values()
                .any(|v| v.get("workspace").and_then(Value::as_bool) == Some(true))
        })
    };
    DEP_SECTIONS.iter().filter_map(|s| doc.get(s)).any(inherits)
        || doc
            .get("target")
            .and_then(Value::as_table)
            .is_some_and(|targets| {
                targets
                    .values()
                    .flat_map(|t| DEP_SECTIONS.iter().filter_map(|s| t.get(s)))
                    .any(inherits)
            })
}

/// The `[workspace.dependencies]` table of a workspace root manifest.
pub fn parse_workspace_deps(content: &str) -> WorkspaceDeps {
    let mut out = HashMap::new();
    let Ok(doc) = content.parse::<Value>() else {
        return out;
    };
    if let Some(Value::Table(table)) = doc.get("workspace").and_then(|w| w.get("dependencies")) {
        for (key, value) in table {
            if let Some((name, version)) = entry_version(key, value) {
                out.insert(key.clone(), (name, version));
            }
        }
    }
    out
}

/// (crate name, version requirement) for one dependency entry that
/// references a registry version.
fn entry_version(key: &str, value: &Value) -> Option<(String, String)> {
    match value {
        Value::String(req) => Some((key.to_string(), req.clone())),
        // Dependency with options (features, path, git, workspace...). Purely
        // local dependencies (path/git with no version) aren't published and
        // have nothing to check.
        Value::Table(t) => {
            let Some(Value::String(req)) = t.get("version") else {
                return None;
            };
            let name = t.get("package").and_then(Value::as_str).unwrap_or(key);
            Some((name.to_string(), req.clone()))
        }
        _ => None,
    }
}

fn collect(
    table: &Map<String, Value>,
    workspace: Option<&WorkspaceDeps>,
    out: &mut Vec<(String, String)>,
) {
    for (key, value) in table {
        let inherited = value.get("workspace").and_then(Value::as_bool) == Some(true);
        if inherited {
            if let Some((name, version)) = workspace.and_then(|ws| ws.get(key)) {
                out.push((name.clone(), version.clone()));
            }
        } else if let Some(entry) = entry_version(key, value) {
            out.push(entry);
        }
    }
}

/// Parses a Cargo.lock, returning every resolved version found per package
/// name. A workspace can resolve multiple versions of the same crate, so
/// each name maps to all versions found rather than a single one.
pub fn parse_cargo_lock(content: &str) -> HashMap<String, Vec<semver::Version>> {
    let Ok(lockfile) = content.parse::<cargo_lock::Lockfile>() else {
        return HashMap::new();
    };

    let mut map: HashMap<String, Vec<semver::Version>> = HashMap::new();
    for package in lockfile.packages {
        map.entry(package.name.as_str().to_string())
            .or_default()
            .push(package.version);
    }
    map
}

/// Picks the version from `candidates` (all resolved versions of a crate
/// name found in `Cargo.lock`) that matches `declared`'s own range; falls
/// back to the highest version when there is no single match (an
/// unparseable `declared`, or none/several candidates matching).
pub fn resolve_installed_version(
    candidates: &[semver::Version],
    declared: &str,
) -> Option<semver::Version> {
    if candidates.len() == 1 {
        return candidates.first().cloned();
    }
    if let Ok(req) = semver::VersionReq::parse(declared) {
        if let Some(matched) = candidates.iter().filter(|v| req.matches(v)).max() {
            return Some(matched.clone());
        }
    }
    candidates.iter().max().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_and_table_deps() {
        let toml = r#"
[dependencies]
serde = "1.0"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
proptest = "1.4"
"#;
        let mut deps = parse_cargo_toml(toml, None);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("proptest".to_string(), "1.4".to_string()),
                ("serde".to_string(), "1.0".to_string()),
                ("tokio".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn skips_path_only_dependencies() {
        let toml = r#"
[dependencies]
internal_lib = { path = "../internal_lib" }
serde = "1.0"
"#;
        let deps = parse_cargo_toml(toml, None);
        assert_eq!(deps, vec![("serde".to_string(), "1.0".to_string())]);
    }

    #[test]
    fn parses_cargo_lock_versions() {
        let lock = r#"
version = 3

[[package]]
name = "serde"
version = "1.0.195"

[[package]]
name = "libc"
version = "0.2.150"
"#;
        let map = parse_cargo_lock(lock);
        assert_eq!(
            map.get("serde").unwrap(),
            &vec![semver::Version::new(1, 0, 195)]
        );
        assert_eq!(
            map.get("libc").unwrap(),
            &vec![semver::Version::new(0, 2, 150)]
        );
    }

    #[test]
    fn parses_cargo_lock_with_multiple_resolved_versions_of_same_crate() {
        let lock = r#"
version = 3

[[package]]
name = "syn"
version = "1.0.109"

[[package]]
name = "syn"
version = "2.0.48"
"#;
        let map = parse_cargo_lock(lock);
        let mut versions = map.get("syn").unwrap().clone();
        versions.sort();
        assert_eq!(
            versions,
            vec![
                semver::Version::new(1, 0, 109),
                semver::Version::new(2, 0, 48)
            ]
        );
    }

    #[test]
    fn malformed_cargo_lock_returns_empty_map() {
        assert!(parse_cargo_lock("not a lockfile").is_empty());
    }

    #[test]
    fn resolve_installed_version_returns_only_candidate() {
        let candidates = vec![semver::Version::new(1, 0, 195)];
        assert_eq!(
            resolve_installed_version(&candidates, "1.0"),
            Some(semver::Version::new(1, 0, 195))
        );
    }

    #[test]
    fn resolve_installed_version_picks_range_match_among_several() {
        let candidates = vec![
            semver::Version::new(1, 0, 109),
            semver::Version::new(2, 0, 48),
        ];
        assert_eq!(
            resolve_installed_version(&candidates, "^1.0"),
            Some(semver::Version::new(1, 0, 109))
        );
    }

    #[test]
    fn resolve_installed_version_falls_back_to_highest_when_none_match() {
        let candidates = vec![
            semver::Version::new(1, 0, 109),
            semver::Version::new(2, 0, 48),
        ];
        assert_eq!(
            resolve_installed_version(&candidates, "^3.0"),
            Some(semver::Version::new(2, 0, 48))
        );
    }

    #[test]
    fn resolve_installed_version_empty_candidates_is_none() {
        assert_eq!(resolve_installed_version(&[], "^1.0"), None);
    }

    #[test]
    fn renamed_dependency_is_reported_under_its_real_crate_name() {
        let toml = r#"
[dependencies]
foo = { package = "serde", version = "1" }
"#;
        assert_eq!(
            parse_cargo_toml(toml, None),
            vec![("serde".to_string(), "1".to_string())]
        );
    }

    #[test]
    fn target_specific_dependencies_are_included() {
        let toml = r#"
[target.'cfg(unix)'.dependencies]
libc = "0.2"

[target.'cfg(windows)'.build-dependencies]
winres = { version = "0.1" }
"#;
        let mut deps = parse_cargo_toml(toml, None);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("libc".to_string(), "0.2".to_string()),
                ("winres".to_string(), "0.1".to_string())
            ]
        );
    }

    #[test]
    fn workspace_root_dependencies_are_listed_and_inherited_by_members() {
        let root = r#"
[workspace]
members = ["a"]

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
serde = "1.0"
local = { path = "crates/local" }
"#;
        let mut own = parse_cargo_toml(root, None);
        own.sort();
        assert_eq!(
            own,
            vec![
                ("serde".to_string(), "1.0".to_string()),
                ("tokio".to_string(), "1".to_string())
            ]
        );

        let ws = parse_workspace_deps(root);
        assert_eq!(ws.len(), 2);

        let member = r#"
[dependencies]
tokio = { workspace = true, features = ["rt"] }
serde.workspace = true
missing = { workspace = true }

[target.'cfg(unix)'.dependencies]
libc = { workspace = true }
"#;
        assert!(uses_workspace_deps(member));
        let mut deps = parse_cargo_toml(member, Some(&ws));
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("serde".to_string(), "1.0".to_string()),
                ("tokio".to_string(), "1".to_string())
            ]
        );
        // Without the root table, inherited entries cannot be resolved.
        assert!(parse_cargo_toml(member, None).is_empty());
    }

    #[test]
    fn plain_manifests_do_not_use_workspace_deps() {
        assert!(!uses_workspace_deps("[dependencies]\nserde = \"1\"\n"));
        assert!(!uses_workspace_deps("not toml {{{"));
        assert!(parse_workspace_deps("not toml {{{").is_empty());
        assert!(parse_cargo_toml("not toml {{{", None).is_empty());
    }

    #[test]
    fn resolve_installed_version_picks_the_highest_of_several_matches() {
        let candidates = vec![
            semver::Version::new(1, 0, 1),
            semver::Version::new(1, 0, 5),
            semver::Version::new(2, 0, 0),
        ];
        assert_eq!(
            resolve_installed_version(&candidates, ">=1, <2"),
            Some(semver::Version::new(1, 0, 5))
        );
    }
}

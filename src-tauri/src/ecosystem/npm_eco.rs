//! Extracts the dependencies declared in a `package.json`.

use std::collections::HashMap;

pub fn parse_package_json(content: &str) -> Vec<(String, String)> {
    let json: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut deps = vec![];
    for section in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        if let Some(obj) = json.get(section).and_then(|v| v.as_object()) {
            for (name, value) in obj {
                if let Some(req) = value.as_str() {
                    deps.push((name.clone(), req.to_string()));
                }
            }
        }
    }
    deps
}

/// The result of attempting to read installed versions out of a
/// `package-lock.json`.
pub enum PackageLock {
    /// Successfully parsed a lockfileVersion 2/3 `packages` map: lockfile
    /// path (`node_modules/x`, `packages/app/node_modules/x`...) → version.
    Packages(HashMap<String, String>),
    /// The file only has lockfileVersion 1's nested `dependencies` tree,
    /// which isn't supported — the caller should surface this as a warning
    /// rather than silently reporting no installed versions.
    UnsupportedV1,
}

/// Extracts installed versions from lockfileVersion 2/3's flat `packages`
/// map (what current npm produces). Workspace links (`"link": true`) point
/// at local folders, not at an installed version, and are skipped. Returns
/// `None` if `content` isn't valid JSON at all.
pub fn parse_package_lock(content: &str) -> Option<PackageLock> {
    let json: serde_json::Value = serde_json::from_str(content).ok()?;

    let Some(packages) = json.get("packages").and_then(|v| v.as_object()) else {
        return Some(PackageLock::UnsupportedV1);
    };

    let mut installed = HashMap::new();
    for (path, info) in packages {
        if path.is_empty() || info.get("link").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        if let Some(version) = info.get("version").and_then(|v| v.as_str()) {
            installed.insert(path.clone(), version.to_string());
        }
    }
    Some(PackageLock::Packages(installed))
}

/// Installed version of `name` for a manifest living in `member_dir`
/// (relative to the lockfile's folder, `""` for the lockfile's own package):
/// a copy nested under the member's own `node_modules` wins over the hoisted
/// top-level one.
pub fn installed_from_lock(
    packages: &HashMap<String, String>,
    member_dir: &str,
    name: &str,
) -> Option<String> {
    if !member_dir.is_empty() {
        if let Some(v) = packages.get(&format!("{member_dir}/node_modules/{name}")) {
            return Some(v.clone());
        }
    }
    packages.get(&format!("node_modules/{name}")).cloned()
}

/// Extracts installed versions from a `yarn.lock` (classic and berry).
/// Keys are both `name@range` (as written in the manifest) and bare `name`.
pub fn parse_yarn_lock(content: &str) -> HashMap<String, String> {
    let mut installed = HashMap::new();
    let mut specs: Vec<(String, String)> = vec![];

    for line in content.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') && line.trim_end().ends_with(':') {
            specs.clear();
            let header = line.trim_end().trim_end_matches(':');
            for spec in header.split(',') {
                let spec = spec.trim().trim_matches('"');
                // The range starts at the last '@' that isn't the scope's own.
                if let Some(at) = spec.rfind('@').filter(|&i| i > 0) {
                    let range = spec[at + 1..].trim_start_matches("npm:");
                    specs.push((spec[..at].to_string(), range.to_string()));
                }
            }
            continue;
        }
        let trimmed = line.trim();
        let version = trimmed
            .strip_prefix("version ")
            .or_else(|| trimmed.strip_prefix("version:"))
            .map(|v| v.trim().trim_matches('"'));
        if let Some(version) = version {
            for (name, range) in &specs {
                installed.insert(format!("{name}@{range}"), version.to_string());
                installed.insert(name.clone(), version.to_string());
            }
        }
    }
    installed
}

/// Extracts installed versions from a `pnpm-lock.yaml`'s `packages:` keys
/// (`/name@1.2.3(peer)`, `name@1.2.3`, or the v5 `/name/1.2.3`); when several
/// versions of a package are present, the highest wins.
pub fn parse_pnpm_lock(content: &str) -> HashMap<String, String> {
    let mut installed: HashMap<String, String> = HashMap::new();
    let mut in_packages = false;

    for line in content.lines() {
        if !line.starts_with(' ') && !line.trim().is_empty() {
            in_packages = line.trim_end() == "packages:";
            continue;
        }
        if !in_packages || !line.starts_with("  ") || line.starts_with("   ") {
            continue;
        }
        let key = line.trim().trim_end_matches(':').trim_matches(['\'', '"']);
        let key = key.trim_start_matches('/');
        let key = key.split('(').next().unwrap_or(key);
        // v5 keys are "name/1.2.3[_peer@x]"; v6+ keys are "name@1.2.3".
        let v5 = key
            .rsplit_once('/')
            .filter(|(_, rest)| rest.starts_with(|c: char| c.is_ascii_digit()));
        let parsed = v5.or_else(|| {
            key.rfind('@')
                .filter(|&i| i > 0)
                .map(|at| (&key[..at], &key[at + 1..]))
        });
        let Some((name, rest)) = parsed else {
            continue;
        };
        let version = rest.split('_').next().unwrap_or(rest);
        if name.is_empty() || !version.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let is_higher = installed
            .get(name)
            .is_none_or(|current| version_gt(version, current));
        if is_higher {
            installed.insert(name.to_string(), version.to_string());
        }
    }
    installed
}

/// Extracts installed versions from a text `bun.lock` (JSONC): each entry of
/// `"packages"` looks like `"name": ["name@1.2.3", ...]`. Entries of nested
/// copies (`"parent/name"`) are skipped, only top-level names are kept.
pub fn parse_bun_lock(content: &str) -> HashMap<String, String> {
    let mut installed = HashMap::new();
    let mut in_packages = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("\"packages\"") {
            in_packages = true;
            continue;
        }
        if !in_packages {
            continue;
        }
        // `"key": ["name@version", ...`
        let Some(rest) = trimmed.strip_prefix('"') else {
            continue;
        };
        let Some((key, rest)) = rest.split_once("\": [\"") else {
            continue;
        };
        let Some((spec, _)) = rest.split_once('"') else {
            continue;
        };
        let top_level = match key.strip_prefix('@') {
            Some(scoped) => scoped.matches('/').count() == 1,
            None => !key.contains('/'),
        };
        if !top_level {
            continue;
        }
        if let Some(at) = spec.rfind('@').filter(|&i| i > 0) {
            installed.insert(spec[..at].to_string(), spec[at + 1..].to_string());
        }
    }
    installed
}

fn version_gt(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(a), Ok(b)) => a > b,
        _ => a > b,
    }
}

/// Parses an npm "alias" dependency spec — `"my-name": "npm:<real-name>@<range>"`
/// installs *another* package under a different name. The alias key itself
/// was never published to the registry, only `<real-name>` was, so both the
/// "latest version" lookup and the declared-range comparison need to target
/// that instead. Returns `None` for anything that isn't this syntax.
pub fn parse_npm_alias(declared: &str) -> Option<(&str, &str)> {
    let rest = declared.trim().strip_prefix("npm:")?;
    if rest.is_empty() {
        return None;
    }

    // Scoped names ("@scope/pkg") contain their own '/', so the '@'
    // separating the name from the range has to be looked for *after* the
    // leading '@' of the scope, not the first '@' in the whole string.
    let split_at = if let Some(scoped_rest) = rest.strip_prefix('@') {
        scoped_rest.find('@').map(|i| i + 1)
    } else {
        rest.find('@')
    };

    match split_at {
        Some(i) => {
            let (name, range) = rest.split_at(i);
            if name.is_empty() {
                None
            } else {
                Some((name, &range[1..]))
            }
        }
        // "npm:lodash" with no version pin at all.
        None => Some((rest, "")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dependencies_and_dev_dependencies() {
        let json = r#"{
            "dependencies": { "react": "^18.2.0" },
            "devDependencies": { "vite": "~5.0.0" }
        }"#;
        let mut deps = parse_package_json(json);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("react".to_string(), "^18.2.0".to_string()),
                ("vite".to_string(), "~5.0.0".to_string()),
            ]
        );
    }

    #[test]
    fn ignores_malformed_json() {
        assert!(parse_package_json("not json").is_empty());
    }

    #[test]
    fn parses_v3_package_lock_and_skips_links() {
        let lock = r#"{
            "lockfileVersion": 3,
            "packages": {
                "": { "name": "root" },
                "node_modules/react": { "version": "18.2.0" },
                "node_modules/@scope/pkg": { "version": "2.1.0" },
                "packages/app/node_modules/left-pad": { "version": "1.3.0" },
                "node_modules/@app/web": { "version": "0.0.0", "link": true }
            }
        }"#;
        match parse_package_lock(lock).unwrap() {
            PackageLock::Packages(map) => {
                assert_eq!(map.get("node_modules/react"), Some(&"18.2.0".to_string()));
                assert!(!map.contains_key("node_modules/@app/web"));
                assert!(!map.contains_key(""));
                assert_eq!(
                    installed_from_lock(&map, "", "@scope/pkg"),
                    Some("2.1.0".to_string())
                );
                // Nested copy beats the hoisted one for the member that owns it.
                assert_eq!(
                    installed_from_lock(&map, "packages/app", "left-pad"),
                    Some("1.3.0".to_string())
                );
                assert_eq!(installed_from_lock(&map, "", "left-pad"), None);
                assert_eq!(
                    installed_from_lock(&map, "packages/app", "react"),
                    Some("18.2.0".to_string())
                );
            }
            PackageLock::UnsupportedV1 => panic!("expected a Packages result"),
        }
    }

    #[test]
    fn parses_yarn_classic_and_berry_lockfiles() {
        let classic = r#"# yarn lockfile v1

"@babel/core@^7.0.0", "@babel/core@^7.1.0":
  version "7.1.6"
  resolved "https://registry.yarnpkg.com/x"

lodash@^4.17.0:
  version "4.17.21"
"#;
        let m = parse_yarn_lock(classic);
        assert_eq!(m.get("@babel/core@^7.1.0"), Some(&"7.1.6".to_string()));
        assert_eq!(m.get("@babel/core"), Some(&"7.1.6".to_string()));
        assert_eq!(m.get("lodash@^4.17.0"), Some(&"4.17.21".to_string()));

        let berry = r#"__metadata:
  version: 6

"lodash@npm:^4.17.0":
  version: 4.17.21
  resolution: "lodash@npm:4.17.21"
"#;
        let m = parse_yarn_lock(berry);
        assert_eq!(m.get("lodash@^4.17.0"), Some(&"4.17.21".to_string()));
        assert_eq!(m.get("lodash"), Some(&"4.17.21".to_string()));
    }

    #[test]
    fn parses_pnpm_lock_package_keys_across_formats() {
        let v9 = "lockfileVersion: '9.0'

importers:
  .:
    dependencies:
      react:
        version: 18.2.0

packages:

  react@18.2.0:
    resolution: {integrity: x}

  '@scope/pkg@2.1.0(react@18.2.0)':
    resolution: {integrity: y}

snapshots:
  react@18.2.0: {}
";
        let m = parse_pnpm_lock(v9);
        assert_eq!(m.get("react"), Some(&"18.2.0".to_string()));
        assert_eq!(m.get("@scope/pkg"), Some(&"2.1.0".to_string()));
        assert!(!m.contains_key("lockfileVersion"));

        let v6 = "packages:

  /lodash@4.17.20:
    dev: false
  /lodash@4.17.21:
    dev: false
";
        assert_eq!(
            parse_pnpm_lock(v6).get("lodash"),
            Some(&"4.17.21".to_string())
        );

        let v5 = "packages:
  /@scope/pkg/1.0.0_react@18.0.0:
    dev: false
  /left-pad/1.3.0:
    dev: false
";
        let m = parse_pnpm_lock(v5);
        assert_eq!(m.get("@scope/pkg"), Some(&"1.0.0".to_string()));
        assert_eq!(m.get("left-pad"), Some(&"1.3.0".to_string()));
        assert!(parse_pnpm_lock("").is_empty());
    }

    #[test]
    fn includes_optional_and_peer_dependencies() {
        let json = r#"{
            "dependencies": { "a": "^1" },
            "optionalDependencies": { "b": "^2" },
            "peerDependencies": { "c": "^3" }
        }"#;
        let deps = parse_package_json(json);
        assert_eq!(deps.len(), 3);
        assert!(deps.contains(&("b".to_string(), "^2".to_string())));
        assert!(deps.contains(&("c".to_string(), "^3".to_string())));
    }

    #[test]
    fn v1_only_lockfile_is_reported_as_unsupported() {
        let lock = r#"{
            "lockfileVersion": 1,
            "dependencies": {
                "react": { "version": "18.2.0" }
            }
        }"#;
        match parse_package_lock(lock).unwrap() {
            PackageLock::UnsupportedV1 => {}
            PackageLock::Packages(_) => panic!("expected UnsupportedV1"),
        }
    }

    #[test]
    fn malformed_package_lock_json_is_none() {
        assert!(parse_package_lock("not json").is_none());
    }

    #[test]
    fn parses_a_plain_npm_alias() {
        assert_eq!(
            parse_npm_alias("npm:lodash@^4.17.0"),
            Some(("lodash", "^4.17.0"))
        );
    }

    #[test]
    fn parses_a_scoped_npm_alias() {
        assert_eq!(
            parse_npm_alias("npm:@scope/pkg@^2.0.0"),
            Some(("@scope/pkg", "^2.0.0"))
        );
    }

    #[test]
    fn parses_an_npm_alias_with_no_version_pin() {
        assert_eq!(parse_npm_alias("npm:lodash"), Some(("lodash", "")));
        assert_eq!(parse_npm_alias("npm:@scope/pkg"), Some(("@scope/pkg", "")));
    }

    #[test]
    fn non_alias_declarations_are_not_parsed() {
        assert_eq!(parse_npm_alias("^4.17.0"), None);
        assert_eq!(parse_npm_alias("workspace:*"), None);
        assert_eq!(parse_npm_alias("file:../lib"), None);
        assert_eq!(parse_npm_alias("npm:"), None);
    }

    #[test]
    fn parses_bun_lock_top_level_entries() {
        let lock = r#"{
  "lockfileVersion": 1,
  "workspaces": { "": { "name": "app" } },
  "packages": {
    "lodash": ["lodash@4.17.21", "", {}, "sha512-abc"],
    "@scope/pkg": ["@scope/pkg@2.1.0", "", { "dependencies": {} }, "sha512-def"],
    "react/loose-envify": ["loose-envify@1.4.0", "", {}, "sha512-x"],
    "@scope/pkg/nested": ["nested@1.0.0", "", {}, "sha512-y"],
    "broken": [1, 2],
  }
}"#;
        let m = parse_bun_lock(lock);
        assert_eq!(m.get("lodash"), Some(&"4.17.21".to_string()));
        assert_eq!(m.get("@scope/pkg"), Some(&"2.1.0".to_string()));
        assert!(!m.contains_key("loose-envify"));
        assert!(!m.contains_key("nested"));
        assert_eq!(m.len(), 2);
        assert!(parse_bun_lock("").is_empty());
    }
}

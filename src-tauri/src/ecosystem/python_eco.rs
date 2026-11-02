use std::collections::HashMap;

/// PEP 503 name normalisation: lowercase, runs of `-`, `_`, `.` collapsed to
/// one `-`. Lockfiles store normalised names; manifests can spell them any
/// way.
pub fn normalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_sep = false;
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            if !last_sep {
                out.push('-');
            }
            last_sep = true;
        } else {
            out.push(c.to_ascii_lowercase());
            last_sep = false;
        }
    }
    out
}

/// Joins lines ending with a backslash (as emitted by
/// `pip-compile --generate-hashes`) into logical requirement lines.
fn logical_lines(content: &str) -> Vec<String> {
    let mut lines = vec![];
    let mut current = String::new();
    for raw in content.lines() {
        let trimmed = raw.trim_end();
        if let Some(stripped) = trimmed.strip_suffix('\\') {
            current.push_str(stripped);
            current.push(' ');
        } else {
            current.push_str(trimmed);
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        lines.push(current);
    }
    lines
}

pub fn parse_requirements_txt(content: &str) -> Vec<(String, String)> {
    let mut deps = vec![];
    for raw_line in logical_lines(content) {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        // Strip a trailing comment, an environment marker, then per-requirement
        // options such as `--hash=...`.
        let line = line.split(" #").next().unwrap_or(line).trim();
        let line = line.split(';').next().unwrap_or(line).trim();
        let line = line.split(" --").next().unwrap_or(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some((name, req)) = split_requirement(line) {
            deps.push((name, req));
        }
    }
    deps
}

fn push_spec(deps: &mut Vec<(String, String)>, spec: &str) {
    if let Some(dep) = split_requirement(spec) {
        deps.push(dep);
    }
}

fn push_spec_list(deps: &mut Vec<(String, String)>, list: Option<&toml::Value>) {
    if let Some(items) = list.and_then(|l| l.as_array()) {
        for item in items {
            if let Some(spec) = item.as_str() {
                push_spec(deps, spec);
            }
        }
    }
}

fn push_poetry_table(deps: &mut Vec<(String, String)>, table: Option<&toml::Value>) {
    let Some(table) = table.and_then(|d| d.as_table()) else {
        return;
    };
    for (name, value) in table {
        if name.eq_ignore_ascii_case("python") {
            continue; // constraint on the interpreter itself, not a package
        }
        match value {
            toml::Value::String(req) => deps.push((name.clone(), req.clone())),
            toml::Value::Table(t) => {
                if let Some(toml::Value::String(req)) = t.get("version") {
                    deps.push((name.clone(), req.clone()));
                } else if t.contains_key("git") || t.contains_key("path") || t.contains_key("url") {
                    deps.push((name.clone(), "git+local".to_string()));
                }
            }
            _ => {}
        }
    }
}

pub fn parse_pyproject_toml(content: &str) -> Vec<(String, String)> {
    let doc: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut deps = vec![];

    // PEP 621 : [project] dependencies = ["requests>=2.0", ...]
    push_spec_list(
        &mut deps,
        doc.get("project").and_then(|p| p.get("dependencies")),
    );
    // PEP 621 optional-dependencies: one list per extra.
    if let Some(extras) = doc
        .get("project")
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(|d| d.as_table())
    {
        for list in extras.values() {
            push_spec_list(&mut deps, Some(list));
        }
    }
    // PEP 735 dependency groups (entries may also be `{ include-group = ... }`).
    if let Some(groups) = doc.get("dependency-groups").and_then(|d| d.as_table()) {
        for list in groups.values() {
            push_spec_list(&mut deps, Some(list));
        }
    }

    // Poetry : main table, legacy dev table, and dependency groups.
    let poetry = doc.get("tool").and_then(|t| t.get("poetry"));
    push_poetry_table(&mut deps, poetry.and_then(|p| p.get("dependencies")));
    push_poetry_table(&mut deps, poetry.and_then(|p| p.get("dev-dependencies")));
    if let Some(groups) = poetry
        .and_then(|p| p.get("group"))
        .and_then(|g| g.as_table())
    {
        for group in groups.values() {
            push_poetry_table(&mut deps, group.get("dependencies"));
        }
    }

    deps
}

/// Splits a PEP 508 identifier ("requests>=2.28.0", "black[dev]==23.1.0",
/// "pkg @ https://...") into (name, raw constraint).
fn split_requirement(spec: &str) -> Option<(String, String)> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }

    let end = spec
        .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.'))
        .unwrap_or(spec.len());
    let name = spec[..end].trim().to_string();
    if name.is_empty() {
        return None;
    }

    let mut rest = spec[end..].trim();
    if rest.starts_with('[') {
        // Retire les extras : "requests[security]>=2.0" -> ">=2.0"
        rest = rest.find(']').map(|i| rest[i + 1..].trim()).unwrap_or(rest);
    }

    Some((name, rest.to_string()))
}

/// Extracts installed versions from a lockfile made of `[[package]]` TOML
/// entries (`poetry.lock`, `uv.lock`, `pdm.lock`). Names are PEP 503
/// normalised. Returns an empty map if the file doesn't parse as TOML.
pub fn parse_poetry_lock(content: &str) -> HashMap<String, String> {
    let Ok(doc) = content.parse::<toml::Value>() else {
        return HashMap::new();
    };

    let mut installed = HashMap::new();
    if let Some(packages) = doc.get("package").and_then(|p| p.as_array()) {
        for package in packages {
            let name = package.get("name").and_then(|v| v.as_str());
            let version = package.get("version").and_then(|v| v.as_str());
            if let (Some(name), Some(version)) = (name, version) {
                installed.insert(normalize_name(name), version.to_string());
            }
        }
    }
    installed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requirements_txt() {
        let content = "\
requests==2.28.1
# a comment
flask>=2.0,<3.0  # inline comment
numpy
black[dev]==23.1.0
-e git+https://example.com/foo.git
importlib-metadata>=6.0; python_version < '3.10'
";
        let mut deps = parse_requirements_txt(content);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("black".to_string(), "==23.1.0".to_string()),
                ("flask".to_string(), ">=2.0,<3.0".to_string()),
                ("importlib-metadata".to_string(), ">=6.0".to_string()),
                ("numpy".to_string(), "".to_string()),
                ("requests".to_string(), "==2.28.1".to_string()),
            ]
        );
    }

    #[test]
    fn parses_pep621_dependencies() {
        let toml = r#"
[project]
dependencies = ["requests>=2.28.0", "click==8.1.3"]
"#;
        let mut deps = parse_pyproject_toml(toml);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("click".to_string(), "==8.1.3".to_string()),
                ("requests".to_string(), ">=2.28.0".to_string()),
            ]
        );
    }

    #[test]
    fn parses_poetry_dependencies_and_skips_python() {
        let toml = r#"
[tool.poetry.dependencies]
python = "^3.10"
requests = "^2.28.0"
fastapi = { version = "^0.95", extras = ["all"] }
"#;
        let mut deps = parse_pyproject_toml(toml);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("fastapi".to_string(), "^0.95".to_string()),
                ("requests".to_string(), "^2.28.0".to_string()),
            ]
        );
    }

    #[test]
    fn parses_poetry_lock_installed_versions() {
        let lock = r#"
[[package]]
name = "requests"
version = "2.31.0"

[[package]]
name = "fastapi"
version = "0.95.2"
"#;
        let installed = parse_poetry_lock(lock);
        assert_eq!(installed.get("requests"), Some(&"2.31.0".to_string()));
        assert_eq!(installed.get("fastapi"), Some(&"0.95.2".to_string()));
    }

    #[test]
    fn malformed_poetry_lock_returns_empty_map() {
        assert!(parse_poetry_lock("not toml {{{").is_empty());
    }

    #[test]
    fn normalizes_names_per_pep_503() {
        assert_eq!(normalize_name("Typing_Extensions"), "typing-extensions");
        assert_eq!(normalize_name("zope.interface"), "zope-interface");
        assert_eq!(normalize_name("a--b__c..d"), "a-b-c-d");
    }

    #[test]
    fn poetry_lock_names_are_normalized() {
        let lock = "[[package]]\nname = \"Typing_Extensions\"\nversion = \"4.0.0\"\n";
        let m = parse_poetry_lock(lock);
        assert_eq!(m.get("typing-extensions"), Some(&"4.0.0".to_string()));
    }

    #[test]
    fn joins_hash_continuation_lines() {
        let content =
            "requests==2.28.1 \\\n    --hash=sha256:abc \\\n    --hash=sha256:def\nflask==2.0.0\n";
        assert_eq!(
            parse_requirements_txt(content),
            vec![
                ("requests".to_string(), "==2.28.1".to_string()),
                ("flask".to_string(), "==2.0.0".to_string())
            ]
        );
        // A dangling continuation at EOF is still flushed.
        assert_eq!(
            parse_requirements_txt("numpy==1.0 \\"),
            vec![("numpy".to_string(), "==1.0".to_string())]
        );
    }

    #[test]
    fn keeps_direct_url_requirements_as_non_registry_markers() {
        let deps = parse_requirements_txt("pkg @ https://example.com/pkg.whl\n");
        assert_eq!(
            deps,
            vec![(
                "pkg".to_string(),
                "@ https://example.com/pkg.whl".to_string()
            )]
        );
    }

    #[test]
    fn parses_optional_dependencies_groups_and_poetry_groups() {
        let toml = r#"
[project]
dependencies = ["requests>=2"]

[project.optional-dependencies]
dev = ["pytest>=7"]

[dependency-groups]
lint = ["ruff>=0.4", { include-group = "dev" }]

[tool.poetry.group.test.dependencies]
coverage = "^7.0"

[tool.poetry.dev-dependencies]
black = "^23"

[tool.poetry.dependencies]
internal = { path = "../internal" }
"#;
        let mut deps = parse_pyproject_toml(toml);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("black".to_string(), "^23".to_string()),
                ("coverage".to_string(), "^7.0".to_string()),
                ("internal".to_string(), "git+local".to_string()),
                ("pytest".to_string(), ">=7".to_string()),
                ("requests".to_string(), ">=2".to_string()),
                ("ruff".to_string(), ">=0.4".to_string()),
            ]
        );
    }

    #[test]
    fn malformed_pyproject_is_empty() {
        assert!(parse_pyproject_toml("not toml {{{").is_empty());
    }
}

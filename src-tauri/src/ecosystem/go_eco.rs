//! Extracts the dependencies declared in a `go.mod`.
//!
//! Go modules have no separate "declared range" grammar the way Cargo/npm
//! do: a `require` line pins a single minimum version, and Go's Minimal
//! Version Selection takes it (or a higher one pulled in transitively) from
//! there. There's also no universal per-module "installed version" lockfile
//! entry to read (`go.sum` lists every version ever verified, not the one
//! actually selected) — so `go.mod`'s own version is reported as both the
//! declared and, in effect, the installed version. `replace` directives are
//! honoured: a local replacement removes the module from the check, a module
//! replacement is reported under the fork that is really built.

pub fn parse_go_mod(content: &str) -> Vec<(String, String)> {
    let mut deps: Vec<(String, String)> = vec![];
    let mut replaces: Vec<(String, Option<(String, String)>)> = vec![];
    let mut block: Option<&str> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if let Some(kind) = block {
            if line == ")" {
                block = None;
            } else {
                handle_entry(kind, line, &mut deps, &mut replaces);
            }
            continue;
        }

        for kind in ["require", "replace"] {
            let Some(rest) = line.strip_prefix(kind) else {
                continue;
            };
            let rest = rest.trim();
            if rest == "(" {
                block = Some(kind);
            } else if line.starts_with(&format!("{kind} "))
                || line.starts_with(&format!("{kind}\t"))
            {
                handle_entry(kind, rest, &mut deps, &mut replaces);
            }
        }
    }

    apply_replaces(deps, &replaces)
}

fn handle_entry(
    kind: &str,
    line: &str,
    deps: &mut Vec<(String, String)>,
    replaces: &mut Vec<(String, Option<(String, String)>)>,
) {
    if kind == "require" {
        if let Some(dep) = parse_require_entry(line) {
            deps.push(dep);
        }
    } else if let Some(rep) = parse_replace_entry(line) {
        replaces.push(rep);
    }
}

/// Parses one `module/path version` entry. A trailing `// indirect` comment
/// (or anything else after the version) is simply not consumed, since only
/// the first two whitespace-separated tokens are read.
fn parse_require_entry(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let module = parts.next()?;
    let version = parts.next()?;
    Some((module.to_string(), version.to_string()))
}

/// Parses `old [version] => new [version]`. The replacement is `None` when
/// it points at a local directory (nothing published to check); otherwise
/// the replacement module and its version.
#[allow(clippy::type_complexity)]
fn parse_replace_entry(line: &str) -> Option<(String, Option<(String, String)>)> {
    let line = line.split("//").next().unwrap_or(line);
    let (left, right) = line.split_once("=>")?;
    let old = left.split_whitespace().next()?.to_string();
    let mut right = right.split_whitespace();
    let target = right.next()?;
    let is_local = target.starts_with('.') || target.starts_with('/');
    match (is_local, right.next()) {
        (false, Some(version)) => Some((old, Some((target.to_string(), version.to_string())))),
        _ => Some((old, None)),
    }
}

/// A `replace` swaps the module that is really built: a local replacement
/// drops the dependency from the check, a module replacement is reported
/// under the fork's own path and version.
#[allow(clippy::type_complexity)]
fn apply_replaces(
    deps: Vec<(String, String)>,
    replaces: &[(String, Option<(String, String)>)],
) -> Vec<(String, String)> {
    deps.into_iter()
        .filter_map(
            |(name, version)| match replaces.iter().find(|(old, _)| *old == name) {
                None => Some((name, version)),
                Some((_, Some(replacement))) => Some(replacement.clone()),
                Some((_, None)) => None,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_block_form_require_including_indirect() {
        let content = "\
module github.com/example/app

go 1.21

require (
\tgithub.com/spf13/cobra v1.8.0
\tgolang.org/x/sys v0.15.0 // indirect
)
";
        assert_eq!(
            parse_go_mod(content),
            vec![
                ("github.com/spf13/cobra".to_string(), "v1.8.0".to_string()),
                ("golang.org/x/sys".to_string(), "v0.15.0".to_string()),
            ]
        );
    }

    #[test]
    fn parses_single_line_require() {
        let content = "module github.com/example/app\n\nrequire github.com/single/dep v1.0.0\n";
        assert_eq!(
            parse_go_mod(content),
            vec![("github.com/single/dep".to_string(), "v1.0.0".to_string())]
        );
    }

    #[test]
    fn parses_multiple_require_blocks_and_ignores_other_directives() {
        let content = "\
module github.com/example/app

go 1.21

require (
\tgithub.com/a/a v1.0.0
)

replace github.com/a/a => ../a

require (
\tgithub.com/b/b v2.0.0
)
";
        // `replace github.com/a/a => ../a` points a/a at a local folder, so
        // there is no published version of it to check.
        assert_eq!(
            parse_go_mod(content),
            vec![("github.com/b/b".to_string(), "v2.0.0".to_string())]
        );
    }

    #[test]
    fn ignores_malformed_or_empty_content() {
        assert!(parse_go_mod("").is_empty());
        assert!(parse_go_mod("module github.com/example/app\n").is_empty());
    }

    #[test]
    fn handles_require_without_a_space_and_replace_directives() {
        let content = "\
module m

require(
\tgithub.com/a/a v1.0.0
\tgithub.com/b/b v2.0.0
\tgithub.com/c/c v3.0.0
\tgithub.com/d/d v4.0.0
)
require ()

replace (
\tgithub.com/a/a => ../a
\tgithub.com/b/b v2.0.0 => github.com/fork/b v2.0.1 // fork
)
replace github.com/c/c => github.com/other/c v3.1.0
";
        assert_eq!(
            parse_go_mod(content),
            vec![
                ("github.com/fork/b".to_string(), "v2.0.1".to_string()),
                ("github.com/other/c".to_string(), "v3.1.0".to_string()),
                ("github.com/d/d".to_string(), "v4.0.0".to_string()),
            ]
        );
    }

    #[test]
    fn replace_pointing_at_an_absolute_path_or_without_version_is_local() {
        let content = "require github.com/a/a v1.0.0\nreplace github.com/a/a => /abs/a\n";
        assert!(parse_go_mod(content).is_empty());
        let content = "require github.com/a/a v1.0.0\nreplace github.com/a/a => github.com/x/a\n";
        assert!(parse_go_mod(content).is_empty());
        assert!(parse_replace_entry("no arrow").is_none());
    }
}

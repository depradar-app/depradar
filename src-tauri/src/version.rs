//! Normalizes a declared version constraint (Cargo/npm/PyPI, whatever the
//! grammar: "^1.2.3", "~1.2", ">=2.0,<3.0", "1.2.3", "1.x"...) into a
//! comparable base version, and classifies a dependency against either its
//! actually-installed version (from a lockfile, when known) or the latest
//! published version — using each ecosystem's own range grammar to decide
//! real satisfaction rather than just comparing against the constraint's
//! floor version.

use crate::models::DepStatus;
use semver::Version;
use std::str::FromStr;

/// Which registry/manifest grammar a declared constraint should be
/// interpreted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    Rust,
    Npm,
    Python,
    Go,
    Php,
    Ruby,
    Java,
}

impl Ecosystem {
    /// The stable identifier sent to the frontend.
    pub fn as_str(self) -> &'static str {
        match self {
            Ecosystem::Rust => "rust",
            Ecosystem::Npm => "npm",
            Ecosystem::Python => "python",
            Ecosystem::Go => "go",
            Ecosystem::Php => "php",
            Ecosystem::Ruby => "ruby",
            Ecosystem::Java => "java",
        }
    }
}

/// True for a version string that carries a pre-release qualifier (`-rc.1`,
/// `1.0.0a1`, `6.0.0-M1`, `2.0.0.Beta3`, `1.0-SNAPSHOT`, `dev-main`...). Used
/// to avoid reporting a pre-release as the "latest" version of a package.
pub fn is_prerelease(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.starts_with("dev-") {
        return true;
    }
    lower.split(['.', '-', '_', '+']).any(|token| {
        let alpha_start = token.trim_start_matches(|c: char| c.is_ascii_digit());
        let word = alpha_start.trim_end_matches(|c: char| c.is_ascii_digit());
        let has_digits = word.len() != alpha_start.len();
        match word {
            "alpha" | "beta" | "rc" | "cr" | "dev" | "snapshot" | "preview" | "pre" | "ea"
            | "milestone" => true,
            "a" | "b" | "m" => has_digits,
            _ => false,
        }
    })
}

/// True when a declared constraint can't refer to a published registry
/// version at all (a local path, a git URL, a workspace link...), so asking a
/// public registry about the package name would be pointless — and would leak
/// a private name or surface a same-named public package by mistake.
pub fn is_non_registry(declared: &str) -> bool {
    let lower = declared.trim().to_ascii_lowercase();
    const PREFIXES: [&str; 16] = [
        "workspace:",
        "file:",
        "link:",
        "portal:",
        "patch:",
        "npm:",
        "git",
        "http",
        "github:",
        "gitlab:",
        "bitbucket:",
        "dev-",
        "./",
        "../",
        "/",
        "~/",
    ];
    lower.starts_with('@') || lower.contains("://") || PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// Rewrites a Composer constraint into the npm grammar `node-semver` speaks:
/// `|` is accepted as OR, a comma means AND, `~1.2` means `>=1.2 <2.0` (npm's
/// `~1.2` would mean `<1.3`), and `@stable`-style stability flags are dropped.
pub fn normalize_composer(declared: &str) -> String {
    declared
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|alt| {
            alt.replace(',', " ")
                .split_whitespace()
                .filter_map(|tok| {
                    let tok = tok.split('@').next().unwrap_or("");
                    if tok.is_empty() {
                        return None;
                    }
                    match tok.strip_prefix('~') {
                        Some(rest) if rest.matches('.').count() == 1 => Some(format!("^{rest}")),
                        _ => Some(tok.to_string()),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" || ")
}

fn numeric_parts(v: &str) -> Vec<u64> {
    v.split('.')
        .map_while(|p| p.trim().parse::<u64>().ok())
        .collect()
}

fn join_parts(parts: &[u64]) -> String {
    parts
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn python_caret(rest: &str) -> String {
    let parts = numeric_parts(rest);
    if parts.is_empty() {
        return format!("^{rest}");
    }
    // Poetry: bump the left-most non-zero component (or the last one given).
    let k = parts
        .iter()
        .position(|&p| p != 0)
        .unwrap_or(parts.len() - 1);
    let mut ceiling: Vec<u64> = parts[..k].to_vec();
    ceiling.push(parts[k] + 1);
    format!(">={},<{}", join_parts(&parts), join_parts(&ceiling))
}

fn python_tilde(rest: &str) -> String {
    let parts = numeric_parts(rest);
    if parts.is_empty() {
        return format!("~{rest}");
    }
    let ceiling = if parts.len() == 1 {
        vec![parts[0] + 1]
    } else {
        vec![parts[0], parts[1] + 1]
    };
    format!(">={},<{}", join_parts(&parts), join_parts(&ceiling))
}

/// Rewrites Poetry-only constraint syntax (`^1.2`, `~1.2`, a bare `1.2.3`)
/// into PEP 440 specifiers.
pub fn normalize_python(declared: &str) -> String {
    declared
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(|c| {
            if let Some(rest) = c.strip_prefix('^') {
                python_caret(rest.trim())
            } else if c.starts_with('~') && !c.starts_with("~=") {
                python_tilde(c[1..].trim())
            } else if c.starts_with(|ch: char| ch.is_ascii_digit()) {
                format!("=={c}")
            } else {
                c.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Extracts a usable base version from a raw constraint.
/// Returns `None` if the constraint references no concrete version at all
/// (e.g. "*", "workspace:*", a git URL, "file:../lib"...).
pub fn base_version(raw: &str) -> Option<Version> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let lower = raw.to_ascii_lowercase();
    if lower == "*"
        || lower == "latest"
        || lower.starts_with("workspace:")
        || lower.starts_with("file:")
        || lower.starts_with("link:")
        || lower.starts_with("git")
        || lower.starts_with("http")
        || lower.starts_with("npm:")
    {
        return None;
    }

    // Only keep the first segment of a composite constraint
    // ("1.2.3 - 2.0.0", ">=2.0,<3.0", "^1.0 || ^2.0"...). A token with no
    // digit at all — an operator written with a space before its version,
    // as RubyGems' "~> 7.0.4" or ">= 2.0, < 3.0" conventionally are — is
    // skipped in favor of the next one rather than failing outright.
    let first_token = raw
        .split([',', ' '])
        .find(|t| !t.is_empty() && t.chars().any(|c| c.is_ascii_digit()))?;

    let start = first_token.find(|c: char| c.is_ascii_digit())?;
    let numeric_part = &first_token[start..];

    let cleaned: String = numeric_part
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if cleaned.is_empty() {
        return None;
    }

    let mut components = cleaned
        .split('.')
        .map(|c| if c.is_empty() { "0" } else { c });
    let major = components.next().unwrap_or("0");
    let minor = components.next().unwrap_or("0");
    let patch = components.next().unwrap_or("0");

    Version::parse(&format!("{major}.{minor}.{patch}")).ok()
}

type Triple = (u64, u64, u64);

/// A major bump (or a minor bump within a 0.x line, considered breaking by
/// semver convention) between two concrete versions.
fn is_breaking_bump(current: Triple, latest: Triple) -> bool {
    latest.0 > current.0
        || (current.0 == 0 && latest.1 > current.1)
        // 0.0.x: every release is its own breaking line (`^0.0.1` == `=0.0.1`).
        || (current.0 == 0 && current.1 == 0 && latest.0 == 0 && latest.1 == 0 && latest.2 > current.2)
}

fn pep440_triple(v: &pep440_rs::Version) -> Triple {
    let release = v.release();
    (
        release.first().copied().unwrap_or(0),
        release.get(1).copied().unwrap_or(0),
        release.get(2).copied().unwrap_or(0),
    )
}

/// Parses `raw` as a concrete version in the given ecosystem's own grammar
/// and reduces it to a (major, minor, patch) triple for comparison. `None`
/// if it doesn't parse as a concrete version at all (a malformed registry
/// response, or a lockfile entry that isn't a plain version).
fn version_triple(ecosystem: Ecosystem, raw: &str) -> Option<Triple> {
    match ecosystem {
        Ecosystem::Rust => Version::parse(raw)
            .ok()
            .map(|v| (v.major, v.minor, v.patch)),
        Ecosystem::Npm => node_semver::Version::parse(raw)
            .ok()
            .map(|v| (v.major, v.minor, v.patch)),
        Ecosystem::Python => pep440_rs::Version::from_str(raw)
            .ok()
            .map(|v| pep440_triple(&v)),
        // Go module versions are valid semver with a mandatory "v" prefix.
        Ecosystem::Go => Version::parse(strip_v_prefix(raw))
            .ok()
            .map(|v| (v.major, v.minor, v.patch)),
        // Packagist releases are usually bare ("2.3.1") but some legacy
        // packages still tag with a "v" prefix; strip it defensively.
        Ecosystem::Php => node_semver::Version::parse(strip_v_prefix(raw))
            .ok()
            .map(|v| (v.major, v.minor, v.patch)),
        // No ecosystem-specific crate for RubyGems' or Maven's version
        // grammar — both are close enough to plain dotted-numeric that the
        // same lenient extraction used for a declared constraint's floor
        // version (below) works just as well on a concrete `latest`/
        // `installed` string (e.g. "1.15.5-x86_64-linux", "5.3.20.RELEASE").
        Ecosystem::Ruby | Ecosystem::Java => base_version(raw).map(|v| (v.major, v.minor, v.patch)),
    }
}

fn strip_v_prefix(raw: &str) -> &str {
    raw.strip_prefix('v').unwrap_or(raw)
}

/// Whether `declared`'s own range grammar admits `latest`. `None` means
/// `declared` didn't parse as a range in this ecosystem's grammar at all
/// (rare for a well-formed manifest) — callers should fall back to comparing
/// against the declared constraint's floor version instead.
fn range_admits_latest(ecosystem: Ecosystem, declared: &str, latest: &str) -> Option<bool> {
    match ecosystem {
        Ecosystem::Rust => {
            let req = semver::VersionReq::parse(declared).ok()?;
            let latest_v = Version::parse(latest).ok()?;
            Some(req.matches(&latest_v))
        }
        Ecosystem::Npm => {
            let range = node_semver::Range::parse(declared).ok()?;
            let latest_v = node_semver::Version::parse(latest).ok()?;
            Some(range.satisfies(&latest_v))
        }
        Ecosystem::Python => {
            let spec = pep440_rs::VersionSpecifiers::from_str(&normalize_python(declared)).ok()?;
            let latest_v = pep440_rs::Version::from_str(latest).ok()?;
            Some(spec.contains(&latest_v))
        }
        // go.mod `require` lines pin a single minimum version, not a range
        // — the floor-based comparison below (declared's own base version
        // vs. latest) already gives the right answer.
        Ecosystem::Go => None,
        Ecosystem::Php => {
            // Composer's constraint syntax is modeled closely enough on
            // npm's (caret/tilde/OR/hyphen ranges) that node-semver's
            // grammar is a good approximation; genuinely Composer-only
            // syntax just falls through to the floor-based comparison.
            let range = node_semver::Range::parse(normalize_composer(declared)).ok()?;
            let latest_v = node_semver::Version::parse(strip_v_prefix(latest)).ok()?;
            Some(range.satisfies(&latest_v))
        }
        Ecosystem::Ruby => ruby_range_admits_latest(declared, latest),
        // Maven dependency versions are exact pins in the overwhelmingly
        // common case; only the `[1.0,2.0)` interval syntax is a real range.
        Ecosystem::Java => maven_range_admits(declared, latest),
    }
}

/// Evaluates a Maven interval such as `[1.0,2.0)` or `(,1.5]`. `None` when
/// `declared` isn't an interval (a plain version is an exact/soft pin).
fn maven_range_admits(declared: &str, latest: &str) -> Option<bool> {
    let d = declared.trim();
    let open = d.chars().next()?;
    let close = d.chars().last()?;
    if !matches!(open, '[' | '(') || !matches!(close, ']' | ')') {
        return None;
    }
    let inner = &d[1..d.len() - 1];
    let latest_t = base_version(latest).map(|v| (v.major, v.minor, v.patch))?;
    let triple = |s: &str| base_version(s).map(|v| (v.major, v.minor, v.patch));
    match inner.split_once(',') {
        None => Some(triple(inner)? == latest_t),
        Some((lo, hi)) => {
            let (lo, hi) = (lo.trim(), hi.trim());
            let lo_ok = lo.is_empty()
                || triple(lo).is_some_and(|l| {
                    if open == '[' {
                        latest_t >= l
                    } else {
                        latest_t > l
                    }
                });
            let hi_ok = hi.is_empty()
                || triple(hi).is_some_and(|h| {
                    if close == ']' {
                        latest_t <= h
                    } else {
                        latest_t < h
                    }
                });
            Some(lo_ok && hi_ok)
        }
    }
}

/// RubyGems' constraint grammar: comma-separated requirements, each one of
/// `= != > >= < <= ~>` followed by a version. `~>` is the "pessimistic"
/// operator — it admits any release sharing the given prefix, up to (but
/// excluding) a bump of the segment just above the last one given: `~> 2.2`
/// behaves like `>=2.2, <3.0`; `~> 2.2.3` behaves like `>=2.2.3, <2.3.0`.
fn ruby_range_admits_latest(declared: &str, latest: &str) -> Option<bool> {
    let latest_triple = base_version(latest).map(|v| (v.major, v.minor, v.patch))?;

    let mut satisfied = true;
    let mut saw_any = false;
    for clause in declared.split(',') {
        let clause = clause.trim();
        if clause.is_empty() {
            continue;
        }
        satisfied &= ruby_clause_admits(clause, latest_triple)?;
        saw_any = true;
    }
    saw_any.then_some(satisfied)
}

fn ruby_clause_admits(clause: &str, latest: Triple) -> Option<bool> {
    let digit_start = clause.find(|c: char| c.is_ascii_digit())?;
    let op = clause[..digit_start].trim();
    let rest = clause[digit_start..].trim();
    let floor_version = base_version(rest)?;
    let floor = (
        floor_version.major,
        floor_version.minor,
        floor_version.patch,
    );

    match op {
        "" | "=" => Some(latest == floor),
        ">" => Some(latest > floor),
        ">=" => Some(latest >= floor),
        "<" => Some(latest < floor),
        "<=" => Some(latest <= floor),
        "~>" => {
            // Precision is inferred from how many dot-separated components
            // were actually given: "~> 2" / "~> 2.2" bump the major
            // version; "~> 2.2.3" (or more precise) bumps the minor.
            let given = rest.split('.').count();
            let ceiling = if given <= 2 {
                (floor.0 + 1, 0, 0)
            } else {
                (floor.0, floor.1 + 1, 0)
            };
            Some(latest >= floor && latest < ceiling)
        }
        _ => None,
    }
}

/// Classifies a dependency.
///
/// When `installed` (the version actually pinned in a lockfile) is known
/// and parses, it takes precedence over the declared constraint: a
/// developer running this tool wants to know whether what's really
/// installed is behind latest, not merely whether the declared range would
/// technically permit it. Otherwise, falls back to checking whether the
/// declared constraint's own range grammar already admits the latest
/// version.
pub fn classify(
    ecosystem: Ecosystem,
    declared: &str,
    installed: Option<&str>,
    latest: &str,
) -> DepStatus {
    let Some(latest_numeric) = version_triple(ecosystem, latest) else {
        return DepStatus::Unknown;
    };

    if let Some(installed_raw) = installed {
        if let Some(installed_numeric) = version_triple(ecosystem, installed_raw) {
            return if is_breaking_bump(installed_numeric, latest_numeric) {
                DepStatus::Major
            } else if latest_numeric > installed_numeric
                || (latest_numeric == installed_numeric
                    && is_prerelease(installed_raw)
                    && !is_prerelease(latest))
            {
                // A pre-release of X.Y.Z is behind the final X.Y.Z.
                DepStatus::Outdated
            } else {
                DepStatus::UpToDate
            };
        }
    }

    let Some(current) = base_version(declared) else {
        return DepStatus::Unknown;
    };
    let current_numeric = (current.major, current.minor, current.patch);

    // Check real range satisfaction before falling back to a floor-based
    // major-bump heuristic: an OR-range like "^1.0.0 || ^2.0.0" can already
    // admit a latest version that crosses what looks like a major-version
    // boundary from its first alternative's floor alone, and such a range
    // is exactly as "up to date" as any other satisfied range.
    match range_admits_latest(ecosystem, declared, latest) {
        Some(true) => DepStatus::UpToDate,
        Some(false) => {
            if is_breaking_bump(current_numeric, latest_numeric) {
                DepStatus::Major
            } else {
                DepStatus::Outdated
            }
        }
        None => {
            if is_breaking_bump(current_numeric, latest_numeric) {
                DepStatus::Major
            } else if latest_numeric > current_numeric {
                DepStatus::Outdated
            } else {
                DepStatus::UpToDate
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_common_prefixes() {
        assert_eq!(base_version("^1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(base_version("~1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(base_version("=1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(base_version(">=2.0.0").unwrap(), Version::new(2, 0, 0));
        assert_eq!(base_version("v1.2.3").unwrap(), Version::new(1, 2, 3));
    }

    #[test]
    fn pads_incomplete_versions() {
        assert_eq!(base_version("1.2").unwrap(), Version::new(1, 2, 0));
        assert_eq!(base_version("1").unwrap(), Version::new(1, 0, 0));
        assert_eq!(base_version("1.x").unwrap(), Version::new(1, 0, 0));
    }

    #[test]
    fn skips_a_leading_space_separated_operator_with_no_digits() {
        // RubyGems constraints conventionally put a space between the
        // operator and the version ("~> 7.0.4", ">= 2.0, < 3.0").
        assert_eq!(base_version("~> 7.0.4").unwrap(), Version::new(7, 0, 4));
        assert_eq!(
            base_version(">= 2.0, < 3.0").unwrap(),
            Version::new(2, 0, 0)
        );
    }

    #[test]
    fn takes_first_segment_of_composite_ranges() {
        assert_eq!(base_version(">=2.0,<3.0").unwrap(), Version::new(2, 0, 0));
        assert_eq!(
            base_version("1.2.3 - 2.3.4").unwrap(),
            Version::new(1, 2, 3)
        );
    }

    #[test]
    fn rejects_non_registry_constraints() {
        assert!(base_version("*").is_none());
        assert!(base_version("latest").is_none());
        assert!(base_version("workspace:*").is_none());
        assert!(base_version("file:../lib").is_none());
        assert!(base_version("git+https://github.com/a/b").is_none());
        assert!(base_version("").is_none());
    }

    #[test]
    fn classifies_up_to_date() {
        assert_eq!(
            classify(Ecosystem::Rust, "^1.2.3", None, "1.2.3"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Rust, "1.2.3", None, "1.2.3"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn caret_range_already_permitting_latest_is_up_to_date() {
        // Regression test for the v1 bug: a caret range that already admits
        // the latest release must not be reported as outdated just because
        // it isn't textually identical to it.
        assert_eq!(
            classify(Ecosystem::Rust, "^1.2.3", None, "1.4.0"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn range_that_genuinely_excludes_latest_is_outdated() {
        assert_eq!(
            classify(Ecosystem::Rust, "~1.2.3", None, "1.4.0"),
            DepStatus::Outdated
        );
        assert_eq!(
            classify(Ecosystem::Rust, "=1.2.3", None, "1.4.0"),
            DepStatus::Outdated
        );
    }

    #[test]
    fn classifies_major_bump_as_breaking() {
        assert_eq!(
            classify(Ecosystem::Rust, "^1.2.3", None, "2.0.0"),
            DepStatus::Major
        );
    }

    #[test]
    fn classifies_zero_x_minor_bump_as_breaking() {
        // In semver, 0.x -> 0.y (y>x) is considered a breaking change.
        assert_eq!(
            classify(Ecosystem::Rust, "^0.4.0", None, "0.5.0"),
            DepStatus::Major
        );
    }

    #[test]
    fn classifies_unparseable_constraint_as_unknown() {
        assert_eq!(
            classify(Ecosystem::Rust, "*", None, "1.0.0"),
            DepStatus::Unknown
        );
    }

    #[test]
    fn npm_caret_range_already_permitting_latest_is_up_to_date() {
        assert_eq!(
            classify(Ecosystem::Npm, "^1.2.0", None, "1.9.0"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn npm_tilde_range_excludes_minor_bump() {
        assert_eq!(
            classify(Ecosystem::Npm, "~1.2.0", None, "1.3.0"),
            DepStatus::Outdated
        );
    }

    #[test]
    fn npm_or_range_is_satisfied_by_either_side() {
        // "||" OR-ranges and hyphen ranges aren't valid `semver::VersionReq`
        // syntax; this exercises node-semver's own grammar.
        assert_eq!(
            classify(Ecosystem::Npm, "^1.0.0 || ^2.0.0", None, "2.5.0"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn npm_bare_version_is_an_exact_match_not_a_caret_range() {
        // Unlike Cargo, a bare npm version in a range context means exactly
        // that version, not a caret range.
        assert_eq!(
            classify(Ecosystem::Npm, "1.2.3", None, "1.4.0"),
            DepStatus::Outdated
        );
    }

    #[test]
    fn python_composite_range_is_evaluated_in_full() {
        assert_eq!(
            classify(Ecosystem::Python, ">=2.0,<3.0", None, "2.5.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Python, ">=2.0,<3.0", None, "3.1.0"),
            DepStatus::Major
        );
    }

    #[test]
    fn python_non_strict_semver_latest_no_longer_degrades_to_unknown() {
        // PyPI can publish post-release/local versions that aren't strict
        // semver; PEP 440 parsing must still classify them.
        assert_eq!(
            classify(Ecosystem::Python, ">=2.0", None, "2.0.1.post1"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn installed_version_takes_precedence_over_declared_range() {
        // The declared range would technically already permit 1.9.0, but
        // what's actually pinned in the lockfile is what matters.
        assert_eq!(
            classify(Ecosystem::Rust, "^1.0.0", Some("1.2.3"), "1.9.0"),
            DepStatus::Outdated
        );
    }

    #[test]
    fn installed_version_up_to_date() {
        assert_eq!(
            classify(Ecosystem::Rust, "^1.0.0", Some("1.9.0"), "1.9.0"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn installed_version_major_bump() {
        assert_eq!(
            classify(Ecosystem::Rust, "^1.0.0", Some("1.2.3"), "2.0.0"),
            DepStatus::Major
        );
    }

    #[test]
    fn falls_back_to_declared_range_when_installed_is_unparseable() {
        assert_eq!(
            classify(Ecosystem::Rust, "^1.2.0", Some("not-a-version"), "1.2.3"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn go_pinned_version_up_to_date_and_outdated() {
        assert_eq!(
            classify(Ecosystem::Go, "v1.8.0", None, "v1.8.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Go, "v1.8.0", None, "v1.9.0"),
            DepStatus::Outdated
        );
        assert_eq!(
            classify(Ecosystem::Go, "v1.8.0", None, "v2.0.0"),
            DepStatus::Major
        );
    }

    #[test]
    fn php_caret_range_already_permitting_latest_is_up_to_date() {
        assert_eq!(
            classify(Ecosystem::Php, "^3.0", None, "3.5.0"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn php_tilde_range_excludes_minor_bump() {
        assert_eq!(
            classify(Ecosystem::Php, "~3.0.0", None, "3.1.0"),
            DepStatus::Outdated
        );
    }

    #[test]
    fn ruby_pessimistic_operator_three_segments_bumps_minor() {
        assert_eq!(
            classify(Ecosystem::Ruby, "~> 7.0.4", None, "7.0.9"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Ruby, "~> 7.0.4", None, "7.1.0"),
            DepStatus::Outdated
        );
    }

    #[test]
    fn ruby_pessimistic_operator_two_segments_bumps_major() {
        assert_eq!(
            classify(Ecosystem::Ruby, "~> 7.0", None, "7.9.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Ruby, "~> 7.0", None, "8.0.0"),
            DepStatus::Major
        );
    }

    #[test]
    fn ruby_comparison_operators_and_and_lists() {
        assert_eq!(
            classify(Ecosystem::Ruby, ">= 2.0, < 3.0", None, "2.9.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Ruby, ">= 2.0, < 3.0", None, "3.0.0"),
            DepStatus::Major
        );
    }

    #[test]
    fn ruby_unparseable_constraint_falls_back_to_floor_comparison() {
        assert_eq!(
            classify(Ecosystem::Ruby, "!= 1.0.0", None, "1.0.0"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn java_pinned_version_up_to_date_and_outdated() {
        assert_eq!(
            classify(Ecosystem::Java, "4.13.2", None, "4.13.2"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Java, "5.3.20", None, "5.3.21"),
            DepStatus::Outdated
        );
        assert_eq!(
            classify(Ecosystem::Java, "1.2.3", None, "2.0.0"),
            DepStatus::Major
        );
    }

    #[test]
    fn java_non_strict_semver_latest_is_still_classified() {
        // Maven releases are sometimes qualified ("5.3.20.RELEASE") rather
        // than strict semver.
        assert_eq!(
            classify(Ecosystem::Java, "5.3.19", None, "5.3.20.RELEASE"),
            DepStatus::Outdated
        );
    }

    #[test]
    fn ecosystem_ids_are_stable() {
        let all = [
            (Ecosystem::Rust, "rust"),
            (Ecosystem::Npm, "npm"),
            (Ecosystem::Python, "python"),
            (Ecosystem::Go, "go"),
            (Ecosystem::Php, "php"),
            (Ecosystem::Ruby, "ruby"),
            (Ecosystem::Java, "java"),
        ];
        for (eco, id) in all {
            assert_eq!(eco.as_str(), id);
        }
    }

    #[test]
    fn detects_prerelease_qualifiers() {
        for v in [
            "6.0.0-M1",
            "5.13.0-M3",
            "7.1.0-RC1",
            "1.0.0-rc.1",
            "2.0.0a1",
            "2.0.0.dev3",
            "1.0-SNAPSHOT",
            "3.0.0.Beta2",
            "dev-main",
            "1.0.0-alpha",
            "1.0.0-preview.2",
        ] {
            assert!(is_prerelease(v), "{v} should be a pre-release");
        }
        for v in [
            "1.0.0",
            "5.3.20.RELEASE",
            "2.0.1.post1",
            "1.15.5-x86_64-linux",
            "33.0.0-jre",
            "v1.2.3",
            "1.0.0+build5",
        ] {
            assert!(!is_prerelease(v), "{v} should be stable");
        }
    }

    #[test]
    fn detects_non_registry_constraints() {
        for v in [
            "workspace:*",
            "file:../lib",
            "link:../x",
            "git+https://x/y",
            "github:a/b",
            "https://x/y.tgz",
            "@ https://x/y.whl",
            "../local",
            "dev-main",
            "npm:foo@1",
        ] {
            assert!(is_non_registry(v), "{v}");
        }
        for v in ["^1.2.3", "*", "", "latest", "~> 7.0", ">=2,<3", "1.0"] {
            assert!(!is_non_registry(v), "{v}");
        }
    }

    #[test]
    fn composer_constraints_are_normalized() {
        assert_eq!(normalize_composer("^7.4|^8.0"), "^7.4 || ^8.0");
        assert_eq!(normalize_composer("^7.4 || ^8.0"), "^7.4 || ^8.0");
        assert_eq!(normalize_composer("~3.0"), "^3.0");
        assert_eq!(normalize_composer("~3.0.1"), "~3.0.1");
        assert_eq!(normalize_composer(">=1.0,<2.0"), ">=1.0 <2.0");
        assert_eq!(normalize_composer("^1.0@dev"), "^1.0");
        assert_eq!(normalize_composer("@dev"), "");
    }

    #[test]
    fn composer_tilde_and_pipe_are_classified_correctly() {
        assert_eq!(
            classify(Ecosystem::Php, "~3.0", None, "3.5.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Php, "^7.4|^8.0", None, "8.3.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Php, "~3.0.1", None, "3.1.0"),
            DepStatus::Outdated
        );
    }

    #[test]
    fn python_poetry_constraints_are_normalized() {
        assert_eq!(normalize_python("^2.28.0"), ">=2.28.0,<3");
        assert_eq!(normalize_python("^0.2.3"), ">=0.2.3,<0.3");
        assert_eq!(normalize_python("^0.0.3"), ">=0.0.3,<0.0.4");
        assert_eq!(normalize_python("^0"), ">=0,<1");
        assert_eq!(normalize_python("^0.0"), ">=0.0,<0.1");
        assert_eq!(normalize_python("~2.28"), ">=2.28,<2.29");
        assert_eq!(normalize_python("~2"), ">=2,<3");
        assert_eq!(normalize_python("~=2.28"), "~=2.28");
        assert_eq!(normalize_python("2.28.1"), "==2.28.1");
        assert_eq!(normalize_python(">=1, <2"), ">=1,<2");
        assert_eq!(normalize_python("^x"), "^x");
        assert_eq!(normalize_python("~x"), "~x");
    }

    #[test]
    fn poetry_caret_and_tilde_are_classified_correctly() {
        assert_eq!(
            classify(Ecosystem::Python, "^2.28.0", None, "2.32.3"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Python, "~2.28", None, "2.28.5"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Python, "~2.28", None, "2.29.0"),
            DepStatus::Outdated
        );
        assert_eq!(
            classify(Ecosystem::Python, "^2.28.0", None, "3.0.0"),
            DepStatus::Major
        );
    }

    #[test]
    fn prerelease_installed_is_behind_the_final_release() {
        assert_eq!(
            classify(
                Ecosystem::Npm,
                "^2.0.0-beta.1",
                Some("2.0.0-beta.1"),
                "2.0.0"
            ),
            DepStatus::Outdated
        );
        assert_eq!(
            classify(Ecosystem::Npm, "^2.0.0", Some("2.0.0"), "2.0.0"),
            DepStatus::UpToDate
        );
    }

    #[test]
    fn zero_zero_x_patch_bump_is_breaking() {
        assert_eq!(
            classify(Ecosystem::Rust, "0.0.1", Some("0.0.1"), "0.0.2"),
            DepStatus::Major
        );
    }

    #[test]
    fn maven_intervals_are_evaluated() {
        assert_eq!(
            classify(Ecosystem::Java, "[1.0,2.0)", None, "1.5.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Java, "[1.0,2.0)", None, "2.0.0"),
            DepStatus::Major
        );
        assert_eq!(
            classify(Ecosystem::Java, "[1.0,2.0]", None, "2.0.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Java, "(1.0,)", None, "1.2.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Java, "[1.5]", None, "1.5.0"),
            DepStatus::UpToDate
        );
        assert_eq!(
            classify(Ecosystem::Java, "[1.5]", None, "1.6.0"),
            DepStatus::Outdated
        );
        assert_eq!(
            classify(Ecosystem::Java, "(,1.5]", None, "1.2.0"),
            DepStatus::UpToDate
        );
    }
}

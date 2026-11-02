//! Extracts the dependencies declared in a `Gemfile`, and installed
//! versions from a `Gemfile.lock`.

use std::collections::HashMap;

pub fn parse_gemfile(content: &str) -> Vec<(String, String)> {
    let mut deps = vec![];
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(dep) = parse_gem_line(line) {
            deps.push(dep);
        }
    }
    deps
}

/// Parses a `gem "name", "constraint", other: options` (or `gem(...)`,
/// single-quoted, constraint-less, or with trailing keyword args) line.
fn parse_gem_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("gem")?;
    let rest = rest
        .trim_start()
        .strip_prefix('(')
        .unwrap_or(rest)
        .trim_start();
    let (name, rest) = extract_quoted(rest)?;
    if name.is_empty() {
        return None;
    }

    // A gem sourced from git/path/GitHub is not a registry release.
    if rest.contains("git:")
        || rest.contains("github:")
        || rest.contains("path:")
        || rest.contains("git =>")
        || rest.contains("path =>")
        || rest.contains("github =>")
    {
        return None;
    }

    // Every quoted string that directly follows the name (only commas and
    // whitespace in between) is a version constraint: `gem 'a', '>= 1', '< 2'`.
    // Anything else there — `require: false`, `group: :test`, end of line —
    // ends the list.
    let mut constraints: Vec<&str> = vec![];
    let mut cursor = rest;
    loop {
        let after_comma = cursor
            .trim_start()
            .strip_prefix(',')
            .unwrap_or(cursor)
            .trim_start();
        if !after_comma.starts_with(['\'', '"']) {
            break;
        }
        let Some((value, remaining)) = extract_quoted(after_comma) else {
            break;
        };
        constraints.push(value);
        cursor = remaining;
    }

    Some((name.to_string(), constraints.join(", ")))
}

/// Finds the first quoted string in `s` (single or double quotes — no
/// escape-sequence handling needed for gem/version literals), returning its
/// content and the remainder of `s` after the closing quote.
fn extract_quoted(s: &str) -> Option<(&str, &str)> {
    let start = s.find(['\'', '"'])?;
    let quote = s.as_bytes()[start] as char;
    let after_open = &s[start + 1..];
    let end = after_open.find(quote)?;
    Some((&after_open[..end], &after_open[end + 1..]))
}

/// Extracts installed versions from a `Gemfile.lock`'s `specs:` sections.
/// Bundler resolves exactly one version per gem name across the whole
/// dependency graph (unlike npm, there's no per-scope duplicate
/// installation), so every concrete `name (version)` line found — top-level
/// spec or a note attached under someone else's spec — maps to the one true
/// installed version for that name; only section headers (`GEM`,
/// `DEPENDENCIES`, ...) sit at column 0 and are skipped.
pub fn parse_gemfile_lock(content: &str) -> HashMap<String, String> {
    let mut installed = HashMap::new();
    for raw_line in content.lines() {
        if !raw_line.starts_with(' ') {
            continue;
        }
        if let Some((name, version)) = parse_spec_line(raw_line.trim()) {
            installed.insert(name, version);
        }
    }
    installed
}

/// Parses a `name (version)` line, e.g. `rails (7.0.4)` or, for a
/// platform-specific gem, `nokogiri (1.15.5-x86_64-linux)`. A line that
/// carries a requirement instead of a resolved version (e.g. `activesupport
/// (= 7.0.4)`, a dependency note nested under another spec, or `rails (~>
/// 7.0.4)` under `DEPENDENCIES`) isn't a concrete version and is skipped —
/// the same gem's real resolved version is always also listed at its own
/// top-level spec line elsewhere in the file.
fn parse_spec_line(line: &str) -> Option<(String, String)> {
    let open = line.find('(')?;
    let close = open + line[open..].find(')')?;
    let name = line[..open].trim();
    let version = &line[open + 1..close];
    if name.is_empty() || version.is_empty() || version.starts_with(['=', '<', '>', '~', '!']) {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gem_with_pessimistic_constraint() {
        assert_eq!(
            parse_gemfile("gem \"rails\", \"~> 7.0.4\"\n"),
            vec![("rails".to_string(), "~> 7.0.4".to_string())]
        );
    }

    #[test]
    fn parses_gem_without_a_constraint() {
        assert_eq!(
            parse_gemfile("gem 'devise'\n"),
            vec![("devise".to_string(), "".to_string())]
        );
    }

    #[test]
    fn parses_gem_with_trailing_keyword_args() {
        assert_eq!(
            parse_gemfile("gem \"pg\", \"~> 1.1\", group: :production\n"),
            vec![("pg".to_string(), "~> 1.1".to_string())]
        );
    }

    #[test]
    fn keeps_every_constraint_of_a_gem() {
        assert_eq!(
            parse_gemfile("gem 'rails', '>= 6', '< 8', require: false\n"),
            vec![("rails".to_string(), ">= 6, < 8".to_string())]
        );
    }

    #[test]
    fn skips_gems_from_git_or_path() {
        let content = "\
gem 'a', git: 'https://github.com/x/a'
gem 'b', github: 'x/b'
gem 'c', path: '../c'
gem 'd', :git => 'https://x/d'
gem 'e', '~> 1.0'
";
        assert_eq!(
            parse_gemfile(content),
            vec![("e".to_string(), "~> 1.0".to_string())]
        );
    }

    #[test]
    fn keyword_arg_alone_is_not_mistaken_for_a_version() {
        assert_eq!(
            parse_gemfile("gem \"foo\", require: false\n"),
            vec![("foo".to_string(), "".to_string())]
        );
    }

    #[test]
    fn ignores_comments_and_non_gem_lines() {
        let content = "\
source \"https://rubygems.org\"
# a comment
gemspec
group :test do
gem \"rspec\", \"~> 3.12\"
end
";
        assert_eq!(
            parse_gemfile(content),
            vec![("rspec".to_string(), "~> 3.12".to_string())]
        );
    }

    #[test]
    fn parses_gemfile_lock_specs() {
        let lock = "\
GEM
  remote: https://rubygems.org/
  specs:
    actionpack (7.0.4)
      actionview (= 7.0.4)
      activesupport (= 7.0.4)
    activesupport (7.0.4)
    nokogiri (1.15.5-x86_64-linux)
      racc (~> 1.4)
    rails (7.0.4)

PLATFORMS
  x86_64-linux

DEPENDENCIES
  nokogiri
  rails (~> 7.0.4)

BUNDLED WITH
   2.3.7
";
        let installed = parse_gemfile_lock(lock);
        assert_eq!(installed.get("actionpack"), Some(&"7.0.4".to_string()));
        assert_eq!(installed.get("activesupport"), Some(&"7.0.4".to_string()));
        assert_eq!(
            installed.get("nokogiri"),
            Some(&"1.15.5-x86_64-linux".to_string())
        );
        assert_eq!(installed.get("rails"), Some(&"7.0.4".to_string()));
        // `racc (~> 1.4)` is a requirement note, not racc's own resolved
        // spec (which isn't in this trimmed fixture) — must not be picked up.
        assert!(!installed.contains_key("racc"));
    }

    #[test]
    fn empty_lockfile_returns_empty_map() {
        assert!(parse_gemfile_lock("").is_empty());
    }
}

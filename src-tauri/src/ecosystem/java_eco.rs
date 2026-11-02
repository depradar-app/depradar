//! Extracts the dependencies declared in Maven `pom.xml` files, Gradle build
//! scripts (literal coordinates only) and Gradle version catalogs
//! (`libs.versions.toml`).
//!
//! A pom's `${...}` placeholders and missing `<version>`s are resolved against
//! the pom's own `<properties>` / `<dependencyManagement>` plus whatever its
//! parent chain (`<parent>`, resolved by the caller) and imported BOMs
//! contribute — see [`PomContext`]. A dependency whose version still can't be
//! resolved is skipped, the same as an unparseable version anywhere else in
//! this app. Gradle scripts are arbitrary Groovy/Kotlin: only dependencies
//! written with a literal `group:artifact:version` (or the map notation) are
//! recognised, never computed ones.

use roxmltree::{Document, Node};
use std::collections::HashMap;

/// What a pom inherits from its ancestors and imported BOMs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PomContext {
    pub properties: HashMap<String, String>,
    /// `"groupId:artifactId"` → managed version.
    pub managed: HashMap<String, String>,
}

/// A `<parent>` reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PomParent {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    /// Path to the parent pom relative to this pom's folder.
    pub relative_path: String,
}

pub fn pom_parent(content: &str) -> Option<PomParent> {
    let doc = Document::parse(content).ok()?;
    let parent = child(&doc.root_element(), "parent")?;
    let relative_path = child_text(&parent, "relativePath").unwrap_or_else(|| "../pom.xml".into());
    Some(PomParent {
        group_id: child_text(&parent, "groupId")?,
        artifact_id: child_text(&parent, "artifactId")?,
        version: child_text(&parent, "version")?,
        relative_path,
    })
}

/// The context seen by this pom: `inherited` overlaid with the pom's own
/// properties and `<dependencyManagement>` versions.
pub fn pom_context(content: &str, inherited: &PomContext) -> PomContext {
    let Ok(doc) = Document::parse(content) else {
        return inherited.clone();
    };
    let root = doc.root_element();
    let mut ctx = inherited.clone();

    if let Some(parent_version) = child(&root, "parent").and_then(|p| child_text(&p, "version")) {
        ctx.properties
            .insert("project.parent.version".into(), parent_version.clone());
        ctx.properties
            .entry("project.version".into())
            .or_insert_with(|| parent_version.clone());
        ctx.properties
            .entry("pom.version".into())
            .or_insert(parent_version);
    }
    if let Some(node) = child(&root, "properties") {
        for prop in node.children().filter(|n| n.is_element()) {
            if let Some(text) = prop.text() {
                ctx.properties
                    .insert(prop.tag_name().name().to_string(), text.trim().to_string());
            }
        }
    }
    if let Some(version) = child_text(&root, "version") {
        ctx.properties
            .insert("project.version".into(), version.clone());
        ctx.properties.insert("pom.version".into(), version);
    }

    for (key, version, scope, _) in managed_entries(&root) {
        if scope.as_deref() == Some("import") {
            continue;
        }
        if let Some(resolved) = resolve_properties(&version, &ctx.properties) {
            ctx.managed.insert(key, resolved);
        }
    }
    ctx
}

/// `(groupId, artifactId, version)` of every BOM imported through
/// `<dependencyManagement>` (`<scope>import</scope>`).
pub fn pom_bom_imports(content: &str, ctx: &PomContext) -> Vec<(String, String, String)> {
    let Ok(doc) = Document::parse(content) else {
        return vec![];
    };
    managed_entries(&doc.root_element())
        .into_iter()
        .filter(|(_, _, scope, _)| scope.as_deref() == Some("import"))
        .filter_map(|(key, version, _, _)| {
            let (g, a) = key.split_once(':')?;
            let v = resolve_properties(&version, &ctx.properties)?;
            Some((g.to_string(), a.to_string(), v))
        })
        .collect()
}

/// (`g:a`, raw version, scope, type) of each `<dependencyManagement>` entry.
#[allow(clippy::type_complexity)]
fn managed_entries(root: &Node) -> Vec<(String, String, Option<String>, Option<String>)> {
    let Some(deps) = child(root, "dependencyManagement").and_then(|m| child(&m, "dependencies"))
    else {
        return vec![];
    };
    deps.children()
        .filter(|n| n.has_tag_name("dependency"))
        .filter_map(|d| {
            let g = child_text(&d, "groupId")?;
            let a = child_text(&d, "artifactId")?;
            let v = child_text(&d, "version")?;
            Some((
                format!("{g}:{a}"),
                v,
                child_text(&d, "scope"),
                child_text(&d, "type"),
            ))
        })
        .collect()
}

pub fn parse_pom_xml(content: &str, inherited: &PomContext) -> Vec<(String, String)> {
    let Ok(doc) = Document::parse(content) else {
        return vec![];
    };
    let root = doc.root_element();
    let ctx = pom_context(content, inherited);

    let Some(dependencies) = child(&root, "dependencies") else {
        return vec![];
    };

    let mut deps = vec![];
    for dependency in dependencies
        .children()
        .filter(|n| n.has_tag_name("dependency"))
    {
        let (Some(group_id), Some(artifact_id)) = (
            child_text(&dependency, "groupId"),
            child_text(&dependency, "artifactId"),
        ) else {
            continue;
        };
        let key = format!("{group_id}:{artifact_id}");

        let version = match child_text(&dependency, "version") {
            Some(raw) => resolve_properties(&raw, &ctx.properties),
            // No explicit version: managed by this pom, a parent or a BOM.
            None => ctx.managed.get(&key).cloned(),
        };
        let Some(version) = version else {
            continue; // unresolved ${...} placeholder / nothing manages it
        };

        deps.push((key, version));
    }
    deps
}

fn child<'a, 'input>(node: &Node<'a, 'input>, tag: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|n| n.has_tag_name(tag))
}

fn child_text(node: &Node, tag: &str) -> Option<String> {
    child(node, tag)?.text().map(|t| t.trim().to_string())
}

/// Substitutes every `${prop}` placeholder in `raw` from `properties`
/// (properties may themselves reference other properties). Returns `None` if
/// any placeholder can't be resolved, rather than reporting a literal
/// `"${...}"` string as a version.
fn resolve_properties(raw: &str, properties: &HashMap<String, String>) -> Option<String> {
    let mut current = raw.to_string();
    for _ in 0..8 {
        if !current.contains("${") {
            return Some(current);
        }
        let mut out = String::with_capacity(current.len());
        let mut rest = current.as_str();
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after.find('}')?;
            out.push_str(properties.get(&after[..end])?);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        current = out;
    }
    (!current.contains("${")).then_some(current)
}

// ------------------------------------------------------------------- Gradle

const GRADLE_CONFIGS: [&str; 14] = [
    "implementation",
    "api",
    "compileonly",
    "runtimeonly",
    "compile",
    "runtime",
    "testimplementation",
    "testcompile",
    "kapt",
    "annotationprocessor",
    "classpath",
    "developmentonly",
    "debugimplementation",
    "ksp",
];

fn quoted_strings(line: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut rest = line;
    while let Some(start) = rest.find(['\'', '"']) {
        let quote = rest.as_bytes()[start] as char;
        let after = &rest[start + 1..];
        let Some(end) = after.find(quote) else {
            break;
        };
        out.push(&after[..end]);
        rest = &after[end + 1..];
    }
    out
}

/// Value of `key: 'v'` / `key = "v"` in a Gradle map-notation dependency.
fn named_arg(line: &str, key: &str) -> Option<String> {
    let at = line
        .find(&format!("{key}:"))
        .or_else(|| line.find(&format!("{key} =")))?;
    quoted_strings(&line[at..]).first().map(|s| s.to_string())
}

fn usable_version(v: &str) -> bool {
    v.starts_with(|c: char| c.is_ascii_digit()) && !v.contains(['$', '{'])
}

/// Dependencies declared with literal coordinates in a `build.gradle(.kts)`.
pub fn parse_gradle(content: &str) -> Vec<(String, String)> {
    let mut deps = vec![];
    for raw in content.lines() {
        let line = raw.split("//").next().unwrap_or(raw).trim();
        let lower = line.to_ascii_lowercase();
        let first_word: String = lower
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if !GRADLE_CONFIGS.contains(&first_word.as_str()) {
            continue;
        }

        let coordinate = quoted_strings(line)
            .into_iter()
            .map(|s| s.split('@').next().unwrap_or(s))
            .find(|s| s.split(':').count() >= 3 && !s.contains(' '));
        if let Some(coord) = coordinate {
            let mut parts = coord.split(':');
            let (group, artifact, version) = (parts.next(), parts.next(), parts.next());
            if let (Some(g), Some(a), Some(v)) = (group, artifact, version) {
                if usable_version(v) && !g.is_empty() && !a.is_empty() {
                    deps.push((format!("{g}:{a}"), v.to_string()));
                }
            }
            continue;
        }

        if let (Some(g), Some(a), Some(v)) = (
            named_arg(line, "group"),
            named_arg(line, "name"),
            named_arg(line, "version"),
        ) {
            if usable_version(&v) {
                deps.push((format!("{g}:{a}"), v));
            }
        }
    }
    deps
}

/// Dependencies of a Gradle version catalog (`gradle/libs.versions.toml`).
pub fn parse_version_catalog(content: &str) -> Vec<(String, String)> {
    let Ok(doc) = content.parse::<toml::Value>() else {
        return vec![];
    };
    let versions = doc.get("versions").and_then(|v| v.as_table());

    let resolve_version = |value: &toml::Value| -> Option<String> {
        match value {
            toml::Value::String(s) => Some(s.clone()),
            toml::Value::Table(t) => {
                if let Some(r) = t.get("ref").and_then(|r| r.as_str()) {
                    return match versions?.get(r)? {
                        toml::Value::String(s) => Some(s.clone()),
                        toml::Value::Table(rich) => ["strictly", "require", "prefer"]
                            .iter()
                            .find_map(|k| rich.get(*k).and_then(|v| v.as_str()))
                            .map(String::from),
                        _ => None,
                    };
                }
                ["strictly", "require", "prefer"]
                    .iter()
                    .find_map(|k| t.get(*k).and_then(|v| v.as_str()))
                    .map(String::from)
            }
            _ => None,
        }
    };

    let mut deps = vec![];
    let Some(libs) = doc.get("libraries").and_then(|l| l.as_table()) else {
        return deps;
    };
    for lib in libs.values() {
        match lib {
            toml::Value::String(coord) => {
                let mut p = coord.split(':');
                if let (Some(g), Some(a), Some(v)) = (p.next(), p.next(), p.next()) {
                    deps.push((format!("{g}:{a}"), v.to_string()));
                }
            }
            toml::Value::Table(t) => {
                let module = t
                    .get("module")
                    .and_then(|m| m.as_str())
                    .map(String::from)
                    .or_else(|| {
                        let g = t.get("group")?.as_str()?;
                        let n = t.get("name")?.as_str()?;
                        Some(format!("{g}:{n}"))
                    });
                if let (Some(module), Some(version)) =
                    (module, t.get("version").and_then(resolve_version))
                {
                    deps.push((module, version));
                }
            }
            _ => {}
        }
    }
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_dependencies_and_ignores_scope() {
        let xml = r#"<project>
            <dependencies>
                <dependency>
                    <groupId>junit</groupId>
                    <artifactId>junit</artifactId>
                    <version>4.13.2</version>
                    <scope>test</scope>
                </dependency>
            </dependencies>
        </project>"#;
        assert_eq!(
            parse_pom_xml(xml, &PomContext::default()),
            vec![("junit:junit".to_string(), "4.13.2".to_string())]
        );
    }

    #[test]
    fn matches_tags_regardless_of_the_default_maven_namespace() {
        let xml = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
            <dependencies>
                <dependency>
                    <groupId>junit</groupId>
                    <artifactId>junit</artifactId>
                    <version>4.13.2</version>
                </dependency>
            </dependencies>
        </project>"#;
        assert_eq!(
            parse_pom_xml(xml, &PomContext::default()),
            vec![("junit:junit".to_string(), "4.13.2".to_string())]
        );
    }

    #[test]
    fn resolves_property_placeholders() {
        let xml = r#"<project>
            <properties>
                <spring.version>5.3.20</spring.version>
            </properties>
            <dependencies>
                <dependency>
                    <groupId>org.springframework</groupId>
                    <artifactId>spring-core</artifactId>
                    <version>${spring.version}</version>
                </dependency>
            </dependencies>
        </project>"#;
        assert_eq!(
            parse_pom_xml(xml, &PomContext::default()),
            vec![(
                "org.springframework:spring-core".to_string(),
                "5.3.20".to_string()
            )]
        );
    }

    #[test]
    fn resolves_self_referencing_project_version() {
        let xml = r#"<project>
            <version>2.0.0</version>
            <dependencies>
                <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>sibling-module</artifactId>
                    <version>${project.version}</version>
                </dependency>
            </dependencies>
        </project>"#;
        assert_eq!(
            parse_pom_xml(xml, &PomContext::default()),
            vec![(
                "com.example:sibling-module".to_string(),
                "2.0.0".to_string()
            )]
        );
    }

    #[test]
    fn skips_dependency_with_unresolvable_property() {
        let xml = r#"<project>
            <dependencies>
                <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>inherited</artifactId>
                    <version>${parent.defined.elsewhere}</version>
                </dependency>
            </dependencies>
        </project>"#;
        assert!(parse_pom_xml(xml, &PomContext::default()).is_empty());
    }

    #[test]
    fn skips_dependency_with_no_explicit_version() {
        let xml = r#"<project>
            <dependencies>
                <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>managed-elsewhere</artifactId>
                </dependency>
            </dependencies>
        </project>"#;
        assert!(parse_pom_xml(xml, &PomContext::default()).is_empty());
    }

    #[test]
    fn ignores_dependency_management_block() {
        let xml = r#"<project>
            <dependencyManagement>
                <dependencies>
                    <dependency>
                        <groupId>com.example</groupId>
                        <artifactId>bom-only</artifactId>
                        <version>1.0.0</version>
                    </dependency>
                </dependencies>
            </dependencyManagement>
            <dependencies>
                <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>real-dep</artifactId>
                    <version>3.0.0</version>
                </dependency>
            </dependencies>
        </project>"#;
        assert_eq!(
            parse_pom_xml(xml, &PomContext::default()),
            vec![("com.example:real-dep".to_string(), "3.0.0".to_string())]
        );
    }

    #[test]
    fn malformed_xml_returns_empty() {
        assert!(parse_pom_xml("not xml <<<", &PomContext::default()).is_empty());
    }

    #[test]
    fn resolves_versions_through_parent_chain_and_dependency_management() {
        let parent = r#"<project>
            <version>1.0</version>
            <properties><junit.version>4.13.2</junit.version></properties>
            <dependencyManagement><dependencies>
                <dependency><groupId>junit</groupId><artifactId>junit</artifactId><version>${junit.version}</version></dependency>
                <dependency><groupId>org.bom</groupId><artifactId>bom</artifactId><version>2.0</version><scope>import</scope><type>pom</type></dependency>
            </dependencies></dependencyManagement>
        </project>"#;
        let child_pom = r#"<project>
            <parent><groupId>g</groupId><artifactId>p</artifactId><version>1.0</version><relativePath>../pom.xml</relativePath></parent>
            <dependencies>
                <dependency><groupId>junit</groupId><artifactId>junit</artifactId></dependency>
                <dependency><groupId>g</groupId><artifactId>sibling</artifactId><version>${project.parent.version}</version></dependency>
                <dependency><groupId>x</groupId><artifactId>unmanaged</artifactId></dependency>
            </dependencies>
        </project>"#;
        let parent_ctx = pom_context(parent, &PomContext::default());
        assert_eq!(
            parent_ctx.managed.get("junit:junit"),
            Some(&"4.13.2".to_string())
        );
        assert!(!parent_ctx.managed.contains_key("org.bom:bom"));
        assert_eq!(
            pom_bom_imports(parent, &parent_ctx),
            vec![("org.bom".to_string(), "bom".to_string(), "2.0".to_string())]
        );
        assert_eq!(
            parse_pom_xml(child_pom, &parent_ctx),
            vec![
                ("junit:junit".to_string(), "4.13.2".to_string()),
                ("g:sibling".to_string(), "1.0".to_string()),
            ]
        );
        let p = pom_parent(child_pom).unwrap();
        assert_eq!(p.relative_path, "../pom.xml");
        assert_eq!(p.artifact_id, "p");
        assert!(pom_parent(parent).is_none());
        assert!(pom_parent("not xml <<").is_none());
    }

    #[test]
    fn parent_relative_path_defaults_and_chained_properties() {
        let child_pom = "<project><parent><groupId>g</groupId><artifactId>p</artifactId><version>1</version></parent>\
            <properties><a>${b}</a><b>3.1.4</b></properties>\
            <dependencies><dependency><groupId>g</groupId><artifactId>x</artifactId><version>${a}</version></dependency></dependencies></project>";
        assert_eq!(pom_parent(child_pom).unwrap().relative_path, "../pom.xml");
        assert_eq!(
            parse_pom_xml(child_pom, &PomContext::default()),
            vec![("g:x".to_string(), "3.1.4".to_string())]
        );
        let cyclic = "<project><properties><a>${a}</a></properties><dependencies><dependency><groupId>g</groupId><artifactId>x</artifactId><version>${a}</version></dependency></dependencies></project>";
        assert!(parse_pom_xml(cyclic, &PomContext::default()).is_empty());
        let unclosed = "<project><dependencies><dependency><groupId>g</groupId><artifactId>x</artifactId><version>${oops</version></dependency></dependencies></project>";
        assert!(parse_pom_xml(unclosed, &PomContext::default()).is_empty());
        assert_eq!(
            pom_context("not xml <<", &PomContext::default()),
            PomContext::default()
        );
        assert!(pom_bom_imports("not xml <<", &PomContext::default()).is_empty());
    }

    #[test]
    fn parses_gradle_string_and_map_notation() {
        let script = r#"
dependencies {
    implementation("com.google.guava:guava:33.0.0-jre")
    api 'org.slf4j:slf4j-api:2.0.9'
    testImplementation "junit:junit:4.13.2@jar"
    implementation group: 'org.apache.commons', name: 'commons-lang3', version: '3.14.0'
    implementation("com.example:dynamic:$version")
    implementation(project(":core"))
    implementation "com.example:no-version"
    // implementation("commented:out:1.0")
    kapt("com.example:proc:1.2.3")
}
"#;
        let deps = parse_gradle(script);
        assert_eq!(
            deps,
            vec![
                (
                    "com.google.guava:guava".to_string(),
                    "33.0.0-jre".to_string()
                ),
                ("org.slf4j:slf4j-api".to_string(), "2.0.9".to_string()),
                ("junit:junit".to_string(), "4.13.2".to_string()),
                (
                    "org.apache.commons:commons-lang3".to_string(),
                    "3.14.0".to_string()
                ),
                ("com.example:proc".to_string(), "1.2.3".to_string()),
            ]
        );
    }

    #[test]
    fn parses_gradle_version_catalogs() {
        let toml = r#"
[versions]
kotlin = "1.9.22"
okhttp = { strictly = "4.12.0" }

[libraries]
kotlin-stdlib = { module = "org.jetbrains.kotlin:kotlin-stdlib", version.ref = "kotlin" }
okhttp = { group = "com.squareup.okhttp3", name = "okhttp", version.ref = "okhttp" }
inline = { module = "a:b", version = "1.0" }
rich = { module = "c:d", version = { require = "2.0" } }
short = "e:f:3.0"
unversioned = { module = "g:h" }
badref = { module = "i:j", version.ref = "nope" }
"#;
        let mut deps = parse_version_catalog(toml);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("a:b".to_string(), "1.0".to_string()),
                ("c:d".to_string(), "2.0".to_string()),
                (
                    "com.squareup.okhttp3:okhttp".to_string(),
                    "4.12.0".to_string()
                ),
                ("e:f".to_string(), "3.0".to_string()),
                (
                    "org.jetbrains.kotlin:kotlin-stdlib".to_string(),
                    "1.9.22".to_string()
                ),
            ]
        );
        assert!(parse_version_catalog("not toml {{{").is_empty());
        assert!(parse_version_catalog("[versions]\na = \"1\"").is_empty());
    }
}

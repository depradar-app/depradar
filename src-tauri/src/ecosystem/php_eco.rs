//! Extracts the dependencies declared in a `composer.json`, and installed
//! versions from a `composer.lock`.

use std::collections::HashMap;

pub fn parse_composer_json(content: &str) -> Vec<(String, String)> {
    let json: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut deps = vec![];
    for section in ["require", "require-dev"] {
        if let Some(obj) = json.get(section).and_then(|v| v.as_object()) {
            for (name, value) in obj {
                // Platform packages ("php", "ext-mbstring", "lib-openssl"...)
                // aren't published on Packagist and have no registry entry;
                // every real Packagist package name is "vendor/package".
                if !name.contains('/') {
                    continue;
                }
                if let Some(req) = value.as_str() {
                    deps.push((name.clone(), req.to_string()));
                }
            }
        }
    }
    deps
}

/// Extracts installed versions from a `composer.lock`'s `packages` and
/// `packages-dev` arrays.
pub fn parse_composer_lock(content: &str) -> HashMap<String, String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(content) else {
        return HashMap::new();
    };

    let mut installed = HashMap::new();
    for section in ["packages", "packages-dev"] {
        if let Some(list) = json.get(section).and_then(|v| v.as_array()) {
            for package in list {
                let name = package.get("name").and_then(|v| v.as_str());
                let version = package.get("version").and_then(|v| v.as_str());
                if let (Some(name), Some(version)) = (name, version) {
                    installed.insert(name.to_string(), version.to_string());
                }
            }
        }
    }
    installed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_require_and_require_dev_and_skips_platform_packages() {
        let json = r#"{
            "require": {
                "php": "^8.1",
                "ext-mbstring": "*",
                "monolog/monolog": "^3.0"
            },
            "require-dev": {
                "phpunit/phpunit": "^10.0"
            }
        }"#;
        let mut deps = parse_composer_json(json);
        deps.sort();
        assert_eq!(
            deps,
            vec![
                ("monolog/monolog".to_string(), "^3.0".to_string()),
                ("phpunit/phpunit".to_string(), "^10.0".to_string()),
            ]
        );
    }

    #[test]
    fn ignores_malformed_json() {
        assert!(parse_composer_json("not json").is_empty());
    }

    #[test]
    fn parses_composer_lock_packages_and_packages_dev() {
        let lock = r#"{
            "packages": [
                { "name": "monolog/monolog", "version": "3.5.0" }
            ],
            "packages-dev": [
                { "name": "phpunit/phpunit", "version": "10.5.9" }
            ]
        }"#;
        let installed = parse_composer_lock(lock);
        assert_eq!(installed.get("monolog/monolog"), Some(&"3.5.0".to_string()));
        assert_eq!(
            installed.get("phpunit/phpunit"),
            Some(&"10.5.9".to_string())
        );
    }

    #[test]
    fn malformed_composer_lock_returns_empty_map() {
        assert!(parse_composer_lock("not json").is_empty());
    }
}

//! Queries [OSV.dev](https://osv.dev) for known vulnerabilities affecting a
//! package at a specific, concrete version.
//!
//! Only ever called with a version that is really in use: one pinned in a
//! lockfile, or an exact pin written in the manifest itself (Go, Maven,
//! `==x.y.z`). Checking a range's floor would report on a version that might
//! not be what's running, which is a worse signal than no signal at all.

use reqwest::Client;

use crate::registry::Endpoints;
use crate::version::Ecosystem;

/// OSV's documented maximum number of queries per `querybatch` request.
const BATCH_SIZE: usize = 1000;

/// Maps this app's ecosystem to the name OSV's API expects.
pub fn osv_ecosystem(ecosystem: Ecosystem) -> &'static str {
    match ecosystem {
        Ecosystem::Rust => "crates.io",
        Ecosystem::Npm => "npm",
        Ecosystem::Python => "PyPI",
        Ecosystem::Go => "Go",
        Ecosystem::Php => "Packagist",
        Ecosystem::Ruby => "RubyGems",
        Ecosystem::Java => "Maven",
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VulnQuery {
    pub ecosystem: Ecosystem,
    pub name: String,
    pub version: String,
}

/// Looks up known vulnerabilities for every query. The result is aligned
/// with `queries`: `Some(ids)` (possibly empty) when the query was answered,
/// `None` when its batch could not be fetched — so the caller can tell
/// "checked, nothing found" from "could not check".
pub async fn check_vulnerabilities(
    client: &Client,
    ep: &Endpoints,
    queries: &[VulnQuery],
) -> Vec<Option<Vec<String>>> {
    let mut results: Vec<Option<Vec<String>>> = Vec::with_capacity(queries.len());
    for chunk in queries.chunks(BATCH_SIZE) {
        let answered = query_chunk(client, ep, chunk).await;
        match answered {
            Some(per_query) => results.extend(per_query.into_iter().map(Some)),
            None => results.extend(chunk.iter().map(|_| None)),
        }
    }
    results
}

async fn query_chunk(
    client: &Client,
    ep: &Endpoints,
    chunk: &[VulnQuery],
) -> Option<Vec<Vec<String>>> {
    let body: Vec<serde_json::Value> = chunk
        .iter()
        .map(|q| {
            serde_json::json!({
                "package": { "name": q.name, "ecosystem": osv_ecosystem(q.ecosystem) },
                "version": q.version,
            })
        })
        .collect();

    let resp = client
        .post(format!("{}/v1/querybatch", ep.osv))
        .json(&serde_json::json!({ "queries": body }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json = resp.json::<serde_json::Value>().await.ok()?;
    parse_batch_response(chunk.len(), &json)
}

/// Parses a `querybatch` response body: `results[i]` corresponds to the i-th
/// query, in order (OSV's own contract for this endpoint); a query that
/// matched nothing comes back as `{}` (no `vulns` key at all). `None` if the
/// body doesn't have exactly one result per query.
fn parse_batch_response(expected: usize, body: &serde_json::Value) -> Option<Vec<Vec<String>>> {
    let results = body.get("results")?.as_array()?;
    if results.len() != expected {
        return None;
    }
    Some(
        results
            .iter()
            .map(|entry| {
                entry
                    .get("vulns")
                    .and_then(|v| v.as_array())
                    .map(|vulns| {
                        vulns
                            .iter()
                            .filter_map(|v| {
                                v.get("id").and_then(|id| id.as_str()).map(String::from)
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn q(name: &str, version: &str) -> VulnQuery {
        VulnQuery {
            ecosystem: Ecosystem::Npm,
            name: name.into(),
            version: version.into(),
        }
    }

    #[test]
    fn maps_every_ecosystem_to_its_osv_name() {
        let expected = [
            (Ecosystem::Rust, "crates.io"),
            (Ecosystem::Npm, "npm"),
            (Ecosystem::Python, "PyPI"),
            (Ecosystem::Go, "Go"),
            (Ecosystem::Php, "Packagist"),
            (Ecosystem::Ruby, "RubyGems"),
            (Ecosystem::Java, "Maven"),
        ];
        for (eco, name) in expected {
            assert_eq!(osv_ecosystem(eco), name);
        }
    }

    #[test]
    fn parses_a_batch_response_mixing_vulnerable_and_clean_packages() {
        let body = serde_json::json!({
            "results": [
                { "vulns": [
                    { "id": "GHSA-29mw-wpgm-hmr9", "modified": "2025-09-29T21:12:31Z" },
                    { "id": "CVE-2020-8203", "modified": "2025-01-01T00:00:00Z" },
                    { "modified": "no id" }
                ] },
                {}
            ]
        });
        let found = parse_batch_response(2, &body).unwrap();
        assert_eq!(
            found[0],
            vec![
                "GHSA-29mw-wpgm-hmr9".to_string(),
                "CVE-2020-8203".to_string()
            ]
        );
        assert!(found[1].is_empty());
    }

    #[test]
    fn malformed_or_misaligned_bodies_are_rejected() {
        assert!(parse_batch_response(1, &serde_json::json!({"unexpected": true})).is_none());
        assert!(parse_batch_response(2, &serde_json::json!({"results": [{}]})).is_none());
        assert!(parse_batch_response(1, &serde_json::json!({"results": "x"})).is_none());
    }

    #[tokio::test]
    async fn answers_are_aligned_with_the_queries() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/osv/v1/querybatch"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [ {"vulns": [{"id": "GHSA-1"}]}, {} ]
            })))
            .mount(&server)
            .await;
        let ep = Endpoints::all_at(&server.uri());
        let out = check_vulnerabilities(
            &Client::new(),
            &ep,
            &[q("lodash", "4.17.19"), q("react", "18.0.0")],
        )
        .await;
        assert_eq!(out, vec![Some(vec!["GHSA-1".to_string()]), Some(vec![])]);
    }

    #[tokio::test]
    async fn a_failing_service_yields_none_not_an_empty_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/osv/v1/querybatch"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let ep = Endpoints::all_at(&server.uri());
        let out = check_vulnerabilities(&Client::new(), &ep, &[q("a", "1.0.0")]).await;
        assert_eq!(out, vec![None]);

        let dead = Endpoints::all_at("http://127.0.0.1:1");
        let out = check_vulnerabilities(&Client::new(), &dead, &[q("a", "1.0.0")]).await;
        assert_eq!(out, vec![None]);
    }

    #[tokio::test]
    async fn large_query_sets_are_split_in_batches_of_1000() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/osv/v1/querybatch"))
            .respond_with(|req: &wiremock::Request| {
                let n = req.body_json::<serde_json::Value>().unwrap()["queries"]
                    .as_array()
                    .unwrap()
                    .len();
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "results": vec![serde_json::json!({}); n] }))
            })
            .expect(2)
            .mount(&server)
            .await;
        let ep = Endpoints::all_at(&server.uri());
        let queries: Vec<VulnQuery> = (0..1500).map(|i| q(&format!("p{i}"), "1.0.0")).collect();
        let out = check_vulnerabilities(&Client::new(), &ep, &queries).await;
        assert_eq!(out.len(), 1500);
        assert!(out.iter().all(|r| r == &Some(vec![])));
    }

    #[tokio::test]
    async fn no_queries_means_no_request() {
        let ep = Endpoints::all_at("http://127.0.0.1:1");
        assert!(check_vulnerabilities(&Client::new(), &ep, &[])
            .await
            .is_empty());
    }
}

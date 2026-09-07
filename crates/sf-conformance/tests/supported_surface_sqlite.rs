//! Required replay of the sealed public SQLite Query and Protocol subsets.

#[path = "supported_surface_sqlite/mod.rs"]
mod support;

use sf_conformance::supported_surface::{
    parse_manifest, render_manifest, verify_seal, ExpectedStatus, ManifestSeal,
};
use support::{protocol, protocol_seal, query, query_seal};

const QUERY_MANIFEST: &str = include_str!("../../../tests/sparql/query/supported-surface-v1.tsv");
const PROTOCOL_MANIFEST: &str =
    include_str!("../../../tests/sparql/protocol/supported-surface-v1.tsv");
const CAPABILITY_CATALOG: &str = include_str!("../../../tests/capabilities/catalog-v1.json");

#[tokio::test]
async fn sealed_query_supported_surface_matches_public_sqlite_behavior() {
    let manifest = parse_manifest(QUERY_MANIFEST).expect("parse sealed Query manifest");
    verify_seal(QUERY_MANIFEST, &manifest, &query_seal()).expect("verify sealed Query manifest");

    query::replay(&manifest)
        .await
        .expect("replay Query surface");
}

#[tokio::test]
async fn sealed_protocol_supported_surface_matches_public_sqlite_behavior() {
    let manifest = parse_manifest(PROTOCOL_MANIFEST).expect("parse sealed Protocol manifest");
    verify_seal(PROTOCOL_MANIFEST, &manifest, &protocol_seal())
        .expect("verify sealed Protocol manifest");

    protocol::replay(&manifest)
        .await
        .expect("replay Protocol surface");
}

#[test]
fn compiled_seals_reject_exact_inventory_mutants() {
    reject_inventory_mutants(QUERY_MANIFEST, query_seal(), "query-zz-extra");
    reject_inventory_mutants(PROTOCOL_MANIFEST, protocol_seal(), "protocol-zz-extra");
}

#[test]
fn sealed_inventories_have_explicit_typed_outcome_totals() {
    let query = parse_manifest(QUERY_MANIFEST).expect("parse sealed Query manifest");
    let protocol = parse_manifest(PROTOCOL_MANIFEST).expect("parse sealed Protocol manifest");

    assert_eq!(outcome_totals(&query.cases), (9, 3, 0));
    assert_eq!(outcome_totals(&protocol.cases), (13, 0, 12));
}

#[test]
fn standard_snapshots_match_capability_catalog_authority() {
    let catalog: serde_json::Value =
        serde_json::from_str(CAPABILITY_CATALOG).expect("parse capability catalog");
    let standards = catalog["standards"]
        .as_array()
        .expect("catalog standards array");
    for text in [QUERY_MANIFEST, PROTOCOL_MANIFEST] {
        let manifest = parse_manifest(text).expect("parse sealed supported-surface manifest");
        let entry = standards
            .iter()
            .find(|entry| entry["id"].as_str() == Some(manifest.standard.id.as_str()))
            .expect("manifest standard exists in capability catalog");
        assert_eq!(entry["url"].as_str(), Some(manifest.standard.url.as_str()));
        assert_eq!(entry["status"].as_str(), Some("W3C Working Draft"));
        assert_eq!(
            entry["snapshotDate"].as_str(),
            Some(manifest.standard.snapshot_date.as_str())
        );
        assert_eq!(
            entry["byteLength"].as_u64(),
            Some(manifest.standard.byte_length)
        );
        assert_eq!(
            entry["sha256"].as_str(),
            Some(manifest.standard.sha256.as_str())
        );
        assert_eq!(
            entry["classification"].as_str(),
            Some("retrieved-reference-metadata")
        );
    }
}

fn reject_inventory_mutants(text: &str, seal: ManifestSeal, extra_id: &str) {
    let original = parse_manifest(text).expect("parse sealed supported-surface manifest");

    let mut missing = original.clone();
    missing.cases.pop();
    assert_valid_mutant_fails_seal(&missing, &seal);

    let mut extra = original.clone();
    let mut extra_case = extra.cases.last().expect("non-empty manifest").clone();
    extra_case.id = extra_id.to_owned();
    extra_case.scenario = extra_id.to_owned();
    extra.cases.push(extra_case);
    assert_valid_mutant_fails_seal(&extra, &seal);

    let mut reclassified = original.clone();
    let first = reclassified.cases.first_mut().expect("non-empty manifest");
    first.response_media_type = "application/problem+json".to_owned();
    match seal.surface {
        sf_conformance::supported_surface::Surface::SparqlQuery => {
            first.expected_status = ExpectedStatus::Unsupported;
            first.http_status = 501;
            first.cause = "unsupported-query".to_owned();
        }
        sf_conformance::supported_surface::Surface::SparqlProtocol => {
            first.expected_status = ExpectedStatus::Rejected;
            first.http_status = 400;
            first.cause = "invalid-request".to_owned();
        }
    }
    assert_valid_mutant_fails_seal(&reclassified, &seal);

    let mut reordered = original;
    reordered.cases.swap(0, 1);
    assert_eq!(
        parse_manifest(&render_manifest(&reordered)),
        Err("case records are not strictly ordered".to_owned())
    );
}

fn assert_valid_mutant_fails_seal(
    mutant: &sf_conformance::supported_surface::Manifest,
    seal: &ManifestSeal,
) {
    let text = render_manifest(mutant);
    let parsed = parse_manifest(&text).expect("mutant remains internally valid");
    assert_eq!(
        verify_seal(&text, &parsed, seal),
        Err("supported-surface manifest does not match its fixed seal".to_owned())
    );
}

fn outcome_totals(cases: &[sf_conformance::supported_surface::Case]) -> (usize, usize, usize) {
    cases.iter().fold((0, 0, 0), |mut totals, case| {
        match case.expected_status {
            ExpectedStatus::Supported => totals.0 += 1,
            ExpectedStatus::Unsupported => totals.1 += 1,
            ExpectedStatus::Rejected => totals.2 += 1,
        }
        totals
    })
}

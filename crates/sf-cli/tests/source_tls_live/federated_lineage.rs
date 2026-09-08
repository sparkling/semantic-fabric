//! Actual public metadata is checked before comparing the complete ordinary bag.
use super::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(super) fn bag(body: &[u8], source_counts: [usize; 2]) -> (Value, Vec<String>) {
    checked_bag(body, Some(source_counts))
}

pub(super) fn join_bag(body: &[u8]) -> (Value, Vec<String>) {
    checked_bag(body, None)
}

fn checked_bag(body: &[u8], source_counts: Option<[usize; 2]>) -> (Value, Vec<String>) {
    let records: Vec<Value> = std::str::from_utf8(body)
        .unwrap()
        .split('\u{1e}')
        .skip(1)
        .map(|part| serde_json::from_str(part).unwrap())
        .collect();
    let header = &records[0];
    let join = source_counts.is_none();
    assert_eq!(
        header["profile"],
        if join {
            "bounded-federated-join-lineage-v1"
        } else {
            "bounded-federated-union-lineage-v1"
        }
    );
    if join {
        assert_eq!(header["maxBuildTriples"], 128);
        assert_eq!(header["maxProbeTriples"], 4096);
    }
    assert_eq!(header["rowKeys"], "not-provided");
    assert_eq!(header["sources"].as_array().unwrap().len(), 2);
    assert_ne!(
        header["sources"][0]["source"],
        header["sources"][1]["source"]
    );
    assert_eq!(records.last().unwrap()["type"], "complete");
    let count = records.len() - 2;
    if let Some(counts) = source_counts {
        assert_eq!(count, counts.iter().sum::<usize>());
    }
    assert_eq!(records.last().unwrap()["solutions"], count);
    let mut results = Vec::new();
    let mut bundles = BTreeSet::new();
    for (ordinal, record) in records[1..records.len() - 1].iter().enumerate() {
        assert_eq!(record["ordinal"], ordinal);
        assert_eq!(record["type"], "solution");
        assert_eq!(record["result"]["head"]["vars"], header["variables"]);
        let rows = record["result"]["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        results.push(rows[0].clone());
        assert!(bundles.insert(record["provenance"]["@id"].clone().to_string()));
        let nodes = record["provenance"]["@graph"].as_array().unwrap();
        let sources: Vec<_> = nodes
            .iter()
            .filter_map(|n| n["sf:sourceId"].as_u64())
            .collect();
        // Counts are known from the fixture, not inferred from returned metadata.
        let expected_sources: Vec<usize> = source_counts.map_or_else(
            || vec![0, 1],
            |counts| vec![usize::from(ordinal >= counts[0])],
        );
        assert_eq!(
            sources,
            expected_sources
                .iter()
                .map(|&s| s as u64)
                .collect::<Vec<_>>()
        );
        let maps: BTreeSet<_> = nodes
            .iter()
            .filter_map(|n| n["sf:mappingId"].as_str())
            .collect();
        let expected: BTreeSet<_> = expected_sources
            .iter()
            .flat_map(|&source| {
                header["sources"][source]["mappingCatalog"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
            })
            .collect();
        assert_eq!(maps, expected);
        let quads = oxjsonld::JsonLdParser::new()
            .for_slice(record["provenance"].to_string().as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for field in ["snapshot", "logicalPlan", "policy"] {
            let refs: Vec<_> = quads
                .iter()
                .filter(|q| q.predicate.as_str() == format!("urn:semantic-fabric:lineage:{field}"))
                .map(|q| q.object.clone())
                .collect();
            assert_eq!(
                refs,
                vec![oxrdf::NamedNode::new(header[field].as_str().unwrap())
                    .unwrap()
                    .into()]
            );
        }
        let used: BTreeSet<_> = quads
            .iter()
            .filter(|q| q.predicate.as_str() == "http://www.w3.org/ns/prov#used")
            .map(|q| q.object.to_string())
            .collect();
        for source in expected_sources {
            assert!(used.contains(&format!(
                "<{}>",
                header["sources"][source]["source"].as_str().unwrap()
            )));
            assert!(used.contains(&format!(
                "<{}>",
                header["sources"][source]["mappingDocument"]
                    .as_str()
                    .unwrap()
            )));
            if join {
                let map = nodes
                    .iter()
                    .find(|n| {
                        n["sf:mappingId"] == header["sources"][source]["mappingCatalog"][0]
                            && n["sf:source"]["@id"] == header["sources"][source]["source"]
                    })
                    .unwrap();
                assert!(used.contains(&format!("<{}>", map["@id"].as_str().unwrap())));
                assert!(quads.iter().any(|q| q.predicate.as_str()
                    == "urn:semantic-fabric:lineage:source"
                    && q.subject.to_string() == format!("<{}>", map["@id"].as_str().unwrap())
                    && q.object.to_string()
                        == format!(
                            "<{}>",
                            header["sources"][source]["source"].as_str().unwrap()
                        )));
            }
        }
    }
    stop_matrix::bag(
        &serde_json::to_vec(
            &json!({"head":{"vars":header["variables"]}, "results":{"bindings":results}}),
        )
        .unwrap(),
    )
}

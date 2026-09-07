//! Canonical Product Mock semantic document assembly.
//!
//! Categories 1–12 and 14 are the ontology document `T`. Category 7 is kept
//! because its SHACL statements are part of the semantic contract, and Category
//! 9 is kept because provenance is part of the sealed document identity. The
//! cross-domain alignments in Category 8 are semantic assertions, not source
//! mappings. Category 13 is the independently admitted mapping document `M` and
//! is therefore never copied into `T`: a mapping must not self-authorize.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{array, expect_str, expect_u64, sha256, string, unsigned};

pub(super) const MAX_ONTOLOGY_SHARDS: usize = 192;
pub(super) const MAX_ONTOLOGY_SHARD_BYTES: u64 = 64 * 1024;
pub(super) const MAX_ONTOLOGY_TURTLE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct CategorySpec {
    pub number: u64,
    pub slug: &'static str,
    pub concern: &'static str,
    pub coverage: &'static str,
    pub in_ontology: bool,
}

impl CategorySpec {
    pub fn directory(self) -> String {
        format!("categories/{:02}-{}", self.number, self.slug)
    }

    pub fn manifest_path(self) -> String {
        format!("{}/category.json", self.directory())
    }

    pub fn shard_path(self, ordinal: usize) -> String {
        format!("{}/part-{ordinal:03}.ttl", self.directory())
    }
}

pub(super) const CATEGORY_SPECS: [CategorySpec; 14] = [
    CategorySpec {
        number: 1,
        slug: "domain-structure",
        concern: "Domain Structure",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 2,
        slug: "vocabulary-taxonomy",
        concern: "Vocabulary & Taxonomy",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 3,
        slug: "process-modelling",
        concern: "Process Modelling",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 4,
        slug: "service-architecture",
        concern: "Service Architecture",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 5,
        slug: "classification-metadata",
        concern: "Classification Metadata",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 6,
        slug: "governance-compliance",
        concern: "Governance & Compliance",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 7,
        slug: "validation-constraints",
        concern: "Validation & Constraints",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 8,
        slug: "cross-domain-mappings",
        concern: "Cross-Domain Mappings",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 9,
        slug: "extraction-provenance-quality",
        concern: "Extraction Provenance & Quality",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 10,
        slug: "temporal-state-history",
        concern: "Temporal State & History",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 11,
        slug: "access-control-data-sensitivity",
        concern: "Access Control & Data Sensitivity",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 12,
        slug: "capability-intent",
        concern: "Capability & Intent",
        coverage: "covered",
        in_ontology: true,
    },
    CategorySpec {
        number: 13,
        slug: "source-mapping",
        concern: "Source Mapping",
        coverage: "facet-scoped-complete",
        in_ontology: false,
    },
    CategorySpec {
        number: 14,
        slug: "data-product",
        concern: "Data Product",
        coverage: "covered",
        in_ontology: true,
    },
];

pub(super) fn is_canonical_category_artifact(path: &str) -> bool {
    CATEGORY_SPECS.iter().copied().any(|category| {
        path == category.manifest_path()
            || path
                .strip_prefix(&format!("{}/part-", category.directory()))
                .and_then(|tail| tail.strip_suffix(".ttl"))
                .is_some_and(|ordinal| {
                    ordinal.len() == 3 && ordinal.bytes().all(|byte| byte.is_ascii_digit())
                })
    })
}

pub(super) fn account_descriptor(
    path: &str,
    bytes: u64,
    shards: &mut usize,
    total: &mut u64,
) -> Result<(), &'static str> {
    let Some(category) = category_for_path(path) else {
        return Ok(());
    };
    if !category.in_ontology || !path.ends_with(".ttl") {
        return Ok(());
    }
    if bytes > MAX_ONTOLOGY_SHARD_BYTES {
        return Err("canonical ontology shard exceeds its byte limit");
    }
    *shards = shards
        .checked_add(1)
        .ok_or("canonical ontology shard count overflow")?;
    if *shards > MAX_ONTOLOGY_SHARDS {
        return Err("canonical ontology shard count exceeds its limit");
    }
    *total = total
        .checked_add(bytes)
        .ok_or("canonical ontology byte count overflow")?;
    if *total > MAX_ONTOLOGY_TURTLE_BYTES {
        return Err("canonical ontology exceeds its byte limit");
    }
    Ok(())
}

pub(super) fn assemble(
    root_manifest: &Value,
    artifacts: &mut BTreeMap<String, Vec<u8>>,
) -> Result<String, &'static str> {
    validate_root_categories(root_manifest)?;
    let mut turtle = Vec::new();
    let mut shard_count = 0_usize;

    for category in CATEGORY_SPECS
        .iter()
        .copied()
        .filter(|category| category.in_ontology)
    {
        let manifest_path = category.manifest_path();
        let bytes = artifacts
            .remove(&manifest_path)
            .ok_or("canonical ontology category manifest is missing")?;
        let manifest: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "canonical ontology category manifest is invalid")?;
        expect_u64(&manifest, "/category", category.number)?;
        expect_str(&manifest, "/concern", category.concern)?;
        expect_str(&manifest, "/coverageStatus", category.coverage)?;
        let descriptors = array(&manifest, "/shards")?;
        if descriptors.is_empty() {
            return Err("canonical ontology category has no shards");
        }
        for (index, descriptor) in descriptors.iter().enumerate() {
            let path = string(descriptor, "/path")?;
            if path != category.shard_path(index + 1) {
                return Err("canonical ontology shard path or order mismatch");
            }
            let source = artifacts
                .remove(path)
                .ok_or("canonical ontology shard artifact is missing")?;
            validate_shard(descriptor, &source)?;
            shard_count = shard_count
                .checked_add(1)
                .ok_or("canonical ontology shard count overflow")?;
            if shard_count > MAX_ONTOLOGY_SHARDS {
                return Err("canonical ontology shard count exceeds its limit");
            }
            let projected = turtle
                .len()
                .checked_add(source.len())
                .and_then(|size| size.checked_add(1))
                .ok_or("canonical ontology byte count overflow")?;
            if projected as u64 > MAX_ONTOLOGY_TURTLE_BYTES {
                return Err("canonical ontology exceeds its byte limit");
            }
            turtle.extend_from_slice(&source);
            if !source.ends_with(b"\n") {
                turtle.push(b'\n');
            }
        }
    }
    if artifacts
        .keys()
        .any(|path| category_for_path(path).is_some_and(|category| category.in_ontology))
    {
        return Err("canonical ontology shard set mismatch");
    }
    String::from_utf8(turtle).map_err(|_| "canonical ontology Turtle is not UTF-8")
}

fn validate_root_categories(manifest: &Value) -> Result<(), &'static str> {
    let categories = array(manifest, "/categories")?;
    if categories.len() != CATEGORY_SPECS.len() {
        return Err("canonical category inventory mismatch");
    }
    for (claim, expected) in categories.iter().zip(CATEGORY_SPECS) {
        expect_u64(claim, "/category", expected.number)?;
        expect_str(claim, "/concern", expected.concern)?;
        expect_str(claim, "/coverageStatus", expected.coverage)?;
    }
    Ok(())
}

fn validate_shard(descriptor: &Value, source: &[u8]) -> Result<(), &'static str> {
    let bytes = unsigned(descriptor, "/bytes")?;
    let lines = unsigned(descriptor, "/lines")?;
    let digest = string(descriptor, "/digest")?;
    let actual_lines =
        source.iter().filter(|byte| **byte == b'\n').count() as u64 + u64::from(!source.is_empty());
    if bytes > MAX_ONTOLOGY_SHARD_BYTES
        || source.len() as u64 != bytes
        || actual_lines != lines
        || sha256(source) != digest
    {
        return Err("canonical ontology shard seal mismatch");
    }
    Ok(())
}

fn category_for_path(path: &str) -> Option<CategorySpec> {
    CATEGORY_SPECS.iter().copied().find(|category| {
        path == category.manifest_path() || path.starts_with(&format!("{}/", category.directory()))
    })
}

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::{
    ontology::CATEGORY_SPECS, pin, sha256, Pin, SealPolicy, CATEGORY_MAPPING_PATH,
    CATEGORY_SHARD_PREFIX, COVERAGE_PATH, FK_MIGRATION, INITIAL_MIGRATION, SNAPSHOT_PATH,
    SOURCE_REVISION, STYLE_CLASS, STYLE_MAP, STYLE_NUMBER_PREDICATE, VERSION_PREDICATE,
};

#[derive(Clone, Debug)]
pub struct SyntheticFixture {
    pub policy: SealPolicy,
    pub manifest: Vec<u8>,
    pub artifacts: BTreeMap<String, Vec<u8>>,
    pub sources: BTreeMap<String, Vec<u8>>,
}

impl SyntheticFixture {
    pub fn valid() -> Self {
        let sources = BTreeMap::from([
            (
                INITIAL_MIGRATION.to_owned(),
                b"synthetic initial migration".to_vec(),
            ),
            (FK_MIGRATION.to_owned(), b"synthetic FK migration".to_vec()),
            (
                "src/unpinned.rs".to_owned(),
                b"synthetic source outside the required migration pins".to_vec(),
            ),
        ]);
        let snapshot_pins: Vec<_> = sources
            .iter()
            .map(|(path, bytes)| pin(path, bytes))
            .collect();
        let source_pins: Vec<_> = snapshot_pins
            .iter()
            .filter(|pin| [INITIAL_MIGRATION, FK_MIGRATION].contains(&pin.path.as_str()))
            .cloned()
            .collect();
        let snapshot = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "repository": {"revision": SOURCE_REVISION},
            "files": snapshot_pins.iter().map(pin_json).collect::<Vec<_>>()
        }))
        .expect("synthetic snapshot serializes");
        let mapping_shards = synthetic_mapping_shards();
        let shard_pins: Vec<_> = mapping_shards
            .iter()
            .map(|(path, bytes)| pin(path, bytes))
            .collect();
        let category = serde_json::to_vec(&json!({
            "category": 13,
            "concern": "Source Mapping",
            "coverageStatus": "facet-scoped-complete",
            "stats": category_stats(),
            "shards": shard_pins.iter().map(|entry| json!({
                "path": entry.path,
                "digest": entry.digest,
                "bytes": entry.bytes,
                "lines": mapping_shards[&entry.path].iter().filter(|byte| **byte == b'\n').count()
                    + usize::from(!mapping_shards[&entry.path].is_empty())
            })).collect::<Vec<_>>()
        }))
        .expect("synthetic Category 13 manifest serializes");
        let mut artifacts = BTreeMap::from([
            (SNAPSHOT_PATH.to_owned(), snapshot),
            (COVERAGE_PATH.to_owned(), synthetic_coverage()),
            (CATEGORY_MAPPING_PATH.to_owned(), category),
        ]);
        artifacts.extend(synthetic_ontology_artifacts());
        artifacts.extend(mapping_shards);
        let artifact_pins: Vec<_> = artifacts
            .iter()
            .map(|(path, bytes)| pin(path, bytes))
            .collect();
        let manifest = serde_json::to_vec(&json!({
            "purpose": "development",
            "source": {"pinnedRevision": SOURCE_REVISION, "admittedView": "exact-committed-tree", "mutableHeadAndWorkingTreeExcluded": true},
            "categoryCount": 14,
            "categories": synthetic_category_claims(),
            "applicability": {"productionAuthority": false},
            "operationalQualification": {"productionAuthority": false},
            "artifactFiles": artifact_pins.iter().map(pin_json).collect::<Vec<_>>()
        }))
        .expect("synthetic manifest serializes");
        let policy = SealPolicy {
            manifest_bytes: manifest.len(),
            manifest_sha256: sha256(&manifest),
            artifact_count: artifacts.len(),
            artifact_bytes: artifacts.values().map(|value| value.len() as u64).sum(),
            snapshot_file_count: sources.len(),
            snapshot_file_bytes: sources.values().map(|value| value.len() as u64).sum(),
            source_pins,
        };
        Self {
            policy,
            manifest,
            artifacts,
            sources,
        }
    }

    pub fn reseal_manifest(&mut self) {
        self.policy.manifest_bytes = self.manifest.len();
        self.policy.manifest_sha256 = sha256(&self.manifest);
    }

    pub fn reseal_artifact(&mut self, path: &str) {
        let bytes = self.artifacts.get(path).expect("synthetic artifact exists");
        let mut manifest: Value =
            serde_json::from_slice(&self.manifest).expect("synthetic manifest parses");
        let entries = manifest["artifactFiles"]
            .as_array_mut()
            .expect("synthetic artifactFiles array");
        let entry = entries
            .iter_mut()
            .find(|entry| entry["path"].as_str() == Some(path))
            .expect("synthetic artifact pin exists");
        entry["bytes"] = json!(bytes.len());
        entry["digest"] = json!(sha256(bytes));
        self.policy.artifact_bytes = self
            .artifacts
            .values()
            .map(|value| value.len() as u64)
            .sum();
        self.manifest = serde_json::to_vec(&manifest).expect("synthetic manifest serializes");
        self.reseal_manifest();
    }
}

fn pin_json(entry: &Pin) -> serde_json::Value {
    json!({"path": entry.path, "bytes": entry.bytes, "digest": entry.digest})
}

fn synthetic_category_claims() -> Vec<Value> {
    CATEGORY_SPECS
        .iter()
        .map(|category| {
            let stats = if category.number == 13 {
                category_stats()
            } else {
                json!({})
            };
            json!({
                "category": category.number,
                "concern": category.concern,
                "coverageStatus": category.coverage,
                "stats": stats
            })
        })
        .collect()
}

fn synthetic_ontology_artifacts() -> BTreeMap<String, Vec<u8>> {
    let mut artifacts = BTreeMap::new();
    for category in CATEGORY_SPECS
        .iter()
        .copied()
        .filter(|category| category.in_ontology)
    {
        let shard_path = category.shard_path(1);
        let concern = serde_json::to_string(category.concern).expect("static concern serializes");
        let shard = format!(
            "<https://example.invalid/ontology/category/{:02}> \
             <https://example.invalid/ontology/concern> {concern} .\n",
            category.number
        )
        .into_bytes();
        let category_manifest = serde_json::to_vec(&json!({
            "category": category.number,
            "concern": category.concern,
            "coverageStatus": category.coverage,
            "stats": {},
            "shards": [{
                "path": shard_path,
                "digest": sha256(&shard),
                "bytes": shard.len(),
                "lines": shard.iter().filter(|byte| **byte == b'\n').count()
                    + usize::from(!shard.is_empty())
            }]
        }))
        .expect("synthetic ontology category manifest serializes");
        artifacts.insert(shard_path, shard);
        artifacts.insert(category.manifest_path(), category_manifest);
    }
    artifacts
}

fn synthetic_coverage() -> Vec<u8> {
    let mut bindings = (0..111)
        .map(|index| {
            json!({
                "tableIdentity": format!("synthetic/source::{index}"),
                "subjectClassIri": format!("https://example.invalid/class/{index}")
            })
        })
        .collect::<Vec<_>>();
    bindings.push(json!({
        "tableIdentity": "src/services/ProductDesign/ProductDesign.Infrastructure::style",
        "subjectClassIri": STYLE_CLASS
    }));
    serde_json::to_vec(&json!({
        "sourceRevision": SOURCE_REVISION,
        "relationalSchema": {"summary": {"tables": 112, "columns": 598},
            "stores": synthetic_stores()},
        "relationalR2rml": {"status":"complete","mappedTables":112,"mappedColumns":598,
            "unmappedTables":0,"unmappedColumns":0,"triplesMaps":148,
            "predicateObjectMaps":721,"bindings":bindings}
    }))
    .expect("synthetic coverage serializes")
}

fn category_stats() -> Value {
    json!({
        "rmlTriplesMaps": 134,
        "predicateObjectMaps": 492,
        "relationalSchemaTables": 112,
        "relationalSchemaColumns": 598,
        "relationalR2rmlTriplesMaps": 148,
        "relationalR2rmlPredicateObjectMaps": 721,
        "relationalR2rmlMappedTables": 112,
        "relationalR2rmlMappedColumns": 598
    })
}

fn synthetic_stores() -> Vec<Value> {
    let stores = [
        ("ProductDesign", 11),
        ("CostingPricing", 10),
        ("FinanceCostAccounting", 13),
        ("LogisticsAllocation", 10),
        ("MaterialsBom", 12),
        ("MerchandisingAssortment", 12),
        ("ProductionManufacturing", 8),
        ("QualityCompliance", 10),
        ("SamplingFit", 9),
        ("Style360", 3),
        ("SupplierSourcing", 14),
    ];
    let mut six_column_relations = 38;
    stores
        .into_iter()
        .map(|(store, table_count)| {
            let relations = (0..table_count)
                .map(|index| {
                    if store == "ProductDesign" && index == 0 {
                        return synthetic_style();
                    }
                    let column_count = if six_column_relations > 0 {
                        six_column_relations -= 1;
                        6
                    } else {
                        5
                    };
                    json!({
                        "name": format!("synthetic_relation_{index}"),
                        "columns": (0..column_count).map(|column| json!({
                            "name": format!("column_{column}"),
                            "storeType": "text",
                            "nullable": false
                        })).collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>();
            json!({"store": store, "relations": relations})
        })
        .collect()
}

fn synthetic_style() -> Value {
    json!({"name": "style", "columns": [
        {"name":"style_number","storeType":"text","nullable":false},
        {"name":"season_code","storeType":"text","nullable":false},
        {"name":"design_brief_id","storeType":"uuid","nullable":false},
        {"name":"status","storeType":"text","nullable":false},
        {"name":"version","storeType":"integer","nullable":false}
    ], "primaryKey":{"columns":["style_number"]}, "foreignKeyDefinitions":[
        {"name":"FK_style_season_ref","childColumns":["season_code"],"parentTable":"season_ref","parentColumns":["season_code"]},
        {"name":"FK_style_design_brief","childColumns":["design_brief_id"],"parentTable":"design_brief","parentColumns":["id"]}
    ]})
}

fn synthetic_mapping_shards() -> BTreeMap<String, Vec<u8>> {
    let mut triples = Vec::new();
    for index in 0..148 {
        let map = if index == 0 {
            STYLE_MAP.to_owned()
        } else {
            format!("https://example.invalid/r2rml/map/{index}")
        };
        let logical = format!("{map}/logical-table");
        let subject = format!("{map}/subject-map");
        push_iri(&mut triples, &map, RDF_TYPE, RR_TRIPLES_MAP);
        push_iri(&mut triples, &map, RR_LOGICAL_TABLE, &logical);
        if index == 0 {
            push_literal(&mut triples, &logical, RR_SQL_QUERY, STYLE_SQL_QUERY);
        } else {
            push_literal(
                &mut triples,
                &logical,
                RR_TABLE_NAME,
                &format!("synthetic_table_{index}"),
            );
        }
        push_iri(&mut triples, &map, RR_SUBJECT_MAP, &subject);
        let template = if index == 0 {
            format!("{STYLE_TEMPLATE_PREFIX}{{style_number}}")
        } else {
            format!("https://example.invalid/resource/{index}/{{id}}")
        };
        push_literal(&mut triples, &subject, RR_TEMPLATE, &template);
        push_iri(&mut triples, &subject, RR_TERM_TYPE, RR_IRI);
        push_iri(
            &mut triples,
            &subject,
            RR_CLASS,
            if index == 0 {
                STYLE_CLASS
            } else {
                "https://example.invalid/class/Synthetic"
            },
        );
        let pom_count = if index == 0 {
            11
        } else if index <= 122 {
            5
        } else {
            4
        };
        for pom_index in 0..pom_count {
            let pom = format!("{map}/predicate-object-map/{pom_index}");
            let object = format!("{pom}/object-map");
            let (predicate, column, datatype) = style_or_synthetic_term(index, pom_index);
            push_iri(&mut triples, &map, RR_PREDICATE_OBJECT_MAP, &pom);
            push_iri(&mut triples, &pom, RR_PREDICATE, &predicate);
            push_iri(&mut triples, &pom, RR_OBJECT_MAP, &object);
            push_literal(&mut triples, &object, RR_COLUMN, &column);
            push_iri(&mut triples, &object, RR_DATATYPE, &datatype);
        }
    }
    for index in 0..134 {
        let map = format!("https://example.invalid/rml/map/{index}");
        let logical = format!("{map}/logical-source");
        let subject = format!("{map}/subject-map");
        push_iri(&mut triples, &map, RDF_TYPE, RML_TRIPLES_MAP);
        push_iri(&mut triples, &map, RML_LOGICAL_SOURCE, &logical);
        push_iri(
            &mut triples,
            &logical,
            RML_SOURCE,
            "https://example.invalid/source",
        );
        push_iri(&mut triples, &map, RR_SUBJECT_MAP, &subject);
        push_literal(
            &mut triples,
            &subject,
            RR_TEMPLATE,
            &format!("https://example.invalid/rml-resource/{index}/{{id}}"),
        );
    }
    let mut parts = [String::new(), String::new()];
    for (index, triple) in triples.into_iter().enumerate() {
        parts[index % 2].push_str(&triple);
    }
    BTreeMap::from([
        (
            format!("{CATEGORY_SHARD_PREFIX}001.ttl"),
            parts[0].as_bytes().to_vec(),
        ),
        (
            format!("{CATEGORY_SHARD_PREFIX}002.ttl"),
            parts[1].as_bytes().to_vec(),
        ),
    ])
}

fn style_or_synthetic_term(index: usize, pom_index: usize) -> (String, String, String) {
    if index == 0 && pom_index == 0 {
        return (
            STYLE_NUMBER_PREDICATE.to_owned(),
            "style_number".to_owned(),
            XSD_STRING.to_owned(),
        );
    }
    if index == 0 && pom_index == 1 {
        return (
            VERSION_PREDICATE.to_owned(),
            "version".to_owned(),
            XSD_INTEGER.to_owned(),
        );
    }
    (
        format!("https://example.invalid/predicate/{index}/{pom_index}"),
        format!("column_{pom_index}"),
        XSD_STRING.to_owned(),
    )
}

fn push_iri(triples: &mut Vec<String>, subject: &str, predicate: &str, object: &str) {
    triples.push(format!("<{subject}> <{predicate}> <{object}> .\n"));
}

fn push_literal(triples: &mut Vec<String>, subject: &str, predicate: &str, object: &str) {
    let object = serde_json::to_string(object).expect("synthetic literal serializes");
    triples.push(format!("<{subject}> <{predicate}> {object} .\n"));
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RR_TRIPLES_MAP: &str = "http://www.w3.org/ns/r2rml#TriplesMap";
const RR_LOGICAL_TABLE: &str = "http://www.w3.org/ns/r2rml#logicalTable";
const RR_SQL_QUERY: &str = "http://www.w3.org/ns/r2rml#sqlQuery";
const RR_TABLE_NAME: &str = "http://www.w3.org/ns/r2rml#tableName";
const RR_SUBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#subjectMap";
const RR_TEMPLATE: &str = "http://www.w3.org/ns/r2rml#template";
const RR_TERM_TYPE: &str = "http://www.w3.org/ns/r2rml#termType";
const RR_IRI: &str = "http://www.w3.org/ns/r2rml#IRI";
const RR_CLASS: &str = "http://www.w3.org/ns/r2rml#class";
const RR_PREDICATE_OBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#predicateObjectMap";
const RR_PREDICATE: &str = "http://www.w3.org/ns/r2rml#predicate";
const RR_OBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#objectMap";
const RR_COLUMN: &str = "http://www.w3.org/ns/r2rml#column";
const RR_DATATYPE: &str = "http://www.w3.org/ns/r2rml#datatype";
const RML_TRIPLES_MAP: &str = "http://w3id.org/rml/TriplesMap";
const RML_LOGICAL_SOURCE: &str = "http://w3id.org/rml/logicalSource";
const RML_SOURCE: &str = "http://w3id.org/rml/source";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const STYLE_SQL_QUERY: &str = "SELECT source.*, CASE source.\"status\" WHEN 'Draft' THEN \
    'https://hm.com/ns/semantic-product-mock/product-design/vocabulary/StyleStatus/Draft' \
    WHEN 'Locked' THEN \
    'https://hm.com/ns/semantic-product-mock/product-design/vocabulary/StyleStatus/Locked' \
    ELSE NULL END AS \"__sb_enum_2789c5144a07118e\" FROM \"style\" AS source";
const STYLE_TEMPLATE_PREFIX: &str = "https://hm.com/ns/semantic-product-mock/resource/record/\
    2e86ee77d484e286f8cf45bf9ef67aee749476294c96b3a13eb8fcd3e554b020/";

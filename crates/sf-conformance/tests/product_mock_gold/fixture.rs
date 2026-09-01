use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::{
    pin, sha256, Pin, SealPolicy, COVERAGE_PATH, FK_MIGRATION, INITIAL_MIGRATION, MAPPING_PATH,
    SNAPSHOT_PATH, SOURCE_REVISION,
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
        let artifacts = BTreeMap::from([
            (SNAPSHOT_PATH.to_owned(), snapshot),
            (COVERAGE_PATH.to_owned(), synthetic_coverage()),
            (
                MAPPING_PATH.to_owned(),
                synthetic_mapping().as_bytes().to_vec(),
            ),
        ]);
        let artifact_pins: Vec<_> = artifacts
            .iter()
            .map(|(path, bytes)| pin(path, bytes))
            .collect();
        let manifest = serde_json::to_vec(&json!({
            "purpose": "development",
            "source": {"pinnedRevision": SOURCE_REVISION, "admittedView": "exact-committed-tree", "mutableHeadAndWorkingTreeExcluded": true},
            "categoryCount": 14,
            "categories": [{"category": 13, "stats": {
                "relationalSchemaTables": 112, "relationalSchemaColumns": 598,
                "relationalR2rmlTriplesMaps": 1, "relationalR2rmlPredicateObjectMaps": 2,
                "relationalR2rmlMappedTables": 1, "relationalR2rmlMappedColumns": 2
            }}],
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

fn synthetic_coverage() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "sourceRevision": SOURCE_REVISION,
        "relationalSchema": {"summary": {"tables": 112, "columns": 598},
            "stores": synthetic_stores()},
        "relationalR2rml": {"status":"partial","mappedTables":1,"mappedColumns":2,
            "unmappedTables":111,"unmappedColumns":596,"bindings":[{"store":"ProductDesign",
            "table":"style","sourcePath":INITIAL_MIGRATION,"columns":[{"column":"style_number"},{"column":"version"}]}]}
    }))
    .expect("synthetic coverage serializes")
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

fn synthetic_mapping() -> &'static str {
    r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://w3id.org/rml/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

<https://hm.com/ns/semantic-product-mock/source-map/relational/ProductDesign/style> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "style" ] ;
  rr:subjectMap [ rr:template "https://hm.com/ns/semantic-product-mock/resource/product-design/Style/{style_number}" ; rr:termType rr:IRI ; rr:class <https://hm.com/ns/semantic-product-mock/product-design/Style> ] ;
  rr:predicateObjectMap
    [ rr:predicate <https://hm.com/ns/semantic-product-mock/product-design/Style/field/StyleNumber> ; rr:objectMap [ rr:column "style_number" ; rr:datatype xsd:string ] ],
    [ rr:predicate <https://hm.com/ns/semantic-product-mock/product-design/Style/field/Version> ; rr:objectMap [ rr:column "version" ; rr:datatype xsd:integer ] ] .

<https://example.invalid/first-rml-map> a rml:TriplesMap ;
  rml:logicalSource [ rml:source <https://example.invalid/source> ] .
"#
}

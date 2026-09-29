//! Unforgeable mapping/ontology admission receipt for runtime construction.

use oxrdf::{Graph, NamedNode, NamedOrBlankNodeRef, TermRef};
use sf_core::ir::LogicalSource;
use sf_core::{datatype, SourceMapping};
use sf_sparql::{MappingDigest, OntologyDigest, SemanticAdmissionDigest};
use sf_sql::backend::mysql::{mysql_natural_xsd, MysqlTypeProfile};
use sha2::{Digest, Sha256};

use crate::{BackendKind, IntrospectedSource, SemanticOntology};

#[allow(dead_code)]
#[path = "generated_mapping_coverage.rs"]
pub(crate) mod generated_mapping_coverage;

const ADMISSION_DOMAIN: &[u8] = b"semantic-fabric/m-join-t-admission/v2";
const PROJECTION_DOMAIN: &[u8] = b"semantic-fabric/m-join-t-projection/v1";

#[derive(Clone, Copy)]
enum WarningPolicy {
    AdmitAndCount,
}

impl WarningPolicy {
    const fn admits(self, outcome: sf_validation::GateOutcome) -> bool {
        match self {
            Self::AdmitAndCount => outcome.violations == 0,
        }
    }

    const fn tag(self) -> &'static [u8] {
        match self {
            Self::AdmitAndCount => b"admit-and-count/v1",
        }
    }
}

const WARNING_POLICY: WarningPolicy = WarningPolicy::AdmitAndCount;

/// Mapping provenance is part of admission identity even when two origins
/// happen to project to the same executable IR.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MappingOrigin {
    Authored,
    Direct,
}

/// Mapping plus the receipt produced only by the sealed product gate.
pub(crate) struct ValidatedMapping {
    mapping: SourceMapping,
    origin: MappingOrigin,
    mapping_digest: MappingDigest,
    ontology_digest: OntologyDigest,
    admission_digest: SemanticAdmissionDigest,
    projection_digest: [u8; 32],
    warnings: usize,
}

impl ValidatedMapping {
    pub(crate) fn validate(
        mapping: SourceMapping,
        origin: MappingOrigin,
        ontology: &SemanticOntology,
        source: &IntrospectedSource,
    ) -> Result<Self, SemanticAdmissionError> {
        let projection = project_for_source(&mapping, source)?;
        let closure = join_graphs(ontology.graph(), &projection)?;
        let outcome = sf_validation::validate_graph(&closure).map_err(map_gate_error)?;
        if !WARNING_POLICY.admits(outcome) {
            return Err(SemanticAdmissionError::Violations {
                count: outcome.violations,
            });
        }
        let ontology_digest = OntologyDigest::from_sha256(ontology.document_digest());
        let projection_digest = graph_digest(&projection);
        let validation_policy_digest =
            sf_validation::validation_policy_digest().map_err(map_gate_error)?;
        let admission_digest = SemanticAdmissionDigest::from_sha256(admission_digest(
            ontology.document_digest(),
            validation_policy_digest,
            &projection,
            origin,
            outcome,
        ));
        let mapping_digest = MappingDigest::from_mapping(&mapping);
        Ok(Self {
            mapping,
            origin,
            mapping_digest,
            ontology_digest,
            admission_digest,
            projection_digest,
            warnings: outcome.warnings,
        })
    }

    /// Run schema-independent class, predicate, and already-known datatype
    /// checks before connector I/O. This cannot mint a runtime receipt because
    /// implicit column datatypes still require a coherent source observation.
    pub(crate) fn preflight(
        mapping: &SourceMapping,
        ontology: &SemanticOntology,
    ) -> Result<(), SemanticAdmissionError> {
        let projection =
            sf_mapping::project_static_to_rdf(mapping).map_err(map_projection_error)?;
        let closure = join_graphs(ontology.graph(), &projection)?;
        let outcome = sf_validation::validate_graph(&closure).map_err(map_gate_error)?;
        if WARNING_POLICY.admits(outcome) {
            Ok(())
        } else {
            Err(SemanticAdmissionError::Violations {
                count: outcome.violations,
            })
        }
    }

    pub(crate) const fn source_id(&self) -> sf_core::SourceId {
        self.mapping.source_id()
    }

    pub(crate) const fn origin(&self) -> MappingOrigin {
        self.origin
    }

    pub(crate) const fn mapping_digest(&self) -> MappingDigest {
        self.mapping_digest
    }

    #[allow(dead_code)]
    pub(crate) fn coverage(&self) -> generated_mapping_coverage::MappingCoverage<'_> {
        generated_mapping_coverage::MappingCoverage::new(&self.mapping)
    }

    pub(crate) fn ensure_context(
        self,
        ontology: &SemanticOntology,
        source: &IntrospectedSource,
    ) -> Result<Self, SemanticAdmissionError> {
        if self.ontology_digest != OntologyDigest::from_sha256(ontology.document_digest()) {
            return Err(SemanticAdmissionError::ReceiptOntologyMismatch);
        }
        if self.projection_digest != graph_digest(&project_for_source(&self.mapping, source)?) {
            return Err(SemanticAdmissionError::ReceiptSourceMismatch);
        }
        Ok(self)
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        SourceMapping,
        OntologyDigest,
        SemanticAdmissionDigest,
        usize,
    ) {
        (
            self.mapping,
            self.ontology_digest,
            self.admission_digest,
            self.warnings,
        )
    }
}

impl std::fmt::Debug for ValidatedMapping {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedMapping")
            .field("source_id", &self.source_id())
            .field("origin", &self.origin)
            .field("mapping_digest", &self.mapping_digest)
            .field("ontology_digest", &self.ontology_digest)
            .field("admission_digest", &self.admission_digest)
            .field("projection_digest", &self.projection_digest)
            .field("warnings", &self.warnings)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SemanticAdmissionError {
    #[error("mapping has a non-constant predicate target")]
    DynamicPredicate,
    #[error("mapping predicate target is invalid")]
    InvalidPredicate,
    #[error("semantic admission input exceeds its limit")]
    Limit,
    #[error("semantic admission validation failed")]
    Validation,
    #[error("ontology collides with the reserved mapping-projection namespace")]
    ProvenanceCollision,
    #[error("mapping implicit literal datatype cannot be proven from the source")]
    ImplicitDatatypeUnresolved,
    #[error("semantic admission receipt does not belong to this ontology")]
    ReceiptOntologyMismatch,
    #[error("semantic admission receipt does not match this source-derived mapping projection")]
    ReceiptSourceMismatch,
    #[error("semantic admission receipt does not match this verified source generation")]
    ReceiptGenerationMismatch,
    #[error("mapping and ontology are incompatible ({count} violations)")]
    Violations { count: usize },
}

fn project_for_source(
    mapping: &SourceMapping,
    source: &IntrospectedSource,
) -> Result<Graph, SemanticAdmissionError> {
    let kind = source.kind();
    let schema = source.observed_schema();
    sf_mapping::project_to_rdf(mapping, |logical_source, column| {
        effective_column_datatype(kind, schema, logical_source, column)
    })
    .map_err(map_projection_error)
}

fn effective_column_datatype(
    kind: BackendKind,
    schema: &[sf_core::TableSchema],
    source: &LogicalSource,
    column: &str,
) -> Option<NamedNode> {
    let LogicalSource::Table(table_name) = source else {
        // rr:sqlQuery result types are execution-statement facts, not facts in
        // the base-table catalogue snapshot. Require an explicit rr:datatype
        // until a coherent result-schema probe is sealed for each dialect.
        return None;
    };
    let mut tables = schema.iter().filter(|table| table.name == *table_name);
    let table = tables.next()?;
    if tables.next().is_some() {
        return None;
    }
    let mut columns = table
        .columns
        .iter()
        .filter(|candidate| candidate.name == column);
    let column = columns.next()?;
    if columns.next().is_some() {
        return None;
    }
    let code = match kind {
        BackendKind::MySql => mysql_natural_xsd(&column.sql_type, MysqlTypeProfile::Native),
        BackendKind::Sqlite | BackendKind::Postgres => datatype::natural_xsd(&column.sql_type),
    }?;
    Some(code.iri().into_owned())
}

fn map_projection_error(error: sf_mapping::ProjectionError) -> SemanticAdmissionError {
    match error {
        sf_mapping::ProjectionError::DynamicPredicate => SemanticAdmissionError::DynamicPredicate,
        sf_mapping::ProjectionError::InvalidPredicate => SemanticAdmissionError::InvalidPredicate,
        sf_mapping::ProjectionError::TripleLimit | sf_mapping::ProjectionError::ByteLimit => {
            SemanticAdmissionError::Limit
        }
        sf_mapping::ProjectionError::ImplicitDatatypeUnresolved => {
            SemanticAdmissionError::ImplicitDatatypeUnresolved
        }
    }
}

fn join_graphs(ontology: &Graph, mapping: &Graph) -> Result<Graph, SemanticAdmissionError> {
    if ontology.iter().any(|triple| {
        matches!(triple.subject, NamedOrBlankNodeRef::NamedNode(node) if is_projection_node(node.as_str()))
            || is_projection_node(triple.predicate.as_str())
            || matches!(triple.object, TermRef::NamedNode(node) if is_projection_node(node.as_str()))
    }) {
        return Err(SemanticAdmissionError::ProvenanceCollision);
    }
    let mut closure = ontology.clone();
    for triple in mapping.iter() {
        closure.insert(triple);
        if closure.len() > sf_validation::DEFAULT_GRAPH_LIMITS.max_parsed_triples {
            return Err(SemanticAdmissionError::Limit);
        }
    }
    Ok(closure)
}

fn is_projection_node(iri: &str) -> bool {
    iri.starts_with(sf_mapping::MAPPING_PROJECTION_NODE_PREFIX)
}

fn map_gate_error(error: sf_validation::GateError) -> SemanticAdmissionError {
    match error {
        sf_validation::GateError::ByteLimit
        | sf_validation::GateError::TripleLimit
        | sf_validation::GateError::ValidationWorkLimit
        | sf_validation::GateError::ValidationResultLimit => SemanticAdmissionError::Limit,
        sf_validation::GateError::InvalidTurtle
        | sf_validation::GateError::InvalidShapeSet
        | sf_validation::GateError::ValidationFailed
        | sf_validation::GateError::ValidationPanicked => SemanticAdmissionError::Validation,
    }
}

fn admission_digest(
    ontology_digest: [u8; 32],
    validation_policy_digest: [u8; 32],
    projection: &Graph,
    origin: MappingOrigin,
    outcome: sf_validation::GateOutcome,
) -> [u8; 32] {
    // The projection is ground by construction. Sorted, deduplicated N-Triples
    // is therefore a complete graph identity without a blank-node algorithm.
    let triples = canonical_triples(projection);

    let mut hasher = Sha256::new();
    hash_bytes(&mut hasher, ADMISSION_DOMAIN);
    hash_bytes(&mut hasher, &ontology_digest);
    hash_bytes(&mut hasher, &validation_policy_digest);
    hasher.update([match origin {
        MappingOrigin::Authored => 0,
        MappingOrigin::Direct => 1,
    }]);
    hash_bytes(&mut hasher, WARNING_POLICY.tag());
    hasher.update(
        u64::try_from(outcome.violations)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hasher.update(
        u64::try_from(outcome.warnings)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hasher.update(
        u64::try_from(triples.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for triple in triples {
        hash_bytes(&mut hasher, triple.as_bytes());
    }
    hasher.finalize().into()
}

fn graph_digest(graph: &Graph) -> [u8; 32] {
    let triples = canonical_triples(graph);
    let mut hasher = Sha256::new();
    hash_bytes(&mut hasher, PROJECTION_DOMAIN);
    hasher.update(
        u64::try_from(triples.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for triple in triples {
        hash_bytes(&mut hasher, triple.as_bytes());
    }
    hasher.finalize().into()
}

fn canonical_triples(graph: &Graph) -> Vec<String> {
    let mut triples = graph
        .iter()
        .map(|triple| triple.to_string())
        .collect::<Vec<_>>();
    triples.sort_unstable();
    triples.dedup();
    triples
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod digest_tests {
    use super::*;

    fn effective(kind: BackendKind, sql_type: &str) -> Option<NamedNode> {
        let mut table = sf_core::TableSchema::new("items");
        table.columns = vec![sf_core::Column::new("value", sql_type, false)];
        effective_column_datatype(
            kind,
            &[table],
            &LogicalSource::Table("items".into()),
            "value",
        )
    }

    #[test]
    fn validation_policy_alone_partitions_admission_identity() {
        let graph = Graph::new();
        let outcome = sf_validation::GateOutcome {
            violations: 0,
            warnings: 0,
        };
        let first = admission_digest([7; 32], [11; 32], &graph, MappingOrigin::Authored, outcome);
        let second = admission_digest([7; 32], [12; 32], &graph, MappingOrigin::Authored, outcome);
        assert_ne!(first, second);

        let source_id = sf_core::SourceId::new(0).unwrap();
        let binding = |admission| {
            sf_sparql::CompilerBinding::from_observation_with_semantic_identity(
                SourceMapping::new(source_id, Vec::new()),
                sf_sql::Dialect::Sqlite,
                sf_sparql::Tbox::default(),
                Vec::new(),
                sf_sparql::Epoch(0),
                sf_sparql::SemanticIdentity::new(
                    OntologyDigest::from_sha256([7; 32]),
                    SemanticAdmissionDigest::from_sha256(admission),
                ),
                1,
            )
        };
        let first_binding = binding(first);
        let second_binding = binding(second);
        assert_ne!(first_binding.scope(), second_binding.scope());
        assert_ne!(
            first_binding.scope().digests().semantic_admission(),
            second_binding.scope().digests().semantic_admission()
        );
    }

    #[test]
    fn mysql_table_admission_uses_the_native_execution_type_law() {
        assert_eq!(
            effective(BackendKind::MySql, "tinyint(1)"),
            Some(oxrdf::vocab::xsd::INTEGER.into_owned())
        );
        assert_eq!(
            effective(BackendKind::MySql, "bit(8)"),
            Some(oxrdf::vocab::xsd::HEX_BINARY.into_owned())
        );
        assert_eq!(
            effective(BackendKind::MySql, "datetime(6)"),
            Some(oxrdf::vocab::xsd::DATE_TIME.into_owned())
        );
    }

    #[test]
    fn mysql_only_spellings_do_not_change_other_dialect_admission() {
        for kind in [BackendKind::Sqlite, BackendKind::Postgres] {
            assert_eq!(effective(kind, "tinyint(1)"), None);
            assert_eq!(effective(kind, "bit(8)"), None);
            assert_eq!(effective(kind, "datetime(6)"), None);
        }
    }

    #[test]
    fn mysql_query_source_stays_unresolved_without_statement_type_evidence() {
        let mut table = sf_core::TableSchema::new("items");
        table.columns = vec![sf_core::Column::new("value", "integer", false)];
        assert_eq!(
            effective_column_datatype(
                BackendKind::MySql,
                &[table],
                &LogicalSource::Query("SELECT value FROM items".into()),
                "value",
            ),
            None
        );
    }

    #[test]
    fn mysql_catalog_type_change_changes_projection_identity() {
        let mapping = sf_mapping::parse_r2rml_for_source(
            "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n\
             <#m> rr:logicalTable [ rr:tableName \"items\" ];\n\
             rr:subjectMap [ rr:constant <http://example.test/item> ];\n\
             rr:predicateObjectMap [ rr:predicate <http://example.test/value>;\n\
             rr:objectMap [ rr:column \"value\" ] ] .",
            sf_core::SourceId::new(0).unwrap(),
        )
        .unwrap();
        let project = |sql_type| {
            let mut table = sf_core::TableSchema::new("items");
            table.columns = vec![sf_core::Column::new("value", sql_type, false)];
            sf_mapping::project_to_rdf(&mapping, |source, column| {
                effective_column_datatype(BackendKind::MySql, &[table.clone()], source, column)
            })
            .unwrap()
        };

        assert_ne!(
            graph_digest(&project("tinyint(1)")),
            graph_digest(&project("bit(8)"))
        );
    }
}

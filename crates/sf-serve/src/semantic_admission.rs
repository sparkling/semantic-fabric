//! Unforgeable mapping/ontology admission receipt for runtime construction.

use oxrdf::{Graph, NamedNode, NamedOrBlankNodeRef, TermRef};
use sf_core::ir::LogicalSource;
use sf_core::{datatype, SourceMapping};
use sf_sparql::{OntologyDigest, SemanticAdmissionDigest};
use sha2::{Digest, Sha256};

use crate::{BackendKind, IntrospectedSource, SemanticOntology};

const ADMISSION_DOMAIN: &[u8] = b"semantic-fabric/m-join-t-admission/v1";
const PROJECTION_DOMAIN: &[u8] = b"semantic-fabric/m-join-t-projection/v1";

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
        if !outcome.conforms() {
            return Err(SemanticAdmissionError::Violations {
                count: outcome.violations,
            });
        }
        let ontology_digest = OntologyDigest::from_sha256(ontology.document_digest());
        let projection_digest = graph_digest(&projection);
        let admission_digest = SemanticAdmissionDigest::from_sha256(admission_digest(
            ontology.document_digest(),
            &projection,
            origin,
            outcome,
        ));
        Ok(Self {
            mapping,
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
        if outcome.conforms() {
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
    // The current MySQL executor intentionally has no per-column wire-type
    // channel and reconstructs every implicit literal as xsd:string.
    if kind == BackendKind::MySql {
        return Some(oxrdf::vocab::xsd::STRING.into_owned());
    }
    let LogicalSource::Table(table_name) = source else {
        // PostgreSQL/SQLite rr:sqlQuery result types are execution-statement
        // facts, not facts in the base-table catalogue snapshot. Require an
        // explicit rr:datatype until a coherent result-schema probe is sealed.
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
    datatype::natural_xsd(&column.sql_type).map(|code| code.iri().into_owned())
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
    hash_bytes(&mut hasher, &sf_validation::shape_set_digest());
    hasher.update([match origin {
        MappingOrigin::Authored => 0,
        MappingOrigin::Direct => 1,
    }]);
    hasher.update([0]); // warning policy v1: warnings are admitted and counted.
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

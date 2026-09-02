//! Source identity and mapping affinity without changing the R2RML IR.
//!
//! [`TriplesMap`] remains a source-local semantic model:
//! its [`LogicalSource`](crate::ir::LogicalSource) names a relation, not a
//! connection. [`SourceMapping`] is a source-affinity sidecar that associates one
//! immutable mapping bundle with one opaque [`SourceId`]. It does not itself own
//! schema, backend or cache state; the current single-source serve lane composes
//! those through `sf_serve::RuntimeBinding`. Digest-addressed snapshots, registry
//! membership and federation remain future runtime work.

use std::fmt;

use crate::ir::TriplesMap;

/// Largest registry slot representable by a [`SourceId`].
pub const MAX_SOURCE_INDEX: usize = u16::MAX as usize;

/// An opaque, non-secret source registry index within one runtime snapshot.
///
/// It is deliberately a fixed-width index rather than a name, connection
/// string, hostname, or token. That keeps its value domain and diagnostic
/// representation bounded. It is meaningful only with the immutable registry
/// that assigned it and is not a persistent source identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct SourceId(u16);

impl SourceId {
    /// Construct an ID for `index`, rejecting values outside the fixed domain.
    pub fn new(index: usize) -> Result<Self, SourceIdError> {
        let index = u16::try_from(index).map_err(|_| SourceIdError::OutOfRange {
            max: MAX_SOURCE_INDEX,
        })?;
        Ok(Self(index))
    }

    /// Return the snapshot-local registry index.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "source[{}]", self.index())
    }
}

impl TryFrom<usize> for SourceId {
    type Error = SourceIdError;

    fn try_from(index: usize) -> Result<Self, Self::Error> {
        Self::new(index)
    }
}

/// Why a registry slot could not be represented as a [`SourceId`].
///
/// Error messages never echo the rejected value, keeping diagnostics bounded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SourceIdError {
    /// The index cannot be represented by the fixed-width source domain.
    #[error("source registry index exceeds the maximum of {max}")]
    OutOfRange { max: usize },
}

/// One immutable R2RML mapping bundle associated with one source identity.
///
/// This owned sidecar preserves the existing `Vec<TriplesMap>` representation
/// and exposes it by shared slice, so current compilers and executors do not need
/// a parallel mapping IR. Separate bundles may share a [`SourceId`]; source
/// uniqueness and registry membership belong to the future runtime snapshot.
#[derive(Clone)]
pub struct SourceMapping {
    source_id: SourceId,
    triples_maps: Vec<TriplesMap>,
}

impl SourceMapping {
    /// Associate a validated source identity with an R2RML mapping bundle.
    pub fn new(source_id: SourceId, triples_maps: Vec<TriplesMap>) -> Self {
        Self {
            source_id,
            triples_maps,
        }
    }

    /// The source identity for every triples map in this bundle.
    pub const fn source_id(&self) -> SourceId {
        self.source_id
    }

    /// The unchanged R2RML IR consumed by the existing semantic compiler.
    pub fn triples_maps(&self) -> &[TriplesMap] {
        &self.triples_maps
    }

    /// Consume the sidecar and recover its source identity and unchanged IR.
    pub fn into_parts(self) -> (SourceId, Vec<TriplesMap>) {
        (self.source_id, self.triples_maps)
    }

    /// Return the number of triples maps in this source-local bundle.
    pub fn len(&self) -> usize {
        self.triples_maps.len()
    }

    /// Return whether this source-local bundle contains no triples maps.
    pub fn is_empty(&self) -> bool {
        self.triples_maps.is_empty()
    }
}

impl fmt::Debug for SourceMapping {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceMapping")
            .field("source_id", &self.source_id)
            .field("triples_map_count", &self.triples_maps.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{LogicalSource, SubjectMap, TermMap};
    use crate::{NamedNode, Term};

    fn mapping_with_query(query: &str) -> TriplesMap {
        TriplesMap {
            id: "http://example.com/mapping/items".to_owned(),
            source: LogicalSource::Query(query.to_owned()),
            subject: SubjectMap {
                term: TermMap::Constant(Term::NamedNode(NamedNode::new_unchecked(
                    "http://example.com/item",
                ))),
                classes: vec![],
                graphs: vec![],
            },
            predicate_object_maps: vec![],
        }
    }

    #[test]
    fn source_id_is_a_bounded_snapshot_local_registry_index() {
        let first = SourceId::new(0).unwrap();
        let last = SourceId::new(MAX_SOURCE_INDEX).unwrap();

        assert_eq!(first.index(), 0);
        assert_eq!(last.index(), MAX_SOURCE_INDEX);
        assert_eq!(last.to_string(), format!("source[{MAX_SOURCE_INDEX}]"));
    }

    #[test]
    fn source_id_rejects_an_index_outside_its_fixed_domain() {
        assert_eq!(
            SourceId::new(MAX_SOURCE_INDEX + 1).unwrap_err(),
            SourceIdError::OutOfRange {
                max: MAX_SOURCE_INDEX
            }
        );
    }

    #[test]
    fn source_id_error_does_not_echo_the_rejected_index() {
        let rejected = MAX_SOURCE_INDEX + 73;
        let error = SourceId::new(rejected).unwrap_err().to_string();
        assert!(!error.contains(&rejected.to_string()));
    }

    #[test]
    fn source_mapping_round_trips_ir_without_debugging_source_sql() {
        let source_id = SourceId::new(7).unwrap();
        let source_mapping = SourceMapping::new(
            source_id,
            vec![mapping_with_query(
                "SELECT secret_column FROM private_table",
            )],
        );

        assert_eq!(source_mapping.source_id(), source_id);
        assert_eq!(source_mapping.len(), 1);
        assert!(!source_mapping.is_empty());
        assert!(format!("{source_mapping:?}").contains("triples_map_count: 1"));
        assert!(!format!("{source_mapping:?}").contains("secret_column"));

        let (round_trip_id, maps) = source_mapping.into_parts();
        assert_eq!(round_trip_id, source_id);
        assert!(matches!(
            &maps[0].source,
            LogicalSource::Query(query) if query == "SELECT secret_column FROM private_table"
        ));
    }
}

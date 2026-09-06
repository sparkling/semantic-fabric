//! Semantic-identity construction and redacted diagnostics for compiler bindings.

use std::fmt;

use sf_core::SourceMapping;
use sf_sql::{Dialect, TableSchema};

use super::{CompilerBinding, Epoch};
use crate::compiler_schema::{ColumnTypeAuthority, CompilerSchema, ConstraintAuthority};
use crate::runtime_identity::{CompileDigests, SemanticIdentity};
use crate::Tbox;

impl CompilerBinding {
    /// Build a binding with externally supplied semantic identity. The exact
    /// document and admission digests partition every plan/cache entry, but the
    /// caller remains responsible for enforcing its opaque admission boundary.
    pub fn from_observation_with_semantic_identity(
        mapping: SourceMapping,
        dialect: Dialect,
        tbox: Tbox,
        observed_schema: Vec<TableSchema>,
        epoch: Epoch,
        semantic: SemanticIdentity,
        cache_capacity: usize,
    ) -> Self {
        assert!(cache_capacity > 0, "plan cache capacity must be non-zero");
        let constraint_authority = ConstraintAuthority::Unverified;
        let column_type_authority = ColumnTypeAuthority::Unverified;
        let digests = CompileDigests::from_inputs_with_semantic_identity(
            &mapping,
            &observed_schema,
            dialect,
            constraint_authority,
            column_type_authority,
            semantic,
        );
        let schema = CompilerSchema::from_unverified_observation(observed_schema);
        Self::from_parts(
            mapping,
            dialect,
            tbox,
            schema,
            epoch,
            digests,
            cache_capacity,
        )
    }
}

impl fmt::Debug for CompilerBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompilerBinding")
            .field("scope", &self.scope)
            .field("source_id", &self.source_id())
            .field("dialect", &self.dialect)
            .field("triples_map_count", &self.mapping.len())
            .field("schema", &self.schema)
            .field("tbox_empty", &self.tbox.is_empty())
            .field("cache_entries", &self.cache().len())
            .finish()
    }
}

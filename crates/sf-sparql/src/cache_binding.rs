//! Bound compiler work control, semantic identity and redacted diagnostics.

use std::fmt;
use std::sync::Arc;

use sf_core::query_control::QueryControl;
use sf_core::SourceMapping;
use sf_sql::{Dialect, TableSchema};
use spargebra::Query;

use super::{CachedPlan, CompileProfileId, CompilerBinding, Epoch};
use crate::compiler_control::CompileContext;
use crate::compiler_schema::{ColumnTypeAuthority, CompilerSchema, ConstraintAuthority};
use crate::runtime_identity::{CompileDigests, SemanticIdentity};
use crate::{CompilerWorkMode, Error, Plan, Result, Tbox};

// Dormant structural screen; serving integration is a later slice.
#[allow(dead_code)]
#[path = "generated_query_shape.rs"]
mod generated_query_shape;

impl CompilerBinding {
    /// Carry request control through structural BUILD and the metered normalization, lowering
    /// and nested-cascade operations on a cache miss, plus canonical key
    /// output/growth/hash and acquired cache logical work on hits and misses.
    /// Cache locks are attempted once:
    /// contention is a miss/skipped insertion, never a wait or a query error.
    /// Parsing, formatter internals and physical allocation/last-Arc destruction
    /// are not fully governed by this seam. BUILD controls visits, logical collection
    /// growth, stable scope comparisons and actual owned copies, not physical heap/drop.
    /// Cache identity and semantics are unchanged: this does not activate
    /// `GovernedV1`. A shared hit performs no recursive plan clone to charge.
    /// Measurement limits protect each performed clone, not whole-plan admission.
    /// The iterative measurement itself pays traversal/logical stack work and
    /// observes cancellation, separately from reserving the measured clone payload.
    pub fn compile_shared_with_work_control(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>> {
        control.checkpoint()?;
        let query = crate::parse_query(sparql)?;
        self.compile_parsed_shared_with_work_control(&query, control)
    }

    /// Uncached counterpart for structural preflight. The request retains the
    /// same cumulative control for its later authoritative compilation.
    pub fn compile_uncached_shared_with_work_control(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>> {
        control.checkpoint()?;
        let query = crate::parse_query(sparql)?;
        self.compile_parsed_uncached_shared_with_work_control(&query, control)
    }

    pub(crate) fn compile_parsed_shared_with_work_control(
        &self,
        query: &Query,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>> {
        control.checkpoint()?;
        let profile = CompileProfileId::Uncontrolled;
        let key =
            super::bounded_key::plan_key_with_work_control(query, self.scope(), profile, control)?;
        control.checkpoint()?;
        let cached = self.cache().get_if_uncontended(&key, control)?;
        control.checkpoint()?;
        if let Some(cached) = cached {
            if cached.scope() != self.scope() || cached.profile() != profile {
                return Err(Error::Mapping(
                    "compiled-plan cache identity mismatch".into(),
                ));
            }
            control.checkpoint()?;
            return Ok(cached.shared_plan());
        }
        let plan = self.compile_parsed_uncached_shared_with_work_control(query, control)?;
        control.checkpoint()?;
        self.cache().put_if_uncontended(
            key,
            CachedPlan::from_shared(self.scope(), profile, Arc::clone(&plan)),
            control,
        )?;
        control.checkpoint()?;
        Ok(plan)
    }

    pub(crate) fn compile_parsed_uncached_shared_with_work_control(
        &self,
        query: &Query,
        control: &dyn QueryControl,
    ) -> Result<Arc<Plan>> {
        control.checkpoint()?;
        let result = self.compile_parsed_with_work_mode(
            query,
            CompilerWorkMode::Metered(CompileContext::new(control)),
        );
        control.checkpoint()?;
        result
    }

    pub(super) fn compile_parsed_with_work_mode(
        &self,
        query: &Query,
        work: CompilerWorkMode<'_>,
    ) -> Result<Arc<Plan>> {
        crate::translate_tree_with_column_type_use(
            query,
            self.triples_maps(),
            self.tbox(),
            self.dialect(),
            self.schema(),
            self.column_type_use(),
            work,
        )
        .map(Arc::new)
    }

    /// Prove the constant mapping origin of every emitted solution for the
    /// initial on-demand lineage profile. This is a query/input proof, NOT an
    /// execution capability for an independently mutable `Plan`.
    ///
    /// Exactly one authored map, no referencing object maps, and nonempty
    /// mandatory BGPs (possibly joined/unioned/projected/deduplicated/sliced).
    /// Saturation preserves the authored map identity. Every witness therefore
    /// has this same origin, even where semantic dedup erases witness identity.
    /// Other shapes reject; never substitute a list of candidate maps.
    pub fn constant_mapping_origin(
        &self,
        sparql: &str,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> crate::Result<String> {
        use sf_core::{ir::ObjectMap, query_control::QueryCharge};
        use spargebra::{algebra::GraphPattern, Query};
        let unsupported = || crate::Error::Unsupported("lineage profile is not admitted".into());
        control.checkpoint()?;
        let [map] = self.mapping.triples_maps() else {
            return Err(unsupported());
        };
        if map.id.is_empty()
            || map.id.len() > 1024
            || map
                .predicate_object_maps
                .iter()
                .flat_map(|pom| &pom.objects)
                .any(|object| matches!(object, ObjectMap::Ref(_)))
        {
            return Err(unsupported());
        }
        let query = crate::parse_query(sparql)?;
        let (pattern, mut work) = match &query {
            Query::Select {
                pattern,
                dataset: None,
                ..
            } => (pattern, 0),
            Query::Construct {
                pattern,
                dataset: None,
                template,
                ..
            } => {
                // Template size is part of lineage eligibility, not merely WHERE
                // algebra size. This is not a total parser/term-recursion bound.
                if template.len() > 256 {
                    return Err(unsupported());
                }
                control.consume(QueryCharge::CompilerWork, template.len() as u64)?;
                (pattern, template.len())
            }
            _ => return Err(unsupported()),
        };
        let mut pending = vec![pattern];
        while let Some(pattern) = pending.pop() {
            control.checkpoint()?;
            control.consume(QueryCharge::CompilerWork, 1)?;
            work += 1;
            if work > 256 {
                return Err(unsupported());
            }
            match pattern {
                GraphPattern::Bgp { patterns } if !patterns.is_empty() => {
                    work = work.saturating_add(patterns.len());
                    if work > 256 {
                        return Err(unsupported());
                    }
                    control.consume(QueryCharge::CompilerWork, patterns.len() as u64)?;
                }
                GraphPattern::Project { inner, .. }
                | GraphPattern::Distinct { inner }
                | GraphPattern::Reduced { inner }
                | GraphPattern::Slice { inner, .. } => pending.push(inner),
                GraphPattern::Join { left, right } | GraphPattern::Union { left, right } => {
                    pending.push(left);
                    pending.push(right);
                }
                _ => return Err(unsupported()),
            }
        }
        Ok(map.id.clone())
    }

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

#[cfg(test)]
#[path = "cache_binding_work_tests.rs"]
mod work_tests;

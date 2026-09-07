//! Narrow, source-affine federation primitives.
//!
//! This module admits only a top-level two-arm `SELECT ... UNION ...` whose
//! arms each contain exactly one triple pattern. Each arm is still compiled by
//! an unchanged source-local [`Plan`]; [`FederatedPlan`] only records that the
//! two resulting bags are concatenated by sequential streaming. This is the
//! non-blocking merge shape accepted by ADR-0006; it adds no blocking global
//! operator, spill substrate, general federated algebra, or in-process join
//! engine, and therefore does not treat proposed ADR-0040 as accepted.

use std::sync::Arc;

use sf_core::ir::TermMap;
use sf_core::query_control::QueryControl;
use sf_core::{SourceId, Term};
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TriplePattern, Variable};
use spargebra::{Query, SparqlParser};

use crate::{CompilerBinding, Error, Plan, PlanForm, Result};

#[allow(dead_code)] // Private ADR-0040 comparison prototype; not serving admission.
mod global;

/// One parsed source-local arm of the narrow federated UNION profile.
#[derive(Clone, Debug)]
pub(crate) struct SourceAffineUnionArm {
    query: Query,
    triple: TriplePattern,
}

impl SourceAffineUnionArm {
    pub(crate) const fn query(&self) -> &Query {
        &self.query
    }
}

/// A SPARQL query parsed once and proven to be the narrow two-arm profile.
#[derive(Clone, Debug)]
struct SourceAffineUnion {
    arms: [SourceAffineUnionArm; 2],
    variables: Vec<String>,
}

impl SourceAffineUnion {
    /// Parse once, reject every global or composite operator outside the first
    /// bounded federation vertical, and construct two source-local SELECTs.
    fn parse(sparql: &str) -> Result<Self> {
        let query = SparqlParser::new()
            .parse_query(sparql)
            .map_err(|error| Error::Parse(error.to_string()))?;
        Self::from_query(query)
    }

    fn from_query(query: Query) -> Result<Self> {
        let Query::Select {
            dataset,
            pattern,
            base_iri,
        } = query
        else {
            return unsupported();
        };
        if dataset.is_some() {
            return unsupported();
        }
        let GraphPattern::Project { inner, variables } = pattern else {
            return unsupported();
        };
        let GraphPattern::Union { left, right } = *inner else {
            return unsupported();
        };
        validate_arm(&left)?;
        validate_arm(&right)?;

        let variable_names = variables
            .iter()
            .map(Variable::as_str)
            .map(str::to_owned)
            .collect();
        let arm = |inner: Box<GraphPattern>| SourceAffineUnionArm {
            triple: match inner.as_ref() {
                GraphPattern::Bgp { patterns } => patterns[0].clone(),
                _ => unreachable!("validated source-affine arm"),
            },
            query: Query::Select {
                dataset: None,
                pattern: GraphPattern::Project {
                    inner,
                    variables: variables.clone(),
                },
                base_iri: base_iri.clone(),
            },
        };
        Ok(Self {
            arms: [arm(left), arm(right)],
            variables: variable_names,
        })
    }

    const fn arms(&self) -> &[SourceAffineUnionArm; 2] {
        &self.arms
    }

    fn variables(&self) -> &[String] {
        &self.variables
    }
}

fn validate_arm(pattern: &GraphPattern) -> Result<()> {
    match pattern {
        GraphPattern::Bgp { patterns } if patterns.len() == 1 => Ok(()),
        _ => unsupported(),
    }
}

fn unsupported<T>() -> Result<T> {
    Err(Error::Unsupported(
        "federation admits only a two-source SELECT UNION with one triple pattern per arm"
            .to_owned(),
    ))
}

/// One unchanged source-local plan bound to its snapshot-local source identity.
#[derive(Clone, Debug)]
pub struct SourceFragment {
    source_id: SourceId,
    plan: Arc<Plan>,
}

impl SourceFragment {
    fn new(source_id: SourceId, plan: Arc<Plan>) -> Result<Self> {
        if !source_local_union_arm(&plan) {
            return unsupported();
        }
        Ok(Self { source_id, plan })
    }

    pub const fn source_id(&self) -> SourceId {
        self.source_id
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn shared_plan(&self) -> Arc<Plan> {
        Arc::clone(&self.plan)
    }
}

fn source_local_union_arm(plan: &Plan) -> bool {
    matches!(plan.form, PlanForm::Select { .. })
        && !plan.distinct
        && plan.limit.is_none()
        && plan.offset == 0
        && plan.order.is_empty()
        && plan.rust_group.is_none()
}

/// The only admitted global operator: concatenate two source-local bags.
#[derive(Clone, Debug)]
pub struct FederatedPlan {
    variables: Vec<String>,
    fragments: [SourceFragment; 2],
}

impl FederatedPlan {
    fn union_all(variables: Vec<String>, fragments: [SourceFragment; 2]) -> Result<Self> {
        if fragments[0].source_id == fragments[1].source_id
            || fragments
                .iter()
                .any(|fragment| select_variables(fragment.plan()) != Some(variables.as_slice()))
        {
            return unsupported();
        }
        Ok(Self {
            variables,
            fragments,
        })
    }

    pub fn variables(&self) -> &[String] {
        &self.variables
    }

    pub const fn fragments(&self) -> &[SourceFragment; 2] {
        &self.fragments
    }
}

/// Parse and compile the exact two-arm vertical using two immutable compiler
/// bindings. Fragment construction stays inside this crate, so no caller can
/// attach an arbitrary `SourceId` to a detached plan.
pub fn compile_source_affine_union(
    sparql: &str,
    bindings: [&CompilerBinding; 2],
    control: &dyn QueryControl,
) -> Result<FederatedPlan> {
    compile_source_affine_union_with(sparql, bindings, control, CompileMode::Cached)
}

/// Compile the exact two-arm profile without reading or populating either
/// source cache. Serving discards preflight results before generation admission,
/// then may invoke this again for authoritative protected UNION compilation
/// under the admitted generation lease and security context.
pub fn compile_source_affine_union_uncached(
    sparql: &str,
    bindings: [&CompilerBinding; 2],
    control: &dyn QueryControl,
) -> Result<FederatedPlan> {
    compile_source_affine_union_with(sparql, bindings, control, CompileMode::Uncached)
}

#[derive(Clone, Copy)]
enum CompileMode {
    Cached,
    Uncached,
}

fn compile_source_affine_union_with(
    sparql: &str,
    bindings: [&CompilerBinding; 2],
    control: &dyn QueryControl,
    mode: CompileMode,
) -> Result<FederatedPlan> {
    if bindings[0].source_id() == bindings[1].source_id() {
        return Err(Error::Mapping(
            "federated compiler bindings must have distinct source identities".to_owned(),
        ));
    }
    let parsed = crate::compiler_telemetry::in_stage(
        crate::compiler_telemetry::CompilerStage::Parse,
        || SourceAffineUnion::parse(sparql),
    )?;
    let mut selected = Vec::with_capacity(2);
    for arm in parsed.arms() {
        let candidates: Vec<_> = bindings
            .into_iter()
            .filter(|binding| binding_could_match(binding, &arm.triple))
            .collect();
        if candidates.len() != 1 {
            return Err(Error::Unsupported(
                "each federated UNION arm must resolve to exactly one source".to_owned(),
            ));
        }
        let binding = candidates[0];
        control.checkpoint()?;
        let plan = match mode {
            CompileMode::Cached => binding.compile_union_arm_shared(arm),
            CompileMode::Uncached => binding.compile_union_arm_uncached_shared(arm),
        }?;
        control.checkpoint()?;
        selected.push(SourceFragment::new(binding.source_id(), plan)?);
    }
    if selected[0].source_id() == selected[1].source_id() {
        return Err(Error::Unsupported(
            "federated UNION arms must resolve to distinct sources".to_owned(),
        ));
    }
    let right = selected.pop().expect("right UNION arm");
    let left = selected.pop().expect("left UNION arm");
    FederatedPlan::union_all(parsed.variables().to_vec(), [left, right])
}

fn binding_could_match(binding: &CompilerBinding, triple: &TriplePattern) -> bool {
    let saturated = crate::saturate::saturate_maps(binding.triples_maps(), binding.tbox());
    saturated
        .iter()
        .any(|mapping| mapping_could_emit(mapping, &triple.predicate))
}

fn mapping_could_emit(mapping: &sf_core::ir::TriplesMap, predicate: &NamedNodePattern) -> bool {
    match predicate {
        NamedNodePattern::Variable(_) => {
            !mapping.subject.classes.is_empty()
                || mapping
                    .predicate_object_maps
                    .iter()
                    .any(|pom| !pom.predicates.is_empty() && !pom.objects.is_empty())
        }
        NamedNodePattern::NamedNode(wanted) => {
            (wanted.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
                && !mapping.subject.classes.is_empty())
                || mapping.predicate_object_maps.iter().any(|pom| {
                    !pom.objects.is_empty()
                        && pom.predicates.iter().any(|predicate| match predicate {
                            TermMap::Constant(Term::NamedNode(node)) => node == wanted,
                            TermMap::Constant(_) => false,
                            TermMap::Column(_, _) | TermMap::Template(_, _) => true,
                        })
                })
        }
    }
}

fn select_variables(plan: &Plan) -> Option<&[String]> {
    match &plan.form {
        PlanForm::Select { vars } => Some(vars),
        PlanForm::Ask | PlanForm::Construct { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::query_control::UncontrolledQueryControl;
    use sf_core::SourceMapping;
    use sf_sql::Dialect;

    fn binding(index: usize, predicate: &str) -> CompilerBinding {
        let mapping = format!(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:constant <http://example.test/item> ] ;
  rr:predicateObjectMap [ rr:predicate <{predicate}> ; rr:objectMap [ rr:column "value" ] ] ."#
        );
        let source_id = SourceId::new(index).unwrap();
        CompilerBinding::from_unverified_observation(
            SourceMapping::new(source_id, sf_mapping::parse_r2rml(&mapping).unwrap()),
            Dialect::Sqlite,
            crate::Tbox::default(),
            vec![sf_sql::TableSchema::new("items")],
            crate::Epoch::default(),
            8,
        )
    }

    #[test]
    fn parses_only_the_exact_two_arm_select_profile() {
        let parsed = SourceAffineUnion::parse(
            "SELECT ?s ?left ?right WHERE { \
             { ?s <http://example.test/left> ?left } UNION \
             { ?s <http://example.test/right> ?right } }",
        )
        .unwrap();

        assert_eq!(parsed.variables(), ["s", "left", "right"]);
        assert_eq!(parsed.arms().len(), 2);
    }

    #[test]
    fn rejects_global_modifiers_non_select_and_composite_arms() {
        for query in [
            "SELECT ?s WHERE { { ?s <http://example.test/a> ?a } UNION { ?s <http://example.test/b> ?b } } ORDER BY ?s",
            "SELECT DISTINCT ?s WHERE { { ?s <http://example.test/a> ?a } UNION { ?s <http://example.test/b> ?b } }",
            "SELECT ?s (COUNT(?a) AS ?count) WHERE { { ?s <http://example.test/a> ?a } UNION { ?s <http://example.test/b> ?b } } GROUP BY ?s",
            "SELECT ?s WHERE { { ?s <http://example.test/a> ?a . ?s <http://example.test/c> ?c } UNION { ?s <http://example.test/b> ?b } }",
            "SELECT ?s WHERE { { ?s <http://example.test/a>+ ?a } UNION { ?s <http://example.test/b> ?b } }",
            "ASK { { ?s <http://example.test/a> ?a } UNION { ?s <http://example.test/b> ?b } }",
            "CONSTRUCT { ?s <http://example.test/a> ?a } WHERE { { ?s <http://example.test/a> ?a } UNION { ?s <http://example.test/b> ?b } }",
        ] {
            assert!(matches!(
                SourceAffineUnion::parse(query),
                Err(Error::Unsupported(_))
            ));
        }
    }

    #[test]
    fn compiles_only_the_statically_affine_owner_for_each_arm() {
        let left = binding(0, "http://example.test/left");
        let right = binding(1, "http://example.test/right");
        let plan = compile_source_affine_union(
            "SELECT ?value WHERE { \
             { ?s <http://example.test/left> ?value } UNION \
             { ?s <http://example.test/right> ?value } }",
            [&left, &right],
            &UncontrolledQueryControl,
        )
        .unwrap();

        assert_eq!(plan.fragments()[0].source_id(), SourceId::new(0).unwrap());
        assert_eq!(plan.fragments()[1].source_id(), SourceId::new(1).unwrap());
        assert_eq!(left.cache_len(), 1, "wrong source must not be compiled");
        assert_eq!(right.cache_len(), 1, "wrong source must not be compiled");
    }

    #[test]
    fn uncached_preflight_leaves_both_source_caches_empty() {
        let left = binding(0, "http://example.test/left");
        let right = binding(1, "http://example.test/right");
        let plan = compile_source_affine_union_uncached(
            "SELECT ?value WHERE { \
             { ?s <http://example.test/left> ?value } UNION \
             { ?s <http://example.test/right> ?value } }",
            [&left, &right],
            &UncontrolledQueryControl,
        )
        .unwrap();

        assert_eq!(plan.fragments().len(), 2);
        assert_eq!(left.cache_len(), 0);
        assert_eq!(right.cache_len(), 0);
    }
}

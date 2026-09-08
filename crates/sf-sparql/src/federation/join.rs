//! Sealed two-pattern bounded semi-join. No source-sized or spill operator.
use super::*;
use sf_core::ir::LogicalSource;
use sf_sql::cost::{plan_semijoin, CostConfig, SemiJoinPlan, Side, SideStats};
use spargebra::term::TermPattern;
use std::collections::BTreeSet;

#[cfg(test)]
mod admission_tests;
mod key;

/// One fixed retained driving batch; an extra row is an explicit resource
/// failure, never a truncated answer. Complete source triples have graph-set
/// identity; joined/projected solutions retain their full bag multiplicity.
pub const MAX_BUILD_ROWS: usize = 128;

/// Compiler-sealed inner join descriptor. All fragment variables, including
/// unprojected keys, survive until exact matching and final projection.
#[derive(Clone, Debug)]
pub struct BoundedJoin {
    origins: Option<[(SourceId, String); 2]>,
    variables: Vec<String>,
    domains: [Vec<usize>; 2],
    shared: Vec<usize>,
    projection: Vec<usize>,
    key: usize,
    reduce: bool,
}

pub(super) fn is_join(query: &Query) -> bool {
    matches!(query, Query::Select { pattern: GraphPattern::Project { inner, .. }, .. }
        if matches!(inner.as_ref(), GraphPattern::Bgp { patterns } if patterns.len() == 2))
}

fn triple_vars(triple: &TriplePattern) -> Result<BTreeSet<String>> {
    let (TermPattern::Variable(subject), TermPattern::Variable(object)) =
        (&triple.subject, &triple.object)
    else {
        return unsupported();
    };
    // Constants and repeated variables would leave term comparisons inside
    // source SQL, whose collation is not RDF term identity.
    if subject == object || !matches!(triple.predicate, NamedNodePattern::NamedNode(_)) {
        return unsupported();
    }
    Ok([subject.as_str().to_owned(), object.as_str().to_owned()].into())
}

pub(super) fn compile(
    query: Query,
    bindings: [&CompilerBinding; 2],
    control: &dyn QueryControl,
) -> Result<FederatedPlan> {
    compile_with_origins(query, bindings, control, false)
}

pub(super) fn compile_lineage(
    query: Query,
    bindings: [&CompilerBinding; 2],
    control: &dyn QueryControl,
) -> Result<FederatedPlan> {
    compile_with_origins(query, bindings, control, true)
}

fn compile_with_origins(
    query: Query,
    bindings: [&CompilerBinding; 2],
    control: &dyn QueryControl,
    lineage: bool,
) -> Result<FederatedPlan> {
    let Query::Select {
        dataset: None,
        pattern,
        base_iri,
    } = query
    else {
        return unsupported();
    };
    let GraphPattern::Project {
        inner,
        variables: output,
    } = pattern
    else {
        return unsupported();
    };
    let GraphPattern::Bgp { patterns } = *inner else {
        return unsupported();
    };
    if patterns.len() != 2 {
        return unsupported();
    }
    let mut domains = [triple_vars(&patterns[0])?, triple_vars(&patterns[1])?];
    let shared: BTreeSet<_> = domains[0].intersection(&domains[1]).cloned().collect();
    if shared.is_empty() {
        return unsupported();
    }
    let variables: Vec<_> = domains[0]
        .union(&domains[1])
        .cloned()
        .chain(output.iter().map(|v| v.as_str().to_owned()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut fragments = Vec::with_capacity(2);
    let mut origins = Vec::with_capacity(2);
    let mut estimates = Vec::with_capacity(2);
    for triple in patterns {
        control.checkpoint()?;
        let owners: Vec<_> = bindings
            .into_iter()
            .filter(|b| binding_could_match(b, &triple))
            .collect();
        if owners.len() != 1 {
            return unsupported();
        }
        let binding = owners[0];
        let arm = SourceAffineUnionArm {
            query: Query::Select {
                dataset: None,
                base_iri: base_iri.clone(),
                pattern: GraphPattern::Project {
                    inner: Box::new(GraphPattern::Bgp {
                        patterns: vec![triple.clone()],
                    }),
                    variables: variables.iter().map(Variable::new_unchecked).collect(),
                },
            },
            triple,
        };
        // Federation authorization is applied later; do not introduce a raw
        // cache path for secured joins.
        let mut plan =
            binding.compile_parsed_uncached_shared_with_work_control(arm.query(), control)?;
        if plan.branches.len() != 1 || !plan.source_sized_states().is_empty() {
            return unsupported();
        }
        let branch = &plan.branches[0];
        if !branch.opts.is_empty()
            || !branch.subplan_joins.is_empty()
            || branch.path.is_some()
            || branch.agg.is_some()
        {
            return unsupported();
        }
        let origin = restore_base_scan(Arc::make_mut(&mut plan), binding, &arm.triple)?;
        if lineage {
            if origin.is_empty()
                || origin.len() > 1024
                || binding
                    .triples_maps()
                    .iter()
                    .filter(|m| m.id == origin)
                    .count()
                    != 1
            {
                return unsupported();
            }
            control.consume(sf_core::query_control::QueryCharge::RetainedBytes, 2048)?;
            origins.push((binding.source_id(), origin.to_owned()));
        }
        let branch = &plan.branches[0];
        estimates.push(branch.core.first().and_then(|scan| {
            match scan.source.logical() {
                Some(LogicalSource::Table(name)) => binding
                    .schema()
                    .iter()
                    .find(|table| table.name == *name)
                    .and_then(|table| table.row_estimate),
                _ => None,
            }
        }));
        fragments.push(SourceFragment::new(binding.source_id(), plan)?);
    }
    if fragments[0].source_id == fragments[1].source_id {
        return unsupported();
    }
    let mut fragments: [SourceFragment; 2] = fragments.try_into().unwrap();
    // Catalog row estimates are only upper proxies when distinct counts are
    // absent. Missing observations use deterministic input order, not invented
    // statistics. No estimate grants retention or semantic authority.
    let mut reduce = true;
    if let [Some(left), Some(right)] = estimates.as_slice() {
        let choice = plan_semijoin(
            SideStats::new(*left, *left),
            SideStats::new(*right, *right),
            &CostConfig::default(),
        );
        let build = match choice {
            SemiJoinPlan::SemiJoin { build, .. } => build,
            SemiJoinPlan::SkipMerge { .. } => {
                reduce = false;
                if left <= right {
                    Side::Left
                } else {
                    Side::Right
                }
            }
        };
        if build == Side::Right {
            fragments.swap(0, 1);
            domains.swap(0, 1);
            if lineage {
                origins.swap(0, 1);
            }
        }
    }
    let key = variables
        .iter()
        .position(|name| {
            shared.contains(name)
                && fragments.iter().all(|f| {
                    f.plan.branches[0]
                        .bindings
                        .get(name)
                        .is_some_and(key::supported)
                })
        })
        .ok_or_else(|| {
            Error::Unsupported(
                "bounded join requires a reversible template or explicit literal key".into(),
            )
        })?;
    let join = BoundedJoin {
        origins: lineage.then(|| origins.try_into().unwrap()),
        domains: domains.map(|domain| {
            variables
                .iter()
                .enumerate()
                .filter_map(|(i, v)| domain.contains(v).then_some(i))
                .collect()
        }),
        shared: variables
            .iter()
            .enumerate()
            .filter_map(|(i, v)| shared.contains(v).then_some(i))
            .collect(),
        projection: output
            .iter()
            .map(|v| {
                variables
                    .iter()
                    .position(|name| name == v.as_str())
                    .unwrap()
            })
            .collect(),
        variables,
        key,
        reduce,
    };
    Ok(FederatedPlan {
        variables: output.iter().map(|v| v.as_str().to_owned()).collect(),
        fragments,
        join: Some(join),
    })
}

/// D1's raw-column SQL DISTINCT inherits database collation and cannot establish
/// RDF graph-set identity. For this sealed operator only, restore the known
/// authored base table; its fixed-cap merge performs exact triple-set handling.
/// Never unwrap authored SQL, joins, unions, computed projections or filters.
fn restore_base_scan<'a>(
    plan: &mut Plan,
    binding: &'a CompilerBinding,
    triple: &TriplePattern,
) -> Result<&'a str> {
    let maps: Vec<_> = binding
        .triples_maps()
        .iter()
        .filter(|map| mapping_could_emit(map, &triple.predicate))
        .collect();
    let [map] = maps.as_slice() else {
        return unsupported();
    };
    let NamedNodePattern::NamedNode(wanted) = &triple.predicate else {
        return unsupported();
    };
    let mut emitters = 0;
    for pom in &map.predicate_object_maps {
        for predicate in &pom.predicates {
            let TermMap::Constant(Term::NamedNode(node)) = predicate else {
                return unsupported();
            };
            if node == wanted {
                if !matches!(pom.objects.as_slice(), [sf_core::ir::ObjectMap::Term(_)]) {
                    return unsupported();
                }
                emitters += 1;
            }
        }
    }
    if emitters != 1
        || !map.subject.graphs.is_empty()
        || map
            .predicate_object_maps
            .iter()
            .any(|pom| !pom.graphs.is_empty())
    {
        return unsupported();
    }
    let LogicalSource::Table(table) = &map.source else {
        return unsupported();
    };
    let branch = &mut plan.branches[0];
    let [scan] = branch.core.as_mut_slice() else {
        return unsupported();
    };
    match scan.source.logical() {
        Some(LogicalSource::Table(name)) if name == table => {}
        Some(LogicalSource::Query(sql)) => {
            let source = sf_sql::policy_projection::single_table_view_source(sql, plan.dialect)
                .map_err(|_| Error::Unsupported("bounded join source projection".into()))?;
            // This helper checks every expression is the same-named qualified
            // raw column, not merely that FROM happens to name one table.
            sf_sql::policy_projection::expose_single_table_view_columns(
                sql,
                plan.dialect,
                &["__sf_join_shape_check"],
            )
            .map_err(|_| Error::Unsupported("bounded join source projection".into()))?;
            if source != *table {
                return unsupported();
            }
            scan.source = map.source.clone().into();
        }
        _ => return unsupported(),
    }
    plan.distinct = false;
    branch.distinct = false;
    Ok(&map.id)
}

impl BoundedJoin {
    /// Each mandatory arm has exactly one validated direct mapping emitter.
    /// This sealed proof travels with the join, never through a witness driver.
    pub fn mapping_origins(&self) -> Option<&[(SourceId, String); 2]> {
        self.origins.as_ref()
    }
    /// Each admitted arm is a mandatory triple, so its own domain is fully
    /// bound. Variables belonging only to the other arm/final projection may
    /// remain unbound; requiring the whole combined header would drop valid rows.
    pub fn accepts_row(&self, side: usize, row: &[Option<Term>]) -> Result<bool> {
        let domain = self
            .domains
            .get(side)
            .ok_or_else(|| Error::Mapping("bounded join side".into()))?;
        if row.len() != self.variables.len() {
            return Err(Error::Mapping("bounded join row arity mismatch".into()));
        }
        Ok(domain.iter().all(|&i| row[i].is_some()))
    }
    pub const fn max_build_rows(&self) -> usize {
        MAX_BUILD_ROWS
    }

    /// Add a conservative bounded reducer to an already authorized plan.
    /// Existing source predicates, dedup and bindings are never replaced.
    pub fn reduced_probe(&self, probe: &Plan, rows: &[Vec<Option<Term>>]) -> Result<Plan> {
        if rows.len() > MAX_BUILD_ROWS
            || probe.branches.len() != 1
            || select_variables(probe) != Some(self.variables.as_slice())
        {
            return Err(Error::Mapping(
                "bounded join probe identity mismatch".into(),
            ));
        }
        let mut result = probe.clone();
        if !self.reduce {
            return Ok(result);
        }
        let branch = &mut result.branches[0];
        let def = branch
            .bindings
            .get(&self.variables[self.key])
            .ok_or_else(|| Error::Mapping("bounded join key missing".into()))?;
        let mut values = BTreeSet::new();
        for row in rows {
            if row.len() != self.variables.len() {
                return Err(Error::Mapping("bounded join row arity mismatch".into()));
            }
            if let Some(term) = &row[self.key] {
                if let Some(value) = key::inverse(def, term)? {
                    values.insert(value);
                }
            }
        }
        let column =
            key::column(def).ok_or_else(|| Error::Mapping("bounded join key changed".into()))?;
        branch.where_conds.push(crate::iq::SqlCond::Or(
            values
                .into_iter()
                .map(|param| crate::iq::SqlCond::StrMatch {
                    col: column.clone(),
                    op: crate::iq::StrMatchOp::CoarseLexicalEqual,
                    param,
                })
                .collect(),
        ));
        Ok(result)
    }

    /// Match every shared RDF term exactly (including datatype, language and
    /// RDF 1.2 direction), then project. Input blank nodes must be source scoped.
    pub fn merge(
        &self,
        left: &[Option<Term>],
        right: &[Option<Term>],
    ) -> Result<Option<Vec<Option<Term>>>> {
        if !self.accepts_row(0, left)? || !self.accepts_row(1, right)? {
            return Ok(None);
        }
        if self.shared.iter().any(|&i| match (&left[i], &right[i]) {
            (Some(a), Some(b)) => !same_term(a, b),
            // A mandatory triple variable cannot be unbound in this profile.
            _ => true,
        }) {
            return Ok(None);
        }
        Ok(Some(
            self.projection
                .iter()
                .map(|&i| left[i].as_ref().or(right[i].as_ref()).cloned())
                .collect(),
        ))
    }
}

fn same_term(left: &Term, right: &Term) -> bool {
    match (left, right) {
        (Term::Literal(a), Term::Literal(b)) => {
            a.value() == b.value()
                && a.datatype() == b.datatype()
                && a.direction() == b.direction()
                && match (a.language(), b.language()) {
                    (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                    (None, None) => true,
                    _ => false,
                }
        }
        (Term::Triple(a), Term::Triple(b)) => {
            a.subject == b.subject && a.predicate == b.predicate && same_term(&a.object, &b.object)
        }
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BaseDirection, Literal};

    #[test]
    fn rdf_direction_datatype_and_lexical_identity_are_not_value_coercions() {
        let plain = Term::from(Literal::new_language_tagged_literal("same", "en").unwrap());
        let ltr = Term::from(
            Literal::new_directional_language_tagged_literal("same", "en", BaseDirection::Ltr)
                .unwrap(),
        );
        let rtl = Term::from(
            Literal::new_directional_language_tagged_literal("same", "en", BaseDirection::Rtl)
                .unwrap(),
        );
        assert!(!same_term(&plain, &ltr));
        assert!(!same_term(&ltr, &rtl));
        assert!(same_term(&ltr, &ltr));
        assert!(!same_term(
            &Literal::new_typed_literal("01", oxrdf::vocab::xsd::INTEGER).into(),
            &Literal::new_typed_literal("1", oxrdf::vocab::xsd::INTEGER).into()
        ));
        assert!(!same_term(
            &Literal::new_simple_literal("1").into(),
            &Literal::new_typed_literal("1", oxrdf::vocab::xsd::INTEGER).into()
        ));
    }
}

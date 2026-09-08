//! Opt-in bounded positive-query lineage. Mapping alternatives remain separate;
//! RDF matching and joins happen in the bounded executor, not SQL equality.
use std::collections::BTreeSet;

use sf_core::ir::{ObjectMap, TermMap};
use sf_core::query_control::{QueryCharge, QueryControl};
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;

use crate::{CompilerBinding, Error, Plan, PlanForm, Result};

pub const MAX_MAPS: usize = 64;
pub const MAX_BRANCHES: usize = 256;
pub const MAX_WITNESSES: usize = 1024;

#[derive(Clone, Debug)]
pub(crate) enum Node {
    Atom(usize),
    Join(Box<Node>, Box<Node>),
    Union(Box<Node>, Box<Node>),
}
#[derive(Clone, Debug)]
pub(crate) enum Modifier {
    Project(Vec<String>),
    Distinct,
    Slice(usize, Option<usize>),
}

/// Private origin recipes accompany a caller-owned raw plan. Serving binds both
/// to the same immutable source/security generation before execution. This raw
/// API does not authorize mutation of a plan from another mapping or source.
#[derive(Clone, Debug)]
pub struct LineageSpec {
    pub(crate) maps: Vec<String>,
    pub(crate) atoms: Vec<TriplePattern>,
    pub(crate) branches: Vec<(usize, usize, u64)>, // scan alias, atom, map bit
    pub(crate) root: Node,
    pub(crate) modifiers: Vec<Modifier>, // outermost first
    pub(crate) output_nodes: u64,
    pub(crate) query_bytes: u64,
}
impl LineageSpec {
    pub fn mapping_ids(&self) -> &[String] {
        &self.maps
    }
}

pub(crate) fn unsupported() -> Error {
    Error::Unsupported("bounded multi-origin lineage shape is not admitted".into())
}

impl CompilerBinding {
    /// Compile positive BGP/JOIN/UNION with a root modifier spine. Every atom
    /// uses fresh private reconstruction slots; constants and repeated variables
    /// are checked using native RDF terms so SQL coercion cannot change matches.
    pub fn compile_lineage(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<(Plan, LineageSpec)> {
        control.checkpoint()?;
        let maps = self.triples_maps();
        if maps.is_empty() || maps.len() > MAX_MAPS {
            return Err(unsupported());
        }
        let mut ids = BTreeSet::new();
        let mut alternatives = 0usize;
        for map in maps {
            if map.id.is_empty() || map.id.len() > 1024 || !ids.insert(&map.id) {
                return Err(unsupported());
            }
            if map
                .subject
                .graphs
                .iter()
                .chain(map.predicate_object_maps.iter().flat_map(|pom| &pom.graphs))
                .any(|graph| !matches!(graph, TermMap::Constant(sf_core::Term::NamedNode(_))))
            {
                return Err(unsupported());
            }
            alternatives = alternatives
                .checked_add(map.subject.classes.len())
                .ok_or_else(unsupported)?;
            for pom in &map.predicate_object_maps {
                // Dynamic predicates need their own exact entailment admission.
                if pom
                    .predicates
                    .iter()
                    .any(|p| !matches!(p, TermMap::Constant(sf_core::Term::NamedNode(_))))
                    || pom.objects.iter().any(|o| matches!(o, ObjectMap::Ref(_)))
                {
                    return Err(unsupported());
                }
                alternatives = alternatives
                    .checked_add(
                        pom.predicates
                            .len()
                            .checked_mul(pom.objects.len())
                            .ok_or_else(unsupported)?,
                    )
                    .ok_or_else(unsupported)?;
            }
        }
        if alternatives > MAX_BRANCHES {
            return Err(unsupported());
        }
        control.consume(QueryCharge::CompilerWork, alternatives as u64)?;
        let query = crate::parse_query(sparql)?;
        let mut output_nodes = 0u64;
        let (pattern, form) = match &query {
            Query::Select {
                pattern,
                dataset: None,
                ..
            } => (pattern, None),
            Query::Construct {
                pattern,
                dataset: None,
                template,
                ..
            } if template.len() <= 256 => {
                let mut pending = Vec::new();
                for triple in template {
                    pending.push(&triple.subject);
                    pending.push(&triple.object);
                }
                while let Some(term) = pending.pop() {
                    output_nodes += 3;
                    if output_nodes > 1024 {
                        return Err(unsupported());
                    }
                    if let TermPattern::Triple(triple) = term {
                        pending.push(&triple.subject);
                        pending.push(&triple.object);
                    }
                }
                control.consume(QueryCharge::CompilerWork, template.len() as u64)?;
                (
                    pattern,
                    Some(PlanForm::Construct {
                        template: template.clone(),
                    }),
                )
            }
            _ => return Err(unsupported()),
        };
        let mut spec = LineageSpec {
            maps: maps.iter().map(|m| m.id.clone()).collect(),
            atoms: vec![],
            branches: vec![],
            root: Node::Atom(0),
            modifiers: vec![],
            output_nodes,
            query_bytes: sparql.len() as u64,
        };
        let mut inner = pattern;
        let mut visits = 0usize;
        loop {
            visits += 1;
            if visits > 128 {
                return Err(unsupported());
            }
            control.consume(QueryCharge::CompilerWork, 1)?;
            match inner {
                GraphPattern::Project {
                    inner: child,
                    variables,
                } => {
                    if variables.len() > 256 {
                        return Err(unsupported());
                    }
                    spec.modifiers.push(Modifier::Project(
                        variables.iter().map(|v| v.as_str().to_owned()).collect(),
                    ));
                    inner = child;
                }
                GraphPattern::Distinct { inner: child }
                | GraphPattern::Reduced { inner: child } => {
                    spec.modifiers.push(Modifier::Distinct);
                    inner = child;
                }
                GraphPattern::Slice {
                    inner: child,
                    start,
                    length,
                } => {
                    spec.modifiers.push(Modifier::Slice(*start, *length));
                    inner = child;
                }
                _ => break,
            }
        }
        spec.root = recipe(inner, &mut spec.atoms, &mut visits, control)?;
        let form = form.unwrap_or_else(|| PlanForm::Select {
            vars: spec
                .modifiers
                .iter()
                .find_map(|m| {
                    if let Modifier::Project(vars) = m {
                        Some(vars.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_default(),
        });
        // Atom expansion is capped before pattern_branches can allocate its
        // predicate/object product. Real TBox lookup retains inverse/subproperty
        // semantics; variable-predicate/type breadth is conservatively excluded.
        let mut uf = crate::unfold::Unfolder::new_with_column_type_use(
            maps,
            self.tbox(),
            self.dialect(),
            self.schema(),
            self.column_type_use(),
        );
        let mut branches = Vec::new();
        for (atom, pattern) in spec.atoms.iter().enumerate() {
            let NamedNodePattern::NamedNode(predicate) = &pattern.predicate else {
                return Err(unsupported());
            };
            if predicate.as_str() == crate::unfold::RDF_TYPE {
                return Err(unsupported());
            }
            let direct = self.tbox().saturate_predicate(predicate.as_str());
            let inverse = self.tbox().inverse_predicates(predicate.as_str());
            if direct.iter().any(|p| inverse.contains(p)) {
                return Err(unsupported());
            }
            if branches
                .len()
                .checked_add(alternatives)
                .is_none_or(|n| n > MAX_BRANCHES)
            {
                return Err(unsupported());
            }
            control.consume(QueryCharge::CompilerWork, alternatives as u64)?;
            let broad = TriplePattern {
                subject: Variable::new_unchecked("s").into(),
                predicate: pattern.predicate.clone(),
                object: Variable::new_unchecked("o").into(),
            };
            for (map, mapping) in maps.iter().enumerate() {
                uf.maps = std::slice::from_ref(mapping);
                for branch in uf.pattern_branches(&broad)? {
                    let [scan] = branch.core.as_slice() else {
                        return Err(unsupported());
                    };
                    spec.branches.push((scan.alias, atom, 1u64 << map));
                    branches.push(branch);
                }
            }
        }
        // No DISTINCT, narrowing, pooling, SQL join or slice erases a witness.
        let plan = Plan {
            branches,
            form,
            distinct: false,
            limit: None,
            offset: 0,
            order: vec![],
            rust_group: None,
            dialect: self.dialect(),
            dedup_scopes: vec![],
            construct_drops_some_branch_var: false,
        };
        control.checkpoint()?;
        Ok((plan, spec))
    }
}

fn recipe(
    pattern: &GraphPattern,
    atoms: &mut Vec<TriplePattern>,
    visits: &mut usize,
    control: &dyn QueryControl,
) -> Result<Node> {
    *visits += 1;
    if *visits > 128 {
        return Err(unsupported());
    }
    control.consume(QueryCharge::CompilerWork, 1)?;
    match pattern {
        GraphPattern::Bgp { patterns } if !patterns.is_empty() => {
            let mut nodes = Vec::new();
            for pattern in patterns {
                *visits += 1;
                if *visits > 128
                    || matches!(pattern.subject, TermPattern::Triple(_))
                    || matches!(pattern.object, TermPattern::Triple(_))
                {
                    return Err(unsupported());
                }
                control.consume(QueryCharge::CompilerWork, 1)?;
                let index = atoms.len();
                atoms.push(pattern.clone());
                nodes.push(Node::Atom(index));
            }
            Ok(nodes
                .into_iter()
                .reduce(|a, b| Node::Join(Box::new(a), Box::new(b)))
                .expect("nonempty BGP"))
        }
        GraphPattern::Join { left, right } => Ok(Node::Join(
            Box::new(recipe(left, atoms, visits, control)?),
            Box::new(recipe(right, atoms, visits, control)?),
        )),
        GraphPattern::Union { left, right } => Ok(Node::Union(
            Box::new(recipe(left, atoms, visits, control)?),
            Box::new(recipe(right, atoms, visits, control)?),
        )),
        _ => Err(unsupported()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::ir::{LogicalSource, PredicateObjectMap, SubjectMap, TermSpec, TriplesMap};
    use sf_core::query_control::{QueryBudget, QueryLimits, UncontrolledQueryControl};
    use sf_core::{NamedNode, SourceId, SourceMapping};

    fn maps() -> Vec<TriplesMap> {
        vec![TriplesMap {
            id: "urn:map".into(),
            source: LogicalSource::Table("items".into()),
            subject: SubjectMap {
                term: TermMap::Constant(NamedNode::new_unchecked("urn:subject").into()),
                classes: vec![],
                graphs: vec![],
            },
            predicate_object_maps: vec![PredicateObjectMap {
                predicates: vec![TermMap::Constant(NamedNode::new_unchecked("urn:p").into())],
                objects: vec![ObjectMap::Term(TermMap::Column(
                    "value".into(),
                    TermSpec::plain_literal(),
                ))],
                graphs: vec![],
            }],
        }]
    }
    fn binding(maps: Vec<TriplesMap>, tbox: crate::Tbox) -> CompilerBinding {
        CompilerBinding::new(
            SourceMapping::new(SourceId::new(0).unwrap(), maps),
            sf_sql::Dialect::Sqlite,
            tbox,
            crate::CompilerSchema::from_unverified_observation(vec![]),
            8,
        )
    }
    #[test]
    fn dynamic_graph_and_overlapping_inverse_membership_reject_before_io() {
        let query = "SELECT ?s ?o WHERE { ?s <urn:p> ?o }";
        let mut dynamic = maps();
        dynamic[0]
            .subject
            .graphs
            .push(TermMap::Column("graph".into(), TermSpec::iri()));
        assert!(matches!(
            binding(dynamic, crate::Tbox::default())
                .compile_lineage(query, &UncontrolledQueryControl),
            Err(Error::Unsupported(_))
        ));
        let mut dynamic = maps();
        dynamic[0].predicate_object_maps[0]
            .graphs
            .push(TermMap::Column("graph".into(), TermSpec::iri()));
        assert!(matches!(
            binding(dynamic, crate::Tbox::default())
                .compile_lineage(query, &UncontrolledQueryControl),
            Err(Error::Unsupported(_))
        ));
        let mut symmetric = crate::Tbox::default();
        symmetric.add_symmetric("urn:p");
        assert!(matches!(
            binding(maps(), symmetric).compile_lineage(query, &UncontrolledQueryControl),
            Err(Error::Unsupported(_))
        ));
    }
    #[test]
    fn ambiguous_ids_and_expansion_overflow_never_create_a_plan() {
        let query = "SELECT ?s WHERE { ?s <urn:p> ?o }";
        let mut duplicate = maps();
        duplicate.push(duplicate[0].clone());
        assert!(binding(duplicate, crate::Tbox::default())
            .compile_lineage(query, &UncontrolledQueryControl)
            .is_err());
        let mut large = maps();
        large[0].predicate_object_maps[0].predicates =
            vec![TermMap::Constant(NamedNode::new_unchecked("urn:p").into()); 257];
        assert!(binding(large, crate::Tbox::default())
            .compile_lineage(query, &UncontrolledQueryControl)
            .is_err());
        let budget = QueryBudget::new(QueryLimits::new(0, 0, 0, 0));
        assert!(matches!(
            binding(maps(), crate::Tbox::default()).compile_lineage(query, &budget),
            Err(Error::QueryControl(_))
        ));
    }
}

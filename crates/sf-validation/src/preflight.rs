//! Deterministic logical-work preflight for the four sealed SHACL shapes.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use oxrdf::{Graph, NamedOrBlankNodeRef, TermRef};

use crate::{GateError, ValidationLimits};

const RR_CLASS: &str = "http://www.w3.org/ns/r2rml#class";
const RML_CLASS: &str = "http://semweb.mmlab.be/ns/rml#class";
const RR_PREDICATE: &str = "http://www.w3.org/ns/r2rml#predicate";
const RML_PREDICATE: &str = "http://semweb.mmlab.be/ns/rml#predicate";
const RR_OBJECT_MAP: &str = "http://www.w3.org/ns/r2rml#objectMap";
const RML_OBJECT_MAP: &str = "http://semweb.mmlab.be/ns/rml#objectMap";
const RR_DATATYPE: &str = "http://www.w3.org/ns/r2rml#datatype";
const RML_DATATYPE: &str = "http://semweb.mmlab.be/ns/rml#datatype";
const SH_PROPERTY: &str = "http://www.w3.org/ns/shacl#property";
const SH_PATH: &str = "http://www.w3.org/ns/shacl#path";
const SH_DATATYPE: &str = "http://www.w3.org/ns/shacl#datatype";
const MF_STEREOTYPE: &str = "http://example.org/mapping-fabric#stereotype";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

type TermCounts<'a> = HashMap<TermRef<'a>, usize>;
type NodeTermCounts<'a> = HashMap<NamedOrBlankNodeRef<'a>, TermCounts<'a>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ValidationWorkload {
    pub(crate) work_units: usize,
    pub(crate) result_cardinality: usize,
    pub(crate) datatype_candidates: usize,
    pub(crate) has_blank_datatype_focus: bool,
    #[cfg(test)]
    pub(crate) datatype_focus_nodes: usize,
    #[cfg(test)]
    pub(crate) datatype_mapping_bindings: usize,
    #[cfg(test)]
    pub(crate) datatype_constraint_bindings: usize,
    #[cfg(test)]
    class_targets: usize,
    #[cfg(test)]
    predicate_targets: usize,
    #[cfg(test)]
    entity_targets: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DatatypeWork {
    mapping_bindings: usize,
    constraint_bindings: usize,
    candidates: usize,
}

#[derive(Default)]
struct ShapeFacts<'a> {
    class_targets: HashSet<TermRef<'a>>,
    predicate_targets: HashSet<TermRef<'a>>,
    entity_targets: HashSet<NamedOrBlankNodeRef<'a>>,
    class_edges_by_object: TermCounts<'a>,
    predicates_by_pom: NodeTermCounts<'a>,
    object_maps_by_pom: NodeTermCounts<'a>,
    datatypes_by_node: HashMap<NamedOrBlankNodeRef<'a>, usize>,
    property_shape_owners: HashMap<NamedOrBlankNodeRef<'a>, usize>,
    paths_by_property_shape: NodeTermCounts<'a>,
    datatypes_by_property_shape: HashMap<NamedOrBlankNodeRef<'a>, usize>,
    rdf_types_by_subject: NodeTermCounts<'a>,
    subclasses_by_subject: NodeTermCounts<'a>,
}

pub(crate) fn admit(
    graph: &Graph,
    limits: ValidationLimits,
) -> Result<ValidationWorkload, GateError> {
    let workload = measure(graph)?;
    if workload.work_units > limits.max_work_units {
        return Err(GateError::ValidationWorkLimit);
    }
    if workload.result_cardinality > limits.max_result_cardinality {
        return Err(GateError::ValidationResultLimit);
    }
    Ok(workload)
}

fn measure(graph: &Graph) -> Result<ValidationWorkload, GateError> {
    let facts = collect_facts(graph)?;
    let class_targets = facts.class_targets.len();
    let predicate_targets = facts.predicate_targets.len();
    let entity_targets = facts.entity_targets.len();
    let datatype_focus_nodes = facts.predicates_by_pom.len();
    let has_blank_datatype_focus = facts
        .predicates_by_pom
        .keys()
        .any(|focus| matches!(focus, NamedOrBlankNodeRef::BlankNode(_)));
    let datatype_work = datatype_work(&facts)?;
    let grounding_candidates = facts
        .entity_targets
        .iter()
        .try_fold(0usize, |sum, target| {
            checked_add(
                sum,
                facts
                    .class_edges_by_object
                    .get(&TermRef::from(*target))
                    .copied()
                    .unwrap_or_default(),
                GateError::ValidationWorkLimit,
            )
        })?;

    let class_work = class_constraint_work(&facts.class_targets, &facts)?;
    let predicate_branch_work = checked_mul(
        class_constraint_work(&facts.predicate_targets, &facts)?,
        4,
        GateError::ValidationWorkLimit,
    )?;
    let work_units = [
        graph.len(),
        class_work,
        predicate_branch_work,
        datatype_focus_nodes,
        datatype_work.mapping_bindings,
        datatype_work.constraint_bindings,
        datatype_work.candidates,
        entity_targets,
        grounding_candidates,
    ]
    .into_iter()
    .try_fold(0usize, |sum, value| {
        checked_add(sum, value, GateError::ValidationWorkLimit)
    })?;

    // sh:or may emit one parent and four branch details for each predicate
    // focus node. The other three shapes emit at most one result per target or
    // SPARQL solution, so this is a conservative report-allocation bound.
    let predicate_result_bound =
        checked_mul(predicate_targets, 5, GateError::ValidationResultLimit)?;
    let result_cardinality = [
        class_targets,
        predicate_result_bound,
        datatype_work.candidates,
        entity_targets,
    ]
    .into_iter()
    .try_fold(0usize, |sum, value| {
        checked_add(sum, value, GateError::ValidationResultLimit)
    })?;

    Ok(ValidationWorkload {
        work_units,
        result_cardinality,
        datatype_candidates: datatype_work.candidates,
        has_blank_datatype_focus,
        #[cfg(test)]
        datatype_focus_nodes,
        #[cfg(test)]
        datatype_mapping_bindings: datatype_work.mapping_bindings,
        #[cfg(test)]
        datatype_constraint_bindings: datatype_work.constraint_bindings,
        #[cfg(test)]
        class_targets,
        #[cfg(test)]
        predicate_targets,
        #[cfg(test)]
        entity_targets,
    })
}

fn collect_facts(graph: &Graph) -> Result<ShapeFacts<'_>, GateError> {
    let mut facts = ShapeFacts::default();
    for triple in graph.iter() {
        match triple.predicate.as_str() {
            RR_CLASS | RML_CLASS => {
                facts.class_targets.insert(triple.object);
                increment(
                    &mut facts.class_edges_by_object,
                    triple.object,
                    GateError::ValidationWorkLimit,
                )?;
            }
            RR_PREDICATE | RML_PREDICATE => {
                facts.predicate_targets.insert(triple.object);
                increment_nested(&mut facts.predicates_by_pom, triple.subject, triple.object)?;
            }
            RR_OBJECT_MAP | RML_OBJECT_MAP => {
                increment_nested(&mut facts.object_maps_by_pom, triple.subject, triple.object)?
            }
            RR_DATATYPE | RML_DATATYPE => increment(
                &mut facts.datatypes_by_node,
                triple.subject,
                GateError::ValidationWorkLimit,
            )?,
            SH_PROPERTY => {
                if let Some(property_shape) = as_node(triple.object) {
                    increment(
                        &mut facts.property_shape_owners,
                        property_shape,
                        GateError::ValidationWorkLimit,
                    )?;
                }
            }
            SH_PATH => increment_nested(
                &mut facts.paths_by_property_shape,
                triple.subject,
                triple.object,
            )?,
            SH_DATATYPE => increment(
                &mut facts.datatypes_by_property_shape,
                triple.subject,
                GateError::ValidationWorkLimit,
            )?,
            MF_STEREOTYPE => {
                facts.entity_targets.insert(triple.subject);
            }
            RDF_TYPE => increment_nested(
                &mut facts.rdf_types_by_subject,
                triple.subject,
                triple.object,
            )?,
            RDFS_SUBCLASS_OF => increment_nested(
                &mut facts.subclasses_by_subject,
                triple.subject,
                triple.object,
            )?,
            _ => {}
        }
    }
    Ok(facts)
}

fn class_constraint_work<'a>(
    targets: &HashSet<TermRef<'a>>,
    facts: &ShapeFacts<'a>,
) -> Result<usize, GateError> {
    targets.iter().try_fold(targets.len(), |sum, target| {
        let Some(target) = as_node(*target) else {
            return Ok(sum);
        };
        let Some(types) = facts.rdf_types_by_subject.get(&target) else {
            return Ok(sum);
        };
        types.iter().try_fold(sum, |sum, (class, type_count)| {
            let subclass_count = if let Some(subclasses) =
                as_node(*class).and_then(|class| facts.subclasses_by_subject.get(&class))
            {
                subclasses.values().try_fold(0usize, |count, value| {
                    checked_add(count, *value, GateError::ValidationWorkLimit)
                })?
            } else {
                0
            };
            let subclass_work =
                checked_mul(*type_count, subclass_count, GateError::ValidationWorkLimit)?;
            checked_add(
                checked_add(sum, *type_count, GateError::ValidationWorkLimit)?,
                subclass_work,
                GateError::ValidationWorkLimit,
            )
        })
    })
}

fn datatype_work(facts: &ShapeFacts<'_>) -> Result<DatatypeWork, GateError> {
    let mut constraints_by_predicate = HashMap::<TermRef<'_>, usize>::new();
    let mut constraint_bindings = 0usize;
    for (property_shape, paths) in &facts.paths_by_property_shape {
        let owner_count = facts
            .property_shape_owners
            .get(property_shape)
            .copied()
            .unwrap_or_default();
        let datatype_count = facts
            .datatypes_by_property_shape
            .get(property_shape)
            .copied()
            .unwrap_or_default();
        let shape_multiplier =
            checked_mul(owner_count, datatype_count, GateError::ValidationWorkLimit)?;
        for (path, path_count) in paths {
            let candidates = checked_mul(
                shape_multiplier,
                *path_count,
                GateError::ValidationWorkLimit,
            )?;
            constraint_bindings = checked_add(
                constraint_bindings,
                candidates,
                GateError::ValidationWorkLimit,
            )?;
            add_to(
                &mut constraints_by_predicate,
                *path,
                candidates,
                GateError::ValidationWorkLimit,
            )?;
        }
    }

    let mut candidates = 0usize;
    let mut mapping_bindings = 0usize;
    for (pom, predicates) in &facts.predicates_by_pom {
        let Some(object_maps) = facts.object_maps_by_pom.get(pom) else {
            continue;
        };
        let object_solutions =
            object_maps
                .iter()
                .try_fold(0usize, |sum, (object_map, edge_count)| {
                    let datatype_count = as_node(*object_map)
                        .and_then(|node| facts.datatypes_by_node.get(&node).copied())
                        .unwrap_or_default()
                        .max(1);
                    let solutions =
                        checked_mul(*edge_count, datatype_count, GateError::ValidationWorkLimit)?;
                    checked_add(sum, solutions, GateError::ValidationWorkLimit)
                })?;
        let predicate_count = predicates.values().try_fold(0usize, |sum, count| {
            checked_add(sum, *count, GateError::ValidationWorkLimit)
        })?;
        mapping_bindings = checked_add(
            mapping_bindings,
            checked_mul(
                predicate_count,
                object_solutions,
                GateError::ValidationWorkLimit,
            )?,
            GateError::ValidationWorkLimit,
        )?;
        for (predicate, edge_count) in predicates {
            let constraint_count = constraints_by_predicate
                .get(predicate)
                .copied()
                .unwrap_or_default();
            let predicate_solutions = checked_mul(
                *edge_count,
                constraint_count,
                GateError::ValidationWorkLimit,
            )?;
            let joined = checked_mul(
                predicate_solutions,
                object_solutions,
                GateError::ValidationWorkLimit,
            )?;
            candidates = checked_add(candidates, joined, GateError::ValidationWorkLimit)?;
        }
    }
    Ok(DatatypeWork {
        mapping_bindings,
        constraint_bindings,
        candidates,
    })
}

fn as_node(term: TermRef<'_>) -> Option<NamedOrBlankNodeRef<'_>> {
    match term {
        TermRef::NamedNode(node) => Some(node.into()),
        TermRef::BlankNode(node) => Some(node.into()),
        TermRef::Literal(_) | TermRef::Triple(_) => None,
    }
}

fn increment<K: Copy + Eq + Hash>(
    counts: &mut HashMap<K, usize>,
    key: K,
    error: GateError,
) -> Result<(), GateError> {
    add_to(counts, key, 1, error)
}

fn increment_nested<'a>(
    counts: &mut NodeTermCounts<'a>,
    node: NamedOrBlankNodeRef<'a>,
    term: TermRef<'a>,
) -> Result<(), GateError> {
    increment(
        counts.entry(node).or_default(),
        term,
        GateError::ValidationWorkLimit,
    )
}

fn add_to<K: Copy + Eq + Hash>(
    counts: &mut HashMap<K, usize>,
    key: K,
    value: usize,
    error: GateError,
) -> Result<(), GateError> {
    let count = counts.entry(key).or_default();
    *count = checked_add(*count, value, error)?;
    Ok(())
}

fn checked_add(left: usize, right: usize, error: GateError) -> Result<usize, GateError> {
    left.checked_add(right).ok_or(error)
}

fn checked_mul(left: usize, right: usize, error: GateError) -> Result<usize, GateError> {
    left.checked_mul(right).ok_or(error)
}

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;

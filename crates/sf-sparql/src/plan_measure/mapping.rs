use sf_core::ir::{
    Join, LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, Segment, SubjectMap,
    Template, TermMap, TermSpec, TriplesMap,
};

use super::{PlanMeasureError, Walker, Work};

pub(super) fn visit_triples_map<'a>(
    walker: &mut Walker<'a>,
    triples_map: &'a TriplesMap,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let TriplesMap {
        id,
        source,
        subject,
        predicate_object_maps,
    } = triples_map;
    walker.payload(id.len())?;
    walker.push(depth, Work::LogicalSource(source))?;
    walker.push(depth, Work::SubjectMap(subject))?;
    walker.collection(predicate_object_maps.len())?;
    for predicate_object_map in predicate_object_maps {
        walker.push(depth, Work::PredicateObjectMap(predicate_object_map))?;
    }
    Ok(())
}

pub(super) fn visit_subject_map<'a>(
    walker: &mut Walker<'a>,
    subject: &'a SubjectMap,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let SubjectMap {
        term,
        classes,
        graphs,
    } = subject;
    walker.push(depth, Work::TermMap(term))?;
    walker.collection(classes.len())?;
    for class in classes {
        walker.push(depth, Work::NamedNode(class))?;
    }
    push_term_maps(walker, graphs, depth)
}

pub(super) fn visit_predicate_object_map<'a>(
    walker: &mut Walker<'a>,
    predicate_object_map: &'a PredicateObjectMap,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let PredicateObjectMap {
        predicates,
        objects,
        graphs,
    } = predicate_object_map;
    push_term_maps(walker, predicates, depth)?;
    walker.collection(objects.len())?;
    for object in objects {
        walker.push(depth, Work::ObjectMap(object))?;
    }
    push_term_maps(walker, graphs, depth)
}

pub(super) fn visit_object_map<'a>(
    walker: &mut Walker<'a>,
    object: &'a ObjectMap,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match object {
        ObjectMap::Term(term_map) => walker.push(depth, Work::TermMap(term_map)),
        ObjectMap::Ref(reference) => walker.push(depth, Work::RefObjectMap(reference)),
    }
}

pub(super) fn visit_ref_object_map<'a>(
    walker: &mut Walker<'a>,
    reference: &'a RefObjectMap,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let RefObjectMap {
        parent_triples_map,
        joins,
    } = reference;
    walker.payload(parent_triples_map.len())?;
    walker.collection(joins.len())?;
    for join in joins {
        walker.push(depth, Work::Join(join))?;
    }
    Ok(())
}

pub(super) fn visit_join(walker: &mut Walker<'_>, join: &Join) -> Result<(), PlanMeasureError> {
    let Join { child, parent } = join;
    walker.payload(child.len())?;
    walker.payload(parent.len())
}

pub(super) fn visit_logical_source(
    walker: &mut Walker<'_>,
    source: &LogicalSource,
) -> Result<(), PlanMeasureError> {
    let (LogicalSource::Table(value) | LogicalSource::Query(value)) = source;
    walker.payload(value.len())
}

pub(super) fn visit_term_map<'a>(
    walker: &mut Walker<'a>,
    term_map: &'a TermMap,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match term_map {
        TermMap::Constant(term) => walker.push(depth, Work::Term(term))?,
        TermMap::Column(column, spec) => {
            walker.payload(column.len())?;
            walker.push(depth, Work::TermSpec(spec))?;
        }
        TermMap::Template(template, spec) => {
            walker.push(depth, Work::Template(template))?;
            walker.push(depth, Work::TermSpec(spec))?;
        }
    }
    Ok(())
}

pub(super) fn visit_template<'a>(
    walker: &mut Walker<'a>,
    template: &'a Template,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    push_segments(walker, template.segments(), depth)
}

pub(super) fn visit_term_spec<'a>(
    walker: &mut Walker<'a>,
    spec: &'a TermSpec,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let TermSpec {
        term_type: _,
        datatype,
        language,
        base,
    } = spec;
    if let Some(datatype) = datatype {
        walker.push(depth, Work::NamedNode(datatype))?;
    }
    if let Some(language) = language {
        walker.payload(language.len())?;
    }
    if let Some(base) = base {
        walker.payload(base.len())?;
    }
    Ok(())
}

pub(super) fn visit_segment(
    walker: &mut Walker<'_>,
    segment: &Segment,
) -> Result<(), PlanMeasureError> {
    let (Segment::Literal(value) | Segment::Column(value)) = segment;
    walker.payload(value.len())
}

pub(super) fn push_segments<'a>(
    walker: &mut Walker<'a>,
    segments: &'a [Segment],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(segments.len())?;
    for segment in segments {
        walker.push(depth, Work::Segment(segment))?;
    }
    Ok(())
}

pub(super) fn push_term_maps<'a>(
    walker: &mut Walker<'a>,
    term_maps: &'a [TermMap],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(term_maps.len())?;
    for term_map in term_maps {
        walker.push(depth, Work::TermMap(term_map))?;
    }
    Ok(())
}

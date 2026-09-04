use sf_core::ir::{LogicalSource, Segment, Template, TermMap, TermSpec};

use super::{PlanMeasureError, Walker, Work};

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

//! Exact scalar and collection roots for prospective compiler clone charging.

use std::collections::BTreeMap;

use sf_core::query_control::QueryControl;

use ::spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
    PropertyPathExpression, QueryDataset,
};
use ::spargebra::term::{
    BlankNode, GroundTerm, GroundTriple, Literal, NamedNode, NamedNodePattern, NamedOrBlankNode,
    Term, TermPattern, Triple, TriplePattern, Variable,
};
use sf_core::ir::{
    Join, LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, Segment, SubjectMap,
    Template, TermMap, TermSpec, TriplesMap,
};

use super::{PlanMeasureError, PlanMeasureLimits, PlanMeasureV1, Walker, Work};
use crate::iq::node::{AggArg, AggDef, BindDef, ColOrConst, IqCond, IqNode, Var};
use crate::iq::{
    AggCol, Aggregation, Branch, ColRef, GroupKey, HopExpr, HopRelation, OptJoin, OrderKey,
    PathClosure, R2rmlGraphScope, RustAgg, RustGroup, Scan, SqlCond, SubPlanJoin, TermDef,
};
use crate::{DedupScope, Plan, PlanForm};

/// One standalone value copied by a concrete compiler `Clone` operation.
///
/// A scalar must use this surface rather than a one-element collection root:
/// the latter would charge a collection slot that the real operation does not
/// copy. The enum mirrors every carrier currently understood by the V1 walker,
/// and its explicit conversion prevents an exposed root from silently falling
/// back to approximate accounting.
#[allow(dead_code)] // Dormant until exact precharges are wired at clone sites.
#[derive(Clone, Copy)]
pub(crate) enum CompilerCloneRootV1<'a> {
    Query(&'a ::spargebra::Query),
    Plan(&'a Plan),
    PlanForm(&'a PlanForm),
    DedupScope(&'a DedupScope),
    Branch(&'a Branch),
    Scan(&'a Scan),
    OptJoin(&'a OptJoin),
    ColRef(&'a ColRef),
    TermDef(&'a TermDef),
    GraphScope(&'a R2rmlGraphScope),
    SqlCond(&'a SqlCond),
    OrderKey(&'a OrderKey),
    PathClosure(&'a PathClosure),
    HopExpr(&'a HopExpr),
    HopRelation(&'a HopRelation),
    Aggregation(&'a Aggregation),
    GroupKey(&'a GroupKey),
    AggCol(&'a AggCol),
    SubPlanJoin(&'a SubPlanJoin),
    RustGroup(&'a RustGroup),
    RustAgg(&'a RustAgg),
    LogicalSource(&'a LogicalSource),
    TriplesMap(&'a TriplesMap),
    SubjectMap(&'a SubjectMap),
    PredicateObjectMap(&'a PredicateObjectMap),
    ObjectMap(&'a ObjectMap),
    RefObjectMap(&'a RefObjectMap),
    Join(&'a Join),
    TermMap(&'a TermMap),
    Template(&'a Template),
    TermSpec(&'a TermSpec),
    Segment(&'a Segment),
    Expression(&'a Expression),
    Function(&'a Function),
    GraphPattern(&'a GraphPattern),
    PropertyPath(&'a PropertyPathExpression),
    AggregateExpression(&'a AggregateExpression),
    AggregateFunction(&'a AggregateFunction),
    OrderExpression(&'a OrderExpression),
    QueryDataset(&'a QueryDataset),
    TriplePattern(&'a TriplePattern),
    TermPattern(&'a TermPattern),
    NamedNodePattern(&'a NamedNodePattern),
    GroundTerm(&'a GroundTerm),
    GroundTriple(&'a GroundTriple),
    Term(&'a Term),
    Triple(&'a Triple),
    NamedOrBlankNode(&'a NamedOrBlankNode),
    NamedNode(&'a NamedNode),
    BlankNode(&'a BlankNode),
    Literal(&'a Literal),
    Variable(&'a Variable),
    IqNode(&'a IqNode),
    IqCond(&'a IqCond),
    IqAggDef(&'a AggDef),
    IqAggArg(&'a AggArg),
    IqColOrConst(&'a ColOrConst),
    IqBindDef(&'a BindDef),
}

/// One collection copied independently of an enclosing compiler value.
///
/// Nested row roots are distinct: `TermDefRow` charges only one row, whereas
/// `TermDefRows` also charges the outer vector. This prevents convenient
/// wrappers from changing the prospective schedule.
#[allow(dead_code)] // Dormant until exact precharges are wired at clone sites.
#[derive(Clone, Copy)]
pub(crate) enum CompilerCloneCollectionV1<'a> {
    Branches(&'a [Branch]),
    Scans(&'a [Scan]),
    SqlConditions(&'a [SqlCond]),
    Bindings(&'a BTreeMap<String, TermDef>),
    OptJoins(&'a [OptJoin]),
    SubPlanJoins(&'a [SubPlanJoin]),
    Segments(&'a [Segment]),
    TriplePatterns(&'a [TriplePattern]),
    IqNodes(&'a [IqNode]),
    IqConditions(&'a [IqCond]),
    IqSubstitution(&'a BTreeMap<Var, BindDef>),
    /// Variable-only copies have slots and payload bytes but no walker nodes;
    /// depth and pending-item counts describe the measurement stack, not
    /// Rust's `Clone` call stack, and are therefore both zero for this root.
    Variables(&'a [Var]),
    OrderKeys(&'a [OrderKey]),
    TermDefRows(&'a [Vec<Option<TermDef>>]),
    TermDefRow(&'a [Option<TermDef>]),
    GroundRows(&'a [Vec<Option<GroundTerm>>]),
    GroundRow(&'a [Option<GroundTerm>]),
    TriplesMaps(&'a [TriplesMap]),
}

#[cfg(test)]
pub(crate) fn measure_compiler_clone_root_v1(
    root: CompilerCloneRootV1<'_>,
) -> Result<PlanMeasureV1, PlanMeasureError> {
    measure_compiler_clone_root_with_limits(root, PlanMeasureLimits::V1)
}

#[cfg(test)]
pub(crate) fn measure_compiler_clone_collection_v1(
    collection: CompilerCloneCollectionV1<'_>,
) -> Result<PlanMeasureV1, PlanMeasureError> {
    measure_compiler_clone_collection_with_limits(collection, PlanMeasureLimits::V1)
}

pub(crate) fn measure_compiler_clone_root_with_control(
    root: CompilerCloneRootV1<'_>,
    control: &dyn QueryControl,
) -> Result<PlanMeasureV1, PlanMeasureError> {
    Walker::controlled(PlanMeasureLimits::V1, control)?.run(root.into_work())
}

pub(crate) fn measure_compiler_clone_collection_with_control(
    collection: CompilerCloneCollectionV1<'_>,
    control: &dyn QueryControl,
) -> Result<PlanMeasureV1, PlanMeasureError> {
    measure_collection(
        collection,
        Walker::controlled(PlanMeasureLimits::V1, control)?,
    )
}

#[cfg(test)]
pub(super) fn measure_compiler_clone_root_with_limits(
    root: CompilerCloneRootV1<'_>,
    limits: PlanMeasureLimits,
) -> Result<PlanMeasureV1, PlanMeasureError> {
    Walker::new(limits).run(root.into_work())
}

#[cfg(test)]
pub(super) fn measure_compiler_clone_collection_with_limits(
    collection: CompilerCloneCollectionV1<'_>,
    limits: PlanMeasureLimits,
) -> Result<PlanMeasureV1, PlanMeasureError> {
    measure_collection(collection, Walker::new(limits))
}

fn measure_collection<'a>(
    collection: CompilerCloneCollectionV1<'a>,
    mut walker: Walker<'a>,
) -> Result<PlanMeasureV1, PlanMeasureError> {
    match collection {
        CompilerCloneCollectionV1::Branches(values) => {
            push_roots(&mut walker, values, Work::Branch)?
        }
        CompilerCloneCollectionV1::Scans(values) => push_roots(&mut walker, values, Work::Scan)?,
        CompilerCloneCollectionV1::SqlConditions(values) => {
            push_roots(&mut walker, values, Work::SqlCond)?
        }
        CompilerCloneCollectionV1::Bindings(values) => push_bindings(&mut walker, values)?,
        CompilerCloneCollectionV1::OptJoins(values) => {
            push_roots(&mut walker, values, Work::OptJoin)?
        }
        CompilerCloneCollectionV1::SubPlanJoins(values) => {
            push_roots(&mut walker, values, Work::SubPlanJoin)?
        }
        CompilerCloneCollectionV1::Segments(values) => {
            push_roots(&mut walker, values, Work::Segment)?
        }
        CompilerCloneCollectionV1::TriplePatterns(values) => {
            push_roots(&mut walker, values, Work::TriplePattern)?
        }
        CompilerCloneCollectionV1::IqNodes(values) => {
            push_roots(&mut walker, values, Work::IqNode)?
        }
        CompilerCloneCollectionV1::IqConditions(values) => {
            push_roots(&mut walker, values, Work::IqCond)?
        }
        CompilerCloneCollectionV1::IqSubstitution(values) => {
            push_iq_substitution(&mut walker, values)?
        }
        CompilerCloneCollectionV1::Variables(values) => {
            super::iq::measure_variables(&mut walker, values)?
        }
        CompilerCloneCollectionV1::OrderKeys(values) => {
            push_roots(&mut walker, values, Work::OrderKey)?
        }
        CompilerCloneCollectionV1::TermDefRows(values) => push_term_def_rows(&mut walker, values)?,
        CompilerCloneCollectionV1::TermDefRow(value) => push_term_def_row(&mut walker, value)?,
        CompilerCloneCollectionV1::GroundRows(values) => push_ground_rows(&mut walker, values)?,
        CompilerCloneCollectionV1::GroundRow(value) => push_ground_row(&mut walker, value)?,
        CompilerCloneCollectionV1::TriplesMaps(values) => {
            push_roots(&mut walker, values, Work::TriplesMap)?
        }
    }
    walker.finish()
}

impl<'a> CompilerCloneRootV1<'a> {
    fn into_work(self) -> Work<'a> {
        match self {
            Self::Query(value) => Work::Query(value),
            Self::Plan(value) => Work::Plan(value),
            Self::PlanForm(value) => Work::PlanForm(value),
            Self::DedupScope(value) => Work::DedupScope(value),
            Self::Branch(value) => Work::Branch(value),
            Self::Scan(value) => Work::Scan(value),
            Self::OptJoin(value) => Work::OptJoin(value),
            Self::ColRef(value) => Work::ColRef(value),
            Self::TermDef(value) => Work::TermDef(value),
            Self::GraphScope(value) => Work::GraphScope(value),
            Self::SqlCond(value) => Work::SqlCond(value),
            Self::OrderKey(value) => Work::OrderKey(value),
            Self::PathClosure(value) => Work::PathClosure(value),
            Self::HopExpr(value) => Work::HopExpr(value),
            Self::HopRelation(value) => Work::HopRelation(value),
            Self::Aggregation(value) => Work::Aggregation(value),
            Self::GroupKey(value) => Work::GroupKey(value),
            Self::AggCol(value) => Work::AggCol(value),
            Self::SubPlanJoin(value) => Work::SubPlanJoin(value),
            Self::RustGroup(value) => Work::RustGroup(value),
            Self::RustAgg(value) => Work::RustAgg(value),
            Self::LogicalSource(value) => Work::LogicalSource(value),
            Self::TriplesMap(value) => Work::TriplesMap(value),
            Self::SubjectMap(value) => Work::SubjectMap(value),
            Self::PredicateObjectMap(value) => Work::PredicateObjectMap(value),
            Self::ObjectMap(value) => Work::ObjectMap(value),
            Self::RefObjectMap(value) => Work::RefObjectMap(value),
            Self::Join(value) => Work::Join(value),
            Self::TermMap(value) => Work::TermMap(value),
            Self::Template(value) => Work::Template(value),
            Self::TermSpec(value) => Work::TermSpec(value),
            Self::Segment(value) => Work::Segment(value),
            Self::Expression(value) => Work::Expression(value),
            Self::Function(value) => Work::Function(value),
            Self::GraphPattern(value) => Work::GraphPattern(value),
            Self::PropertyPath(value) => Work::PropertyPath(value),
            Self::AggregateExpression(value) => Work::AggregateExpression(value),
            Self::AggregateFunction(value) => Work::AggregateFunction(value),
            Self::OrderExpression(value) => Work::OrderExpression(value),
            Self::QueryDataset(value) => Work::QueryDataset(value),
            Self::TriplePattern(value) => Work::TriplePattern(value),
            Self::TermPattern(value) => Work::TermPattern(value),
            Self::NamedNodePattern(value) => Work::NamedNodePattern(value),
            Self::GroundTerm(value) => Work::GroundTerm(value),
            Self::GroundTriple(value) => Work::GroundTriple(value),
            Self::Term(value) => Work::Term(value),
            Self::Triple(value) => Work::Triple(value),
            Self::NamedOrBlankNode(value) => Work::NamedOrBlankNode(value),
            Self::NamedNode(value) => Work::NamedNode(value),
            Self::BlankNode(value) => Work::BlankNode(value),
            Self::Literal(value) => Work::Literal(value),
            Self::Variable(value) => Work::Variable(value),
            Self::IqNode(value) => Work::IqNode(value),
            Self::IqCond(value) => Work::IqCond(value),
            Self::IqAggDef(value) => Work::IqAggDef(value),
            Self::IqAggArg(value) => Work::IqAggArg(value),
            Self::IqColOrConst(value) => Work::IqColOrConst(value),
            Self::IqBindDef(value) => Work::IqBindDef(value),
        }
    }
}

/// The whole query owns its dataset, template and base as well as its graph.
pub(super) fn visit_query<'a>(
    walker: &mut Walker<'a>,
    query: &'a ::spargebra::Query,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    use ::spargebra::Query;
    let (dataset, pattern, base_iri) = match query {
        Query::Select {
            dataset,
            pattern,
            base_iri,
        }
        | Query::Ask {
            dataset,
            pattern,
            base_iri,
        }
        | Query::Describe {
            dataset,
            pattern,
            base_iri,
        } => (dataset, pattern, base_iri),
        Query::Construct {
            template,
            dataset,
            pattern,
            base_iri,
        } => {
            walker.collection(template.len())?;
            for triple in template {
                walker.push(depth, Work::TriplePattern(triple))?;
            }
            (dataset, pattern, base_iri)
        }
    };
    if let Some(base) = base_iri {
        walker.payload(base.as_str().len())?;
    }
    walker.push(depth, Work::GraphPattern(pattern))?;
    if let Some(dataset) = dataset {
        walker.push(depth, Work::QueryDataset(dataset))?;
    }
    Ok(())
}

fn push_roots<'a, T>(
    walker: &mut Walker<'a>,
    values: &'a [T],
    root: impl Fn(&'a T) -> Work<'a>,
) -> Result<(), PlanMeasureError> {
    walker.collection(values.len())?;
    for value in values {
        walker.push(0, root(value))?;
    }
    Ok(())
}

fn push_bindings<'a>(
    walker: &mut Walker<'a>,
    values: &'a BTreeMap<String, TermDef>,
) -> Result<(), PlanMeasureError> {
    walker.collection(values.len())?;
    for (name, definition) in values {
        walker.payload(name.len())?;
        walker.push(0, Work::TermDef(definition))?;
    }
    Ok(())
}

fn push_iq_substitution<'a>(
    walker: &mut Walker<'a>,
    values: &'a BTreeMap<Var, BindDef>,
) -> Result<(), PlanMeasureError> {
    walker.collection(values.len())?;
    for (variable, definition) in values {
        walker.payload(variable.len())?;
        walker.push(0, Work::IqBindDef(definition))?;
    }
    Ok(())
}

fn push_term_def_rows<'a>(
    walker: &mut Walker<'a>,
    rows: &'a [Vec<Option<TermDef>>],
) -> Result<(), PlanMeasureError> {
    walker.collection(rows.len())?;
    for row in rows {
        push_term_def_row(walker, row)?;
    }
    Ok(())
}

fn push_term_def_row<'a>(
    walker: &mut Walker<'a>,
    row: &'a [Option<TermDef>],
) -> Result<(), PlanMeasureError> {
    walker.collection(row.len())?;
    for value in row {
        walker.checkpoint()?;
        if let Some(value) = value {
            walker.push(0, Work::TermDef(value))?;
        }
    }
    Ok(())
}

fn push_ground_rows<'a>(
    walker: &mut Walker<'a>,
    rows: &'a [Vec<Option<GroundTerm>>],
) -> Result<(), PlanMeasureError> {
    walker.collection(rows.len())?;
    for row in rows {
        push_ground_row(walker, row)?;
    }
    Ok(())
}

fn push_ground_row<'a>(
    walker: &mut Walker<'a>,
    row: &'a [Option<GroundTerm>],
) -> Result<(), PlanMeasureError> {
    walker.collection(row.len())?;
    for value in row {
        walker.checkpoint()?;
        if let Some(value) = value {
            walker.push(0, Work::GroundTerm(value))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "clone_root_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) use super::test_support::{measure_copy_collection, measure_copy_root};

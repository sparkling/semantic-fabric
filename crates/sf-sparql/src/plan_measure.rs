//! Deterministic V1 measurement of compiler-owned clone graphs.
//!
//! The metric is deliberately independent of allocator layout: one unit per
//! visited clone carrier, collection slot, and owned payload byte. It is meant
//! for precharging exact plan, branch-forest, or IQ-fragment clones, not as a
//! byte-size estimate. Controlled compiler copies separately pay measurement
//! traversal and logical work-stack growth; raw measurements retain V1 metrics.
//!
//! The measurement walk is iterative. It does not make the model's derived
//! `Clone` implementations iterative or allocation-fallible.

use std::collections::TryReserveError;
use std::fmt;

use sf_core::query_control::{QueryControl, QueryControlError};

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

use crate::iq::node::{AggArg, AggDef, BindDef, ColOrConst, IqCond, IqNode};
use crate::iq::{
    AggCol, Aggregation, Branch, ColRef, GroupKey, HopExpr, HopRelation, OptJoin, OrderKey,
    PathClosure, R2rmlGraphScope, RustAgg, RustGroup, Scan, SqlCond, SubPlanJoin, TermDef,
};
use crate::{DedupScope, Plan, PlanForm};

pub(crate) mod clone_root;
mod control;
mod iq;
mod mapping;
mod model;
mod plan;
mod spargebra;

/// These independent ceilings must receive a new profile identity if changed
/// after the primitive is integrated. Values are conservative, not calibrated.
const MAX_PLAN_NODES_V1: u64 = 256 * 1024;
const MAX_PLAN_COLLECTION_SLOTS_V1: u64 = 256 * 1024;
const MAX_PLAN_PAYLOAD_BYTES_V1: u64 = 16 * 1024 * 1024;
const MAX_PLAN_DEPTH_V1: usize = 128;
const MAX_PLAN_PENDING_ITEMS_V1: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanMeasureLimit {
    Nodes,
    CollectionSlots,
    PayloadBytes,
    Depth,
    PendingItems,
}

impl fmt::Display for PlanMeasureLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Nodes => "nodes",
            Self::CollectionSlots => "collection-slots",
            Self::PayloadBytes => "payload-bytes",
            Self::Depth => "depth",
            Self::PendingItems => "pending-items",
        })
    }
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum PlanMeasureError {
    #[error("plan measure V1 {dimension} limit exceeded ({observed}>{maximum})")]
    LimitExceeded {
        dimension: PlanMeasureLimit,
        observed: u64,
        maximum: u64,
    },
    #[error("plan measure V1 arithmetic overflow")]
    AccountingOverflow,
    #[error("plan measure V1 work-stack allocation failed")]
    AllocationFailed,
    #[error("plan measurement interrupted: {0}")]
    Control(#[from] QueryControlError),
}

impl From<TryReserveError> for PlanMeasureError {
    fn from(_: TryReserveError) -> Self {
        Self::AllocationFailed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlanMeasureV1 {
    pub(crate) nodes: u64,
    pub(crate) collection_slots: u64,
    pub(crate) payload_bytes: u64,
    pub(crate) deep_clone_work: u64,
    pub(crate) max_depth: usize,
    pub(crate) max_pending_items: usize,
}

impl PlanMeasureV1 {
    pub(crate) fn measure(plan: &Plan) -> Result<Self, PlanMeasureError> {
        Walker::new(PlanMeasureLimits::V1).run(Work::Plan(plan))
    }
}

#[derive(Clone, Copy)]
struct PlanMeasureLimits {
    max_nodes: u64,
    max_collection_slots: u64,
    max_payload_bytes: u64,
    max_depth: usize,
    max_pending_items: usize,
}

impl PlanMeasureLimits {
    const V1: Self = Self {
        max_nodes: MAX_PLAN_NODES_V1,
        max_collection_slots: MAX_PLAN_COLLECTION_SLOTS_V1,
        max_payload_bytes: MAX_PLAN_PAYLOAD_BYTES_V1,
        max_depth: MAX_PLAN_DEPTH_V1,
        max_pending_items: MAX_PLAN_PENDING_ITEMS_V1,
    };
}

#[derive(Clone, Copy)]
struct Pending<'a> {
    work: Work<'a>,
    depth: usize,
}

#[derive(Clone, Copy)]
enum Work<'a> {
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

struct Walker<'a> {
    limits: PlanMeasureLimits,
    stack: Vec<Pending<'a>>,
    logical_capacity: usize,
    control: Option<&'a dyn QueryControl>,
    measure: PlanMeasureV1,
}

impl<'a> Walker<'a> {
    fn new(limits: PlanMeasureLimits) -> Self {
        Self {
            limits,
            stack: Vec::new(),
            logical_capacity: 0,
            control: None,
            measure: PlanMeasureV1 {
                nodes: 0,
                collection_slots: 0,
                payload_bytes: 0,
                deep_clone_work: 0,
                max_depth: 0,
                max_pending_items: 0,
            },
        }
    }

    fn run(mut self, root: Work<'a>) -> Result<PlanMeasureV1, PlanMeasureError> {
        self.push_at(root, 1)?;
        self.finish()
    }

    fn finish(mut self) -> Result<PlanMeasureV1, PlanMeasureError> {
        while !self.stack.is_empty() {
            self.checkpoint()?;
            let Pending { work, depth } = self.stack.pop().expect("nonempty work stack");
            self.record_node()?;
            match work {
                Work::Plan(value) => plan::visit_plan(&mut self, value, depth)?,
                Work::PlanForm(value) => plan::visit_plan_form(&mut self, value, depth)?,
                Work::DedupScope(value) => plan::visit_dedup_scope(&mut self, value, depth)?,
                Work::Branch(value) => model::visit_branch(&mut self, value, depth)?,
                Work::Scan(value) => model::visit_scan(&mut self, value, depth)?,
                Work::OptJoin(value) => model::visit_opt_join(&mut self, value, depth)?,
                Work::ColRef(value) => model::visit_col_ref(&mut self, value)?,
                Work::TermDef(value) => model::visit_term_def(&mut self, value, depth)?,
                Work::GraphScope(value) => model::visit_graph_scope(&mut self, value, depth)?,
                Work::SqlCond(value) => model::visit_sql_cond(&mut self, value, depth)?,
                Work::OrderKey(value) => model::visit_order_key(&mut self, value, depth)?,
                Work::PathClosure(value) => model::visit_path_closure(&mut self, value, depth)?,
                Work::HopExpr(value) => model::visit_hop_expr(&mut self, value, depth)?,
                Work::HopRelation(value) => model::visit_hop_relation(&mut self, value, depth)?,
                Work::Aggregation(value) => model::visit_aggregation(&mut self, value, depth)?,
                Work::GroupKey(value) => model::visit_group_key(&mut self, value, depth)?,
                Work::AggCol(value) => model::visit_agg_col(&mut self, value, depth)?,
                Work::SubPlanJoin(value) => model::visit_subplan(&mut self, value, depth)?,
                Work::RustGroup(value) => model::visit_rust_group(&mut self, value, depth)?,
                Work::RustAgg(value) => model::visit_rust_agg(&mut self, value)?,
                Work::LogicalSource(value) => mapping::visit_logical_source(&mut self, value)?,
                Work::TriplesMap(value) => mapping::visit_triples_map(&mut self, value, depth)?,
                Work::SubjectMap(value) => mapping::visit_subject_map(&mut self, value, depth)?,
                Work::PredicateObjectMap(value) => {
                    mapping::visit_predicate_object_map(&mut self, value, depth)?
                }
                Work::ObjectMap(value) => mapping::visit_object_map(&mut self, value, depth)?,
                Work::RefObjectMap(value) => {
                    mapping::visit_ref_object_map(&mut self, value, depth)?
                }
                Work::Join(value) => mapping::visit_join(&mut self, value)?,
                Work::TermMap(value) => mapping::visit_term_map(&mut self, value, depth)?,
                Work::Template(value) => mapping::visit_template(&mut self, value, depth)?,
                Work::TermSpec(value) => mapping::visit_term_spec(&mut self, value, depth)?,
                Work::Segment(value) => mapping::visit_segment(&mut self, value)?,
                Work::Expression(value) => spargebra::visit_expression(&mut self, value, depth)?,
                Work::Function(value) => spargebra::visit_function(&mut self, value, depth)?,
                Work::GraphPattern(value) => {
                    spargebra::visit_graph_pattern(&mut self, value, depth)?
                }
                Work::PropertyPath(value) => {
                    spargebra::visit_property_path(&mut self, value, depth)?
                }
                Work::AggregateExpression(value) => {
                    spargebra::visit_aggregate_expression(&mut self, value, depth)?
                }
                Work::AggregateFunction(value) => {
                    spargebra::visit_aggregate_function(&mut self, value, depth)?
                }
                Work::OrderExpression(value) => {
                    spargebra::visit_order_expression(&mut self, value, depth)?
                }
                Work::QueryDataset(value) => {
                    spargebra::visit_query_dataset(&mut self, value, depth)?
                }
                Work::TriplePattern(value) => {
                    spargebra::visit_triple_pattern(&mut self, value, depth)?
                }
                Work::TermPattern(value) => spargebra::visit_term_pattern(&mut self, value, depth)?,
                Work::NamedNodePattern(value) => {
                    spargebra::visit_named_node_pattern(&mut self, value, depth)?
                }
                Work::GroundTerm(value) => spargebra::visit_ground_term(&mut self, value, depth)?,
                Work::GroundTriple(value) => {
                    spargebra::visit_ground_triple(&mut self, value, depth)?
                }
                Work::Term(value) => spargebra::visit_term(&mut self, value, depth)?,
                Work::Triple(value) => spargebra::visit_triple(&mut self, value, depth)?,
                Work::NamedOrBlankNode(value) => {
                    spargebra::visit_named_or_blank(&mut self, value, depth)?
                }
                Work::NamedNode(value) => self.payload(value.as_str().len())?,
                Work::BlankNode(value) => self.payload(value.as_str().len())?,
                Work::Literal(value) => spargebra::visit_literal(&mut self, value, depth)?,
                Work::Variable(value) => self.payload(value.as_str().len())?,
                Work::IqNode(value) => iq::visit_node(&mut self, value, depth)?,
                Work::IqCond(value) => iq::visit_condition(&mut self, value, depth)?,
                Work::IqAggDef(value) => iq::visit_aggregate(&mut self, value, depth)?,
                Work::IqAggArg(value) => iq::visit_aggregate_arg(&mut self, value, depth)?,
                Work::IqColOrConst(value) => iq::visit_col_or_const(&mut self, value, depth)?,
                Work::IqBindDef(value) => iq::visit_bind_def(&mut self, value, depth)?,
            }
        }
        self.checkpoint()?;
        Ok(self.measure)
    }

    fn push(&mut self, parent_depth: usize, work: Work<'a>) -> Result<(), PlanMeasureError> {
        let depth = parent_depth
            .checked_add(1)
            .ok_or(PlanMeasureError::AccountingOverflow)?;
        self.push_at(work, depth)
    }

    fn push_at(&mut self, work: Work<'a>, depth: usize) -> Result<(), PlanMeasureError> {
        self.charge(1)?;
        enforce_usize(PlanMeasureLimit::Depth, depth, self.limits.max_depth)?;
        let pending = self
            .stack
            .len()
            .checked_add(1)
            .ok_or(PlanMeasureError::AccountingOverflow)?;
        enforce_usize(
            PlanMeasureLimit::PendingItems,
            pending,
            self.limits.max_pending_items,
        )?;
        self.reserve_stack()?;
        self.stack.push(Pending { work, depth });
        self.measure.max_depth = self.measure.max_depth.max(depth);
        self.measure.max_pending_items = self.measure.max_pending_items.max(pending);
        Ok(())
    }

    fn collection(&mut self, slots: usize) -> Result<(), PlanMeasureError> {
        self.charge(slots)?;
        let slots = u64::try_from(slots).map_err(|_| PlanMeasureError::AccountingOverflow)?;
        self.record(
            PlanMeasureLimit::CollectionSlots,
            slots,
            self.limits.max_collection_slots,
        )
    }

    fn payload(&mut self, bytes: usize) -> Result<(), PlanMeasureError> {
        let bytes = u64::try_from(bytes).map_err(|_| PlanMeasureError::AccountingOverflow)?;
        self.record(
            PlanMeasureLimit::PayloadBytes,
            bytes,
            self.limits.max_payload_bytes,
        )
    }

    fn record_node(&mut self) -> Result<(), PlanMeasureError> {
        self.record(PlanMeasureLimit::Nodes, 1, self.limits.max_nodes)
    }

    /// Account for an owned leaf hidden behind a read-only upstream accessor.
    /// The leaf cannot be put on the work stack, but its clone depth and payload
    /// are still part of the deterministic schedule.
    fn inline_leaf(&mut self, parent_depth: usize, bytes: usize) -> Result<(), PlanMeasureError> {
        let depth = parent_depth
            .checked_add(1)
            .ok_or(PlanMeasureError::AccountingOverflow)?;
        enforce_usize(PlanMeasureLimit::Depth, depth, self.limits.max_depth)?;
        self.record_node()?;
        self.payload(bytes)?;
        self.measure.max_depth = self.measure.max_depth.max(depth);
        Ok(())
    }

    fn record(
        &mut self,
        dimension: PlanMeasureLimit,
        amount: u64,
        maximum: u64,
    ) -> Result<(), PlanMeasureError> {
        self.charge(1)?;
        let current = match dimension {
            PlanMeasureLimit::Nodes => self.measure.nodes,
            PlanMeasureLimit::CollectionSlots => self.measure.collection_slots,
            PlanMeasureLimit::PayloadBytes => self.measure.payload_bytes,
            PlanMeasureLimit::Depth | PlanMeasureLimit::PendingItems => unreachable!(),
        };
        let observed = current
            .checked_add(amount)
            .ok_or(PlanMeasureError::AccountingOverflow)?;
        if observed > maximum {
            return Err(PlanMeasureError::LimitExceeded {
                dimension,
                observed,
                maximum,
            });
        }
        let clone_work = self
            .measure
            .deep_clone_work
            .checked_add(amount)
            .ok_or(PlanMeasureError::AccountingOverflow)?;
        match dimension {
            PlanMeasureLimit::Nodes => self.measure.nodes = observed,
            PlanMeasureLimit::CollectionSlots => self.measure.collection_slots = observed,
            PlanMeasureLimit::PayloadBytes => self.measure.payload_bytes = observed,
            PlanMeasureLimit::Depth | PlanMeasureLimit::PendingItems => unreachable!(),
        }
        self.measure.deep_clone_work = clone_work;
        Ok(())
    }
}

fn enforce_usize(
    dimension: PlanMeasureLimit,
    observed: usize,
    maximum: usize,
) -> Result<(), PlanMeasureError> {
    if observed <= maximum {
        return Ok(());
    }
    Err(PlanMeasureError::LimitExceeded {
        dimension,
        observed: u64::try_from(observed).map_err(|_| PlanMeasureError::AccountingOverflow)?,
        maximum: u64::try_from(maximum).map_err(|_| PlanMeasureError::AccountingOverflow)?,
    })
}

#[cfg(test)]
#[path = "plan_measure/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "plan_measure/clone_roots_tests.rs"]
mod clone_roots_tests;

#[cfg(test)]
#[path = "plan_measure/control_tests.rs"]
mod control_tests;

#[cfg(test)]
pub(crate) mod test_support;

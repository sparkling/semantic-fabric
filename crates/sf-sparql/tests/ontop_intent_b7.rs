//! Ontop-parity intent batch 7 of 8 (ADR-0021/ADR-0022).
//!
//! No Ontop optimizer test classes are assigned to this batch. The 33 classes
//! (8 executor + 25 optimizer) are fully covered by batches 0–6:
//!
//!   batch 0 (indices  0– 4): EmptyNodeRemoval, FunctionalDependency, LeftJoinOptimization, LJJoinLift, QueryMerging
//!   batch 1 (indices  5– 9): RedundantJoinFK, RedundantSelfJoin, SubstitutionPropagation, AggregationSplitter, BindingLift
//!   batch 2 (indices 10–14): ConjunctionOfDisjunctionsMerging, ConstructionNodeCleaner, Distinct, ExpressionEvaluator, FlattenLift
//!   batch 3 (indices 15–19): FlattenUnionOptimizer, FunctionalDependencyInference, NodeDeletion, NoQueryContext, NRAJoinOptimizer
//!   batch 4 (indices 20–24): Nullability, NullableUniqueConstraint, PreventDistinct, ProjectionShrinking, PullOutVariable
//!   batch 5 (indices 25–29): PushDownBooleanExpression, PushUpBooleanExpression, SelfJoinSameTerms, TrueNodesRemoval, UniqueConstraintInference
//!   batch 6 (indices 30–32): UriTemplate, ValuesNodeOptimization, ValuesNodeTest
//!   batch 7 (this file):     — (empty)

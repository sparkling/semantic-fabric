//! Driver-agnostic SPARQL execution core (ADR-0024).
//!
//! Public query-form entry points stay on this facade. Their implementation is
//! split by responsibility so execution, reconstruction, ordering, expression
//! evaluation, template instantiation, and aggregation can evolve independently.

mod aggregation;
mod batch;
mod driver;
mod expression;
mod forms;
mod order;
mod row;
mod template;

#[allow(unused_imports)]
pub(crate) use aggregation::rust_group_result_rows;
pub(crate) use driver::{block_on, dedup_group_alias};
#[allow(unused_imports)]
pub(crate) use expression::eval_expr;
pub(crate) use forms::construct_may_need_cross_branch_dedup;
pub use forms::{
    ask, construct, construct_each_async, construct_triples, dump_quads, dump_quads_stream, select,
    select_each, select_each_async, Solutions,
};
#[allow(unused_imports)]
pub(crate) use row::{reconstruct, Bindings, RawRow};
#[allow(unused_imports)]
pub(crate) use template::instantiate;

#[cfg(test)]
#[path = "exec_core/batch_loop_tests.rs"]
mod batch_loop_tests;
#[cfg(test)]
#[path = "exec_core/batch_reconstruct_tests.rs"]
mod batch_reconstruct_tests;
#[cfg(test)]
#[path = "exec_core/bindings_tests.rs"]
mod bindings_tests;
#[cfg(test)]
#[path = "exec_core_deadline_tests.rs"]
mod deadline_checkpoint_tests;
#[cfg(test)]
#[path = "exec_core/order_sort_key_tests.rs"]
mod order_sort_key_tests;
#[cfg(test)]
#[path = "exec_core/probe_backend_tests.rs"]
mod probe_backend;
#[cfg(test)]
#[path = "exec_core/triple_function_tests.rs"]
mod triple_function_tests;

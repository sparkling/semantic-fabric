//! Emit — render an optimized [`Branch`] to dialect SQL (ADR-0007 step 6).
//!
//! Two invariants from the substrate (ADR-0010 §A / R1, ADR-0006):
//!
//! * **Values are bound parameters only.** Every constant from the query becomes
//!   a placeholder (`?` / `$n`) and its lexical value is returned in
//!   [`EmittedBranch::params`]; nothing is interpolated into the SQL text.
//! * **AST, not string assembly.** The rendered skeleton is round-tripped through
//!   the `sqlparser` AST via [`sf_sql::Dialect::emit_via_ast`], so the emitted
//!   statement is the `Display` of a parsed tree.
//!
//! Term construction is **not** here — the SELECT projects raw key columns
//! (ADR-0007 lifting); RDF terms are built during reconstruction ([`crate::exec`]).
//!
//! **SQL identifier case-folding (SQL:2008 §5.4 / per-dialect).** An `rr:column` /
//! `rr:sqlQuery` output-column value carries the *author's* identifier. A regular
//! (unquoted) identifier is case-folded by the DBMS — PostgreSQL folds to
//! lowercase — so the mapping's `"StudentId"` must bind to the column the source
//! actually exposes (`studentid`). Each emitted column reference is therefore
//! resolved against the source's *introspected* column names ([`ColumnCatalog`]):
//! an **exact** match wins (a delimited, case-exact identifier), else a **single
//! ASCII-case-insensitive** match (the regular-identifier folding the W3C suite
//! and every dialect honour). Live execution rejects missing, duplicate, and
//! case-fold-ambiguous metadata before opening a cursor; dialect-neutral
//! [`emit_branch`] remains permissive and emits an unresolved identifier as written.
//! Reconstruction is untouched: it reads result columns by position and matches
//! the *raw* IR column strings, so only the rendered SQL text changes here.

use std::collections::{HashMap, HashSet};

use sf_core::ir::{LogicalSource, Segment, TermMap};
use sf_sql::backend::{NativeScalarKey, SqliteDecode, TextKey};
use sf_sql::Dialect;

use crate::iq::{
    collect_cond_cols, AggCol, AggKind, Aggregation, Branch, ColRef, HopExpr, OrderKey,
    PathClosure, PathKind, R2rmlGraphScope, SqlCond, StrMatchOp, TermDef,
};
use crate::parser_isolation;
use crate::{Error, Result};
mod metadata;
mod metadata_path;
mod metadata_projection;
mod metadata_ref_atom;
mod metadata_source;
mod metadata_subplan;
use metadata::branch_actuals_controlled;
mod condition_control;
mod condition_leaf;
mod condition_metadata;
mod path_sql;
mod scan;
mod source_control;
#[cfg(test)]
use metadata_source::source_actuals;
use metadata_source::source_actuals_controlled;
#[cfg(test)]
mod metadata_tests;
#[cfg(test)]
use scan::scan_actuals;
use scan::scan_ref_controlled;
pub(crate) use source_control::{
    live_metadata_sources_controlled, source_probe_controlled, validate_definition_columns,
    SourceSet,
};
mod aggregate_projection;
pub(crate) use aggregate_projection::projection_controlled;
#[cfg(test)]
#[path = "emit/encoding_tests.rs"]
mod encoding_tests;
mod encoding_ucschar;
mod iri_cmp;
mod projection_layout;
#[cfg(test)]
pub(crate) use projection_layout::projection_layout_with_distinct;
pub(crate) use projection_layout::{projection_layout, source_projection};
mod lexical_key;
mod literal_cmp;
mod literal_datatype;
mod literal_roles;
mod mysql_decimal_value;
mod mysql_float_value;
mod native_literal_key;
mod natural_decimal;
mod natural_literal;
mod path_comparison;
mod pg_decimal_value;
mod pg_float;
mod pg_float_value;
mod pg_numeric;
mod ref_atom;
use aggregate_projection::{aggregate_projection, AggregateProjection};
#[cfg(test)]
use path_comparison::subplan_actuals;
use path_comparison::{path_actuals_controlled, render_key_equality};

mod binding_view;
mod branch;
mod branch_keys;
mod catalog;
mod from;
mod path_agg;
mod percent_encode;
mod source_oracle;
mod subplan;
mod template_sql;
#[cfg(test)]
#[path = "emit/tests.rs"]
mod tests;
mod validation;

// Moved items keep their `crate::emit` paths; siblings still resolve every
// formerly root-owned helper through `use super::*`.
pub(crate) use binding_view::{BindingIter, BindingView, BranchModifiers};
#[cfg(feature = "runtime-identity-evidence")]
pub(crate) use branch::exercise_raw_sql_fallback_for_evidence;
pub use branch::{emit_branch, emit_branch_with, EmittedBranch};
pub(crate) use branch::{
    emit_branch_binding_view, emit_branch_controlled, emit_branch_with_modifiers,
};
use branch::{emit_branch_inner, emit_via_ast_governed};
use branch_keys::{emit_branch_keys, order_column, push_limit_offset, render_order};
#[cfg(test)]
use catalog::branch_actuals;
pub use catalog::ColumnCatalog;
use catalog::{
    colref, physical_row_identifier, resolve_col, source_key, ActualColumns, AliasActuals,
    AliasSourceKind,
};
use from::{
    emit_subplan_join_controlled, render_cond_controlled, render_conjunction,
    render_from_async_controlled, render_from_controlled, render_where,
};
#[cfg(test)]
use from::{render_cond, scan_ref};
use path_agg::{agg_expr_sql, emit_agg_branch, emit_path_branch};
use percent_encode::{
    percent_encode_col, percent_encode_col_controlled, percent_encode_col_mysql,
    percent_encode_col_postgres, percent_encode_col_sqlite, MYSQL_GROUP_CONCAT_MAX_LEN,
    MYSQL_PACKET_RESERVE_BYTES, MYSQL_PERCENT_ENCODE_MAX_INPUT_BYTES,
};
use source_oracle::synthetic_subplan_catalog_controlled;
pub(crate) use source_oracle::{live_metadata_sources, synthetic_subplan_catalog};
#[cfg(test)]
use subplan::{emit_subplan_sql, emit_subplan_sql_controlled};
use subplan::{
    emit_subplan_sql_async_controlled, rebase_placeholders, rebase_placeholders_controlled,
};
pub(crate) use template_sql::{render_immediate_source_column, render_template_inline};
use template_sql::{render_template_concat, sql_string_literal};
pub(crate) use validation::validate_execution_columns;
#[cfg(test)]
pub(crate) use validation::{validate_live_columns, validate_live_columns_controlled};
pub(super) use validation::{validate_source_root, ValidationRoot};

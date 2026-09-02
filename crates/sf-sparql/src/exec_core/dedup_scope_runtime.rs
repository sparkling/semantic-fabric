//! Execution-time revalidation for persisted shared term-dedup scopes.

use std::collections::BTreeMap;

use crate::iq::Branch;
use crate::{DedupScope, Error, PlanForm, Result};

fn impure_scope() -> Error {
    Error::Unsupported(
        "shared term-dedup requires a pure single-source branch or unary SubPlan chain -> 501"
            .to_owned(),
    )
}

/// Revalidate a persisted/mutable public plan immediately before execution.
///
/// Compilation proves this shape once, but callers may mutate public `Plan` and
/// `Branch` fields while the private scope remains attached. Repeating the pure
/// branch/unary-wrapper proof here prevents such mutation from moving the seen
/// set past a join, OPTIONAL, path, aggregate, or slice.
pub(super) fn validate_runtime_scope(branch: &Branch, scope: &DedupScope) -> Result<()> {
    if branch.path.is_some()
        || branch.agg.is_some()
        || branch.limit.is_some()
        || branch.offset > 0
        || branch.nps
    {
        return Err(impure_scope());
    }
    let contributors = branch.core.len() + branch.opts.len() + branch.subplan_joins.len();
    if contributors != 1 || !branch.opts.is_empty() {
        return Err(impure_scope());
    }

    if let Some(scan) = branch.core.first() {
        if branch.core.len() != 1
            || scope.key_bindings.values().any(|definition| {
                definition
                    .columns()
                    .iter()
                    .any(|column| column.alias != scan.alias)
            })
            || scope.key_bindings.iter().any(|(variable, expected)| {
                branch
                    .bindings
                    .get(variable)
                    .is_some_and(|actual| format!("{actual:?}") != format!("{expected:?}"))
            })
        {
            return Err(impure_scope());
        }
        return Ok(());
    }

    let wrapper = branch.subplan_joins.first().ok_or_else(impure_scope)?;
    let nested = &wrapper.plan;
    if branch.subplan_joins.len() != 1
        || wrapper.left
        || !wrapper.on.is_empty()
        || nested.branches.len() != 1
        || nested.limit.is_some()
        || nested.offset > 0
        || nested.rust_group.is_some()
        || !matches!(nested.form, PlanForm::Select { .. })
        || !nested.dedup_scopes.is_empty()
        || scope.key_bindings.values().any(|definition| {
            definition
                .columns()
                .iter()
                .any(|column| column.alias != wrapper.alias)
        })
    {
        return Err(impure_scope());
    }
    let PlanForm::Select { vars } = &nested.form else {
        return Err(impure_scope());
    };
    if scope
        .key_bindings
        .keys()
        .any(|variable| !vars.contains(variable))
    {
        return Err(impure_scope());
    }
    let prepared = nested.prepared_branches();
    let nested_branch = prepared.first().ok_or_else(impure_scope)?;
    let nested_keys = scope
        .key_bindings
        .keys()
        .map(|variable| {
            nested_branch
                .bindings
                .get(variable)
                .cloned()
                .map(|definition| (variable.clone(), definition))
                .ok_or_else(impure_scope)
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let projection = crate::emit::emit_branch(nested_branch, nested.dialect)?.projection;
    for (variable, definition) in &nested_keys {
        let remapped = crate::iq::lower::remap_termdef(definition, &projection, wrapper.alias)?;
        if scope
            .key_bindings
            .get(variable)
            .is_none_or(|expected| format!("{expected:?}") != format!("{remapped:?}"))
        {
            return Err(impure_scope());
        }
    }
    validate_runtime_scope(
        nested_branch,
        &DedupScope {
            group_id: scope.group_id,
            key_bindings: nested_keys,
        },
    )
}

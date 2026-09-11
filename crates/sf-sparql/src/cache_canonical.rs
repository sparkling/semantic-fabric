//! Cache alpha normalization and narrow parser-output DESCRIBE normalization.
//!
//! A fresh global bijection preserves binding/correlation relationships. Only
//! unique aggregate definitions with no authored binding or projection escape
//! qualify. Constant DESCRIBE targets qualify only when their sole occurrences
//! are the root projection and its constant Extend. No spelling heuristic is
//! used; ambiguous roles retain their original names (a safe cache miss).

use std::borrow::Cow;

use sf_core::query_control::QueryControl;
use spargebra::algebra::GraphPattern;
use spargebra::Query;

use crate::build::control::BuildWork;
use crate::compile_envelope::algebra::AlgebraEnvelopeV1;
use crate::compiler_control::CompileContext;
use crate::plan_measure::clone_root::CompilerCloneRootV1;
use crate::{CompilerWorkMode, Result};

#[path = "cache_canonical/variables.rs"]
mod variables;
#[path = "cache_canonical/walk.rs"]
mod walk;

/// Stabilize eligible internal aggregate and constant DESCRIBE binders before
/// metered compilation. The historical function name is retained for both
/// direct and isolated parser callsites.
/// The isolated parser calls this inside its bounded worker, not in the parent.
/// Direct parsing retains caller-owned execution. This is semantic alpha
/// renaming, not a spelling heuristic: an authored isolated constant BIND can
/// have the same AST and qualifies too; observable/ambiguous roles are preserved.
pub(crate) fn normalize_describe_parse(query: &mut Query) -> Result<()> {
    use crate::compile_envelope::CompileEnvelopeError;
    use sf_core::query_control::QueryControlError;
    let envelope = AlgebraEnvelopeV1::validate(query).map_err(|error| match error {
        CompileEnvelopeError::AllocationFailed => QueryControlError::CompilerResourceExhausted,
        _ => QueryControlError::CompilerEnvelopeExceeded,
    })?;
    if !envelope.cache_internal_binders {
        return Ok(());
    }
    let work = BuildWork::new(CompilerWorkMode::Uncontrolled);
    if let Query::Select { pattern, .. } = query {
        if !explicit_projection(pattern, work)? {
            return Ok(());
        }
    }
    let mut names = variables::Names::new(work);
    // Reuse the cache's complete role analysis: authored bindings, projection
    // escapes and ambiguous definitions remain preserved, not guessed by name.
    walk::query(query, work, &mut |v, role| names.observe(v, role))?;
    if names.assign_parser()? {
        walk::query(query, work, &mut |v, _| names.rename(v))?;
    }
    Ok(())
}

pub(super) fn raw(query: &Query) -> Cow<'_, Query> {
    // Raw callers historically accept arbitrary ASTs. If the bounded optional
    // normalization is unavailable, preserve that behavior and original key.
    let Ok(envelope) = AlgebraEnvelopeV1::validate(query) else {
        return Cow::Borrowed(query);
    };
    normalize(
        query,
        &envelope,
        BuildWork::new(CompilerWorkMode::Uncontrolled),
    )
    .unwrap_or(Cow::Borrowed(query))
}

pub(super) fn controlled<'q>(
    query: &'q Query,
    envelope: &AlgebraEnvelopeV1,
    control: &dyn QueryControl,
) -> Result<Cow<'q, Query>> {
    normalize(
        query,
        envelope,
        BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
    )
}

fn normalize<'q>(
    query: &'q Query,
    envelope: &AlgebraEnvelopeV1,
    work: BuildWork<'_>,
) -> Result<Cow<'q, Query>> {
    if !envelope.cache_internal_binders {
        return Ok(Cow::Borrowed(query));
    }
    if let Query::Select { pattern, .. } = query {
        if !explicit_projection(pattern, work)? {
            // SELECT * can expose aggregate names on a caller-supplied AST.
            return Ok(Cow::Borrowed(query));
        }
    }
    if let CompilerWorkMode::Metered(cx) = work.mode {
        cx.reserve_ast_copy(CompilerCloneRootV1::Query(query))?;
    }
    let mut copy = query.clone();
    work.checkpoint()?;
    let mut names = variables::Names::new(work);
    walk::query(&mut copy, work, &mut |v, role| names.observe(v, role))?;
    if !names.assign()? {
        return Ok(Cow::Borrowed(query));
    }
    walk::query(&mut copy, work, &mut |v, _| names.rename(v))?;
    work.checkpoint()?;
    Ok(Cow::Owned(copy))
}

fn explicit_projection(mut pattern: &GraphPattern, work: BuildWork<'_>) -> Result<bool> {
    loop {
        work.charge(1)?;
        match pattern {
            GraphPattern::Project { variables, inner } => {
                if !variables.is_empty() {
                    return Ok(true);
                }
                pattern = inner;
            }
            GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. }
            | GraphPattern::OrderBy { inner, .. } => {
                pattern = inner;
            }
            _ => return Ok(false),
        }
    }
}

#[cfg(test)]
#[path = "cache_canonical/control_tests.rs"]
mod control_tests;
#[cfg(test)]
#[path = "cache_canonical/parse_tests.rs"]
mod parse_tests;
#[cfg(test)]
#[path = "cache_canonical/tests.rs"]
mod tests;

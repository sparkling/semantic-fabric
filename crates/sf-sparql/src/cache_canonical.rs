//! Cache-only alpha normalization, never a rewrite of the executable query.
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
#[path = "cache_canonical/tests.rs"]
mod tests;

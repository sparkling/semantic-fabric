//! Operation-local OPTIONAL admission, sharing the request's existing control.
//! These reservations cover candidate visits, output vectors and direct field
//! copies and measured-input unification, not FILTER work or physical heap bytes.

use crate::build::control::{BuildVec, BuildWork};
use crate::{iq::Branch, CompilerWorkMode, Result};

/// Reuse the compiler's measured-input admission and conservative logical
/// allowance without changing the raw unifier's decisions. This fixed span
/// encloses only this call; it contains no query, term or caller payload.
pub(crate) fn unify_terms(
    mode: CompilerWorkMode<'_>,
    left: &crate::iq::TermDef,
    right: &crate::iq::TermDef,
) -> Result<crate::unify::Unify> {
    match mode {
        CompilerWorkMode::Uncontrolled => Ok(crate::unify::unify(left, right)),
        CompilerWorkMode::Metered(cx) => {
            let _span = tracing::debug_span!("sf.compiler.optional_unification").entered();
            cx.unify_terms(left, right)
        }
    }
}

/// A fast OPTIONAL visits L candidates. Decomposition visits L*R matches and
/// L*R anti-matches, then transfers each original L into its no-match tail.
pub(crate) fn candidates(
    mode: CompilerWorkMode<'_>,
    left: usize,
    right: usize,
    decomposed: bool,
) -> Result<()> {
    if let CompilerWorkMode::Metered(cx) = mode {
        cx.checkpoint()?;
        cx.reserve_checked_product(&[left, right, if decomposed { 2 } else { 1 }])?;
        if decomposed {
            cx.reserve_checked_product(&[left])?;
        }
        cx.checkpoint()?;
    }
    Ok(())
}

/// The closure copies direct fields from this exact right source at most once.
/// Measurement and the copy reservation precede it; no shadow clone is made.
pub(crate) fn right_copy<T>(
    right: &Branch,
    mode: CompilerWorkMode<'_>,
    operation: impl FnOnce(&Branch) -> Result<T>,
) -> Result<T> {
    match mode {
        CompilerWorkMode::Uncontrolled => operation(right),
        CompilerWorkMode::Metered(cx) => cx.with_reserved_branch_copy(right, operation),
    }
}

/// Inner-match fields copy each side once. Anti-match copies the left bindings
/// only for a FILTER's combined scope, and the right conditions/scans once.
pub(crate) fn copies<T>(
    left: &Branch,
    right: &Branch,
    copies_left: bool,
    mode: CompilerWorkMode<'_>,
    operation: impl FnOnce(&Branch, &Branch) -> Result<T>,
) -> Result<T> {
    match mode {
        CompilerWorkMode::Metered(cx) if copies_left => cx
            .with_reserved_branch_copy(left, |left| {
                right_copy(right, mode, |right| operation(left, right))
            }),
        _ => right_copy(right, mode, |right| operation(left, right)),
    }
}

/// Preserve the branch's visible prefix between anti-join calls. Logical growth
/// is admitted independently of spare physical capacity on this owned vector.
pub(crate) fn push_owned<T>(work: BuildWork<'_>, values: &mut Vec<T>, value: T) -> Result<()> {
    let mut out = BuildVec::new(std::mem::take(values));
    work.push(&mut out, value)?;
    *values = out.into_inner();
    Ok(())
}

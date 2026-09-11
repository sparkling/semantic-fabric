//! One finite form-rewrite envelope, not a general RDF-star/compiler bound.
use sf_core::query_control::QueryControlError;
use spargebra::algebra::GraphPattern;
use spargebra::term::{TriplePattern, Variable};

use crate::build::control::BuildWork;
use crate::plan_measure::clone_root::CompilerCloneRootV1;
use crate::plan_measure::PlanMeasureV1;
use crate::{CompilerWorkMode, Error, Result};

pub(crate) fn rewrite_with_work_mode(
    pattern: &GraphPattern,
    mode: CompilerWorkMode<'_>,
) -> Result<(GraphPattern, Vec<TriplePattern>)> {
    let work = BuildWork::new(mode);
    let CompilerWorkMode::Metered(cx) = mode else {
        return super::rewrite(pattern);
    };
    // Shape admission is borrowed and constant-time. Preserve the original
    // Unsupported boundary before spending work discovering a subject.
    if !matches!(pattern, GraphPattern::Project { variables, .. } if variables.len() == 1) {
        return Err(Error::Unsupported(
            "DESCRIBE currently admits exactly one target".to_owned(),
        ));
    }
    let measured = cx.measure_ast_work(CompilerCloneRootV1::GraphPattern(pattern))?;
    let units = envelope(measured).ok_or_else(|| overflow(work))?;
    cx.reserve_checked_sum(&[units])?;
    work.checkpoint()?;
    // The borrowed source measured above is the exact source used once here.
    // Intermediate state is local and cannot escape a failed rewrite.
    let result = super::rewrite_inner(pattern, work);
    // Observe termination even on a semantic failure, but do not replace the
    // operation's first error with a later checkpoint (existing compiler law).
    let terminal = work.checkpoint();
    result.and_then(|value| terminal.map(|()| value))
}

/// D=N+S+P is the existing logical AST copy metric, not physical heap bytes.
/// Two D cover the initial copy and constant-target base/node copy; one D
/// covers scope discovery/comparisons; another D covers variable collection,
/// payload copies and its extra target insertion. Replacing the outer Project
/// by Distinct(Project) adds only one node and no new authored variable payload.
/// With at most N+1 variable insertions, each comparing at most N+1 names,
/// comparison work is bounded by (N+1)*(P+N+1). The extra N+1 pays logical
/// inventory slots. Fixed wrappers/vectors are prepaid below; the one output
/// triple clone and generated names are separately paid against actual inputs.
///
/// Input measurement enforces the existing depth/payload/node envelope and
/// checks each walk step. The finite, prepaid upstream clone/scope/collector
/// block has checkpoints around it, not in-call or allocator preemption.
fn envelope(m: PlanMeasureV1) -> Option<u64> {
    let n = m.nodes.checked_add(1)?;
    let comparisons = n.checked_mul(m.payload_bytes.checked_add(n)?)?;
    let fixed = 4 * std::mem::size_of::<GraphPattern>()
        + std::mem::size_of::<Variable>()
        + 2 * std::mem::size_of::<TriplePattern>()
        + 3;
    m.deep_clone_work
        .checked_mul(4)?
        .checked_add(comparisons)?
        .checked_add(n)?
        .checked_add(u64::try_from(fixed).ok()?)
}

/// Pay before formatting/copying one candidate and B-tree search/insert. An
/// ordered set of K names makes at most K comparisons, each examining at most
/// C candidate bytes. Two candidate payload copies and one slot are additional.
pub(super) fn reserve_fresh(
    work: BuildWork<'_>,
    names: usize,
    role_bytes: usize,
    ordinal: usize,
) -> Result<()> {
    let digits = if ordinal == 0 {
        1
    } else {
        ordinal.ilog10() as usize + 1
    };
    let bytes = "__sf_describe__"
        .len()
        .checked_add(role_bytes)
        .and_then(|v| v.checked_add(digits))
        .ok_or_else(|| overflow(work))?;
    let units = names
        .checked_mul(bytes.checked_add(1).ok_or_else(|| overflow(work))?)
        .and_then(|v| v.checked_add(bytes.checked_mul(2)?))
        .and_then(|v| v.checked_add(1))
        .ok_or_else(|| overflow(work))?;
    work.charge(units)
}

pub(super) fn overflow(work: BuildWork<'_>) -> Error {
    match work.mode {
        CompilerWorkMode::Metered(cx) => {
            cx.reject_build_resource(QueryControlError::AccountingOverflow)
        }
        CompilerWorkMode::Uncontrolled => {
            Error::Unsupported("DESCRIBE variable-name accounting overflow".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_envelope_uses_checked_arithmetic() {
        let m = PlanMeasureV1 {
            nodes: u64::MAX,
            collection_slots: 0,
            payload_bytes: 0,
            deep_clone_work: 0,
            max_depth: 0,
            max_pending_items: 0,
        };
        assert!(envelope(m).is_none());
        assert!(envelope(PlanMeasureV1 {
            nodes: 1,
            deep_clone_work: u64::MAX,
            ..m
        })
        .is_none());
        assert!(envelope(PlanMeasureV1 {
            nodes: 1,
            payload_bytes: u64::MAX,
            ..m
        })
        .is_none());
    }
}

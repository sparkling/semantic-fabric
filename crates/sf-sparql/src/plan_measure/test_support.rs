//! Downstream schedules distinguish traversal from the exact copied payload.
//! Walker mechanics have independent hand-counted tests in `control_tests`.

use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};

use super::clone_root::*;
use super::{PlanMeasureError, PlanMeasureV1};

pub(crate) struct CopyWork {
    pub(crate) deep_clone_work: u64,
    pub(crate) measurement_work: u64,
    pub(crate) total_work: u64,
}

fn budget() -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX))
}

fn schedule(raw: PlanMeasureV1, controlled: PlanMeasureV1, paid: &QueryBudget) -> CopyWork {
    assert_eq!(
        raw, controlled,
        "control must not change the V1 clone metric"
    );
    let measurement_work = paid.consumed(QueryCharge::CompilerWork);
    CopyWork {
        deep_clone_work: raw.deep_clone_work,
        measurement_work,
        total_work: measurement_work.checked_add(raw.deep_clone_work).unwrap(),
    }
}

pub(crate) fn measure_copy_root(
    root: CompilerCloneRootV1<'_>,
) -> Result<CopyWork, PlanMeasureError> {
    let paid = budget();
    let raw = measure_compiler_clone_root_v1(root)?;
    let controlled = measure_compiler_clone_root_with_control(root, &paid)?;
    Ok(schedule(raw, controlled, &paid))
}

pub(crate) fn measure_copy_collection(
    root: CompilerCloneCollectionV1<'_>,
) -> Result<CopyWork, PlanMeasureError> {
    let paid = budget();
    let raw = measure_compiler_clone_collection_v1(root)?;
    let controlled = measure_compiler_clone_collection_with_control(root, &paid)?;
    Ok(schedule(raw, controlled, &paid))
}

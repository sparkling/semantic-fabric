//! Legacy reference-atom D1 dispatch paid on the request's SourceWork.
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::source_work::SourceWork;
use sf_sql::Dialect;

use crate::build::control::BuildWork;
use crate::compiler_control::CompileContext;
use crate::iq::Branch;
use crate::{CompilerWorkMode, Result};

/// Same request identity: only the charge kind is renamed.
struct SourceCharged<'a>(&'a dyn QueryControl);

impl QueryControl for SourceCharged<'_> {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.0.checkpoint()
    }

    fn consume(
        &self,
        charge: QueryCharge,
        amount: u64,
    ) -> std::result::Result<(), QueryControlError> {
        let charge = match charge {
            QueryCharge::CompilerWork => QueryCharge::SourceWork,
            other => other,
        };
        self.0.consume(charge, amount)
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.0.terminate(reason)
    }

    fn capability(&self, type_id: std::any::TypeId) -> Option<&dyn std::any::Any> {
        self.0.capability(type_id)
    }
}

pub(super) fn force_distinct(
    branch: &mut Branch,
    dialect: Dialect,
    work: SourceWork<'_>,
) -> Result<()> {
    let Some(control) = work.control() else {
        crate::cascade::force_distinct_for_dup_safety(std::slice::from_mut(branch), &[], dialect);
        return Ok(());
    };
    let adapter = SourceCharged(control);
    let build = BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&adapter)));
    let schema = crate::cascade::build_resolve_schema(&[], build)?;
    crate::cascade::force_distinct_with_schema(
        std::slice::from_mut(branch),
        &schema,
        dialect,
        build,
    )
}

#[cfg(test)]
#[path = "ref_atom_work_tests.rs"]
mod tests;

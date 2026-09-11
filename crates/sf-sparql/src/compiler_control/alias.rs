//! Prospective alias reservations. Callers commit the counter only after their
//! last fallible preflight, so refusal cannot wrap or consume an identifier.
use std::ops::Range;

use sf_core::query_control::QueryControlError;

use crate::build::control::BuildWork;
use crate::{CompilerWorkMode, Result};

impl CompilerWorkMode<'_> {
    pub(crate) fn alias_range(self, start: usize, count: usize) -> Result<Range<usize>> {
        let work = BuildWork::new(self);
        work.checkpoint()?;
        let end = start.checked_add(count).ok_or_else(|| match self {
            Self::Metered(cx) => cx.reject_build_resource(QueryControlError::AccountingOverflow),
            Self::Uncontrolled => QueryControlError::AccountingOverflow.into(),
        })?;
        // Count actual prospective identifiers, not a whole helper/branch walk.
        // Arithmetic is checked first, so an impossible range spends no work.
        work.charge(count)?;
        Ok(start..end)
    }
}

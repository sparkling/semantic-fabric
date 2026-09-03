//! Request-owned compiler-work accounting and checked prospective-work arithmetic.

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

use crate::compile_envelope::CompileEnvelopeError;
use crate::{Error, Result};

/// A compiler-facing view of the request's shared query control.
///
/// This adapter deliberately owns no counter, clock, deadline, or cancellation
/// state. All observations and terminal causes flow through the supplied
/// request-scoped [`QueryControl`] identity.
#[derive(Clone, Copy)]
pub(crate) struct CompileMeter<'control> {
    control: &'control dyn QueryControl,
}

impl<'control> CompileMeter<'control> {
    pub(crate) const fn new(control: &'control dyn QueryControl) -> Self {
        Self { control }
    }

    pub(crate) fn checkpoint(&self) -> Result<()> {
        self.control.checkpoint().map_err(Error::from)
    }

    /// Charge compiler work that has already occurred.
    pub(crate) fn charge(&self, units: u64) -> Result<()> {
        self.control
            .consume(QueryCharge::CompilerWork, units)
            .map_err(Error::from)
    }

    pub(crate) fn checked_usize(&self, units: usize) -> Result<u64> {
        u64::try_from(units).map_err(|_| self.accounting_overflow())
    }

    pub(crate) fn checked_sum(&self, terms: &[u64]) -> Result<u64> {
        terms.iter().copied().try_fold(0_u64, |total, term| {
            total
                .checked_add(term)
                .ok_or_else(|| self.accounting_overflow())
        })
    }

    /// Compute prospective product work without mutating the request budget.
    pub(crate) fn checked_product(&self, factors: &[usize]) -> Result<u64> {
        if factors.is_empty() || factors.contains(&0) {
            return Ok(0);
        }

        factors.iter().copied().try_fold(1_u64, |total, factor| {
            let factor = self.checked_usize(factor)?;
            total
                .checked_mul(factor)
                .ok_or_else(|| self.accounting_overflow())
        })
    }

    /// Precharge a prospective product once after all arithmetic checks.
    pub(crate) fn precharge_product(&self, factors: &[usize]) -> Result<u64> {
        let units = self.checked_product(factors)?;
        self.charge(units)?;
        Ok(units)
    }

    pub(crate) fn reject_envelope(&self, _error: CompileEnvelopeError) -> Result<()> {
        Err(self
            .control
            .terminate(QueryControlError::CompilerEnvelopeExceeded)
            .into())
    }

    fn accounting_overflow(&self) -> Error {
        self.control
            .terminate(QueryControlError::AccountingOverflow)
            .into()
    }
}

#[cfg(test)]
#[path = "compiler_control/tests.rs"]
mod tests;

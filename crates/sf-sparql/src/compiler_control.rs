//! Request-owned compiler-work accounting and checked prospective-work arithmetic.

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

use crate::compile_envelope::CompileEnvelopeError;
use crate::plan_measure::PlanMeasureV1;
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

    /// Reserve work immediately before performing the corresponding bounded
    /// compiler operation.
    pub(crate) fn reserve_work(&self, units: u64) -> Result<()> {
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
        if factors.contains(&0) {
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
        self.reserve_work(units)?;
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

/// Shared compiler-phase access to one request's accounting identity.
///
/// This is a dormant coordination primitive: it does not admit a query,
/// activate a governed compile profile, or establish parser safety. Copies keep
/// the same [`CompileMeter`] and therefore the same sticky terminal state and
/// compiler-work counter.
#[derive(Clone, Copy)]
pub(crate) struct CompileContext<'control> {
    meter: CompileMeter<'control>,
}

impl<'control> CompileContext<'control> {
    pub(crate) const fn new(control: &'control dyn QueryControl) -> Self {
        Self {
            meter: CompileMeter::new(control),
        }
    }

    pub(crate) fn checkpoint(&self) -> Result<()> {
        self.meter.checkpoint()
    }

    /// Check and reserve the sum of deterministic work terms before the
    /// corresponding compiler operation.
    pub(crate) fn reserve_checked_sum(&self, terms: &[u64]) -> Result<u64> {
        let units = self.meter.checked_sum(terms)?;
        self.meter.reserve_work(units)?;
        Ok(units)
    }

    /// Check and reserve a deterministic product before constructing its
    /// candidate outputs.
    pub(crate) fn reserve_checked_product(&self, factors: &[usize]) -> Result<u64> {
        self.meter.precharge_product(factors)
    }

    /// Reserve the exact V1 prospective work of one measured deep clone.
    pub(crate) fn reserve_measured_clone(&self, measure: &PlanMeasureV1) -> Result<u64> {
        let units = measure.deep_clone_work;
        self.meter.reserve_work(units)?;
        Ok(units)
    }
}

#[cfg(test)]
#[path = "compiler_control/tests.rs"]
mod tests;

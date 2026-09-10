//! Measurement work is distinct from the payload subsequently cloned.
//!
//! Pay one entry, push, and metric-record unit; collection iteration prepays its
//! slots (including absent entries). Logical stack growth pays target plus
//! relocation bytes before fallible reserve. Payload length is read in O(1),
//! never scanned or charged as if measuring it copied the bytes a second time.

use sf_core::query_control::{QueryCharge, QueryControl};

use super::{Pending, PlanMeasureError, PlanMeasureLimits, Walker};

impl<'a> Walker<'a> {
    pub(super) fn controlled(
        limits: PlanMeasureLimits,
        control: &'a dyn QueryControl,
    ) -> Result<Self, PlanMeasureError> {
        let mut walker = Self::new(limits);
        walker.control = Some(control);
        walker.charge(1)?;
        Ok(walker)
    }

    pub(super) fn checkpoint(&self) -> Result<(), PlanMeasureError> {
        if let Some(control) = self.control {
            control.checkpoint()?;
        }
        Ok(())
    }

    pub(super) fn charge(&self, units: usize) -> Result<(), PlanMeasureError> {
        if let Some(control) = self.control {
            control.checkpoint()?;
            let units = u64::try_from(units).map_err(|_| PlanMeasureError::AccountingOverflow)?;
            control.consume(QueryCharge::CompilerWork, units)?;
            control.checkpoint()?;
        }
        Ok(())
    }

    pub(super) fn reserve_stack(&mut self) -> Result<(), PlanMeasureError> {
        if self.stack.len() < self.logical_capacity {
            return Ok(());
        }
        let target = self
            .logical_capacity
            .checked_mul(2)
            .ok_or(PlanMeasureError::AccountingOverflow)?
            .max(1)
            .min(self.limits.max_pending_items);
        let bytes = target
            .checked_add(self.stack.len())
            .and_then(|slots| slots.checked_mul(std::mem::size_of::<Pending<'_>>()))
            .ok_or(PlanMeasureError::AccountingOverflow)?;
        self.charge(bytes)?;
        self.stack.try_reserve_exact(target - self.stack.len())?;
        self.logical_capacity = target;
        Ok(())
    }
}

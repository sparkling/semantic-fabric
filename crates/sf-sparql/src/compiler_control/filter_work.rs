//! Admission for raw FILTER construction only; source validation is separately
//! visited and charged. Each expression node makes a fixed number of binding
//! lookups/helper passes and copies at most its operands' mapped payload. The
//! expression × bindings product conservatively covers repeated lookups, typed
//! comparisons, the existing 64-pass unifier, Debug escaping and LIKE expansion.
//! 128 passes plus 1024 fixed logical units are not a physical heap bound.
//! Changes to filter_cond or its helpers must preserve this bound or update it.
//! Measurement enforces depth/size limits first; the prepaid raw call has no
//! internal cancellation checkpoints and retains its existing error order.
use super::CompileContext;
use crate::iq::{SqlCond, TermDef};
use crate::plan_measure::clone_root::{CompilerCloneCollectionV1, CompilerCloneRootV1};
use crate::{Error, Result};
use spargebra::algebra::Expression;
use std::collections::BTreeMap;

impl CompileContext<'_> {
    pub(crate) fn filter_condition(
        &self,
        expression: &Expression,
        bindings: &BTreeMap<String, TermDef>,
        dialect: sf_sql::Dialect,
    ) -> Result<SqlCond> {
        self.checkpoint()?;
        let expression_work = self
            .measure_root(CompilerCloneRootV1::Expression(expression))
            .map_err(|e| self.measurement_error(e))?;
        let binding_work = self
            .measure_collection(CompilerCloneCollectionV1::Bindings(bindings))
            .map_err(|e| self.measurement_error(e))?;
        self.reserve_filter_work(
            expression_work.deep_clone_work,
            binding_work.deep_clone_work,
        )?;
        self.checkpoint()?;
        let result = crate::unify::filter_cond(expression, bindings, dialect);
        self.checkpoint()?;
        result.map_err(Error::Unsupported)
    }

    pub(super) fn reserve_filter_work(&self, expression: u64, bindings: u64) -> Result<()> {
        let expression = self.meter.checked_sum(&[expression, 1])?;
        let bindings = self.meter.checked_sum(&[bindings, 1])?;
        let passes = expression
            .checked_mul(bindings)
            .and_then(|n| n.checked_mul(128))
            .ok_or_else(|| self.meter.accounting_overflow())?;
        self.reserve_checked_sum(&[1024, passes])?;
        self.checkpoint()
    }
}

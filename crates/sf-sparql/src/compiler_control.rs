//! Request-owned compiler-work accounting and checked prospective-work arithmetic.

use std::collections::BTreeMap;

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

use crate::compile_envelope::CompileEnvelopeError;
use crate::iq::node::{BindDef, IqCond, IqNode, Var};
use crate::iq::Branch;
use crate::plan_measure::clone_root::{
    measure_compiler_clone_collection_v1, measure_compiler_clone_root_v1,
    CompilerCloneCollectionV1, CompilerCloneRootV1,
};
use crate::plan_measure::{PlanMeasureError, PlanMeasureV1};
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
/// Serving uses this view for selected owned compiler operations. It does not
/// admit a query, activate a governed profile, or establish parser safety. Copies keep
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

    /// Measure, reserve, and perform exactly one recursive branch-forest clone.
    ///
    /// Keeping all three operations behind one method prevents a caller from
    /// charging one graph and cloning another, charging once and cloning twice,
    /// or reserving after the allocation has already happened.
    pub(crate) fn clone_branch_forest(&self, branches: &[Branch]) -> Result<Vec<Branch>> {
        self.checkpoint()?;
        let measure =
            measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Branches(branches))
                .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(branches.to_vec())
    }

    /// Measure, reserve, and perform one scalar branch clone, without charging
    /// a synthetic collection slot.
    pub(crate) fn clone_branch(&self, branch: &Branch) -> Result<Branch> {
        self.checkpoint()?;
        let measure = measure_compiler_clone_root_v1(CompilerCloneRootV1::Branch(branch))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(branch.clone())
    }

    /// Measure, reserve, and perform exactly one recursive IQ-condition clone.
    ///
    /// The source slice remains bound to its exact measurement and the one clone,
    /// so normalization cannot charge a different condition forest or reuse one
    /// reservation for multiple Union arms.
    pub(crate) fn clone_iq_conditions(&self, conditions: &[IqCond]) -> Result<Vec<IqCond>> {
        self.checkpoint()?;
        let measure = measure_compiler_clone_collection_v1(
            CompilerCloneCollectionV1::IqConditions(conditions),
        )
        .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(conditions.to_vec())
    }

    /// Measure, reserve, and perform exactly one recursive IQ-node collection clone.
    pub(crate) fn clone_iq_nodes(&self, nodes: &[IqNode]) -> Result<Vec<IqNode>> {
        self.checkpoint()?;
        let measure =
            measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::IqNodes(nodes))
                .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(nodes.to_vec())
    }

    /// Measure, reserve, and perform exactly one recursive scalar IQ-node clone.
    ///
    /// This deliberately uses the scalar root: wrapping `node` in a synthetic
    /// one-element collection would charge work that the actual clone never does.
    pub(crate) fn clone_iq_node(&self, node: &IqNode) -> Result<IqNode> {
        self.checkpoint()?;
        let measure = measure_compiler_clone_root_v1(CompilerCloneRootV1::IqNode(node))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(node.clone())
    }

    /// Measure, reserve, and perform exactly one recursive IQ-substitution clone.
    pub(crate) fn clone_iq_substitution(
        &self,
        substitution: &BTreeMap<Var, BindDef>,
    ) -> Result<BTreeMap<Var, BindDef>> {
        self.checkpoint()?;
        let measure = measure_compiler_clone_collection_v1(
            CompilerCloneCollectionV1::IqSubstitution(substitution),
        )
        .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(substitution.clone())
    }

    /// Measure, reserve, and perform exactly one IQ-variable collection clone.
    pub(crate) fn clone_variables(&self, variables: &[Var]) -> Result<Vec<Var>> {
        self.checkpoint()?;
        let measure =
            measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Variables(variables))
                .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(variables.to_vec())
    }

    fn reserve_measured_clone(&self, measure: &PlanMeasureV1) -> Result<u64> {
        let units = measure.deep_clone_work;
        self.meter.reserve_work(units)?;
        Ok(units)
    }

    fn measurement_error(&self, error: PlanMeasureError) -> Error {
        match error {
            PlanMeasureError::AccountingOverflow => self.meter.accounting_overflow(),
            PlanMeasureError::LimitExceeded { .. } => self
                .meter
                .control
                .terminate(QueryControlError::CompilerEnvelopeExceeded)
                .into(),
            PlanMeasureError::AllocationFailed => self
                .meter
                .control
                .terminate(QueryControlError::CompilerResourceExhausted)
                .into(),
        }
    }
}

#[cfg(test)]
#[path = "compiler_control/tests.rs"]
mod tests;

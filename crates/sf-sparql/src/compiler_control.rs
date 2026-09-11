//! Request-owned compiler-work accounting and checked prospective-work arithmetic.

use std::collections::BTreeMap;

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

use crate::compile_envelope::CompileEnvelopeError;
use crate::iq::node::{BindDef, IqCond, IqNode, Var};
use crate::iq::Branch;
use crate::plan_measure::clone_root::{
    measure_compiler_clone_collection_with_control, measure_compiler_clone_root_with_control,
    CompilerCloneCollectionV1, CompilerCloneRootV1,
};
use crate::plan_measure::{PlanMeasureError, PlanMeasureV1};
use crate::{Error, Result};

mod optional_work;
mod unification_work;

#[cfg(test)]
mod unification_work_tests;

#[cfg(test)]
pub(crate) mod normalization_test_support;

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
        let measure = self
            .measure_collection(CompilerCloneCollectionV1::Branches(branches))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(branches.to_vec())
    }

    /// Measure, reserve, and perform one scalar branch clone, without charging
    /// a synthetic collection slot.
    pub(crate) fn clone_branch(&self, branch: &Branch) -> Result<Branch> {
        self.checkpoint()?;
        let measure = self
            .measure_root(CompilerCloneRootV1::Branch(branch))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(branch.clone())
    }

    /// Reserve one conservative whole-branch copy before the inner join copies
    /// direct fields from this same borrowed source. The caller must copy each
    /// field at most once; this does not account for unification or temporary
    /// allocations inside the merge, and creates no shadow branch clone.
    pub(crate) fn with_reserved_branch_copy<T>(
        &self,
        source: &Branch,
        operation: impl FnOnce(&Branch) -> Result<T>,
    ) -> Result<T> {
        self.checkpoint()?;
        let measure = self
            .measure_root(CompilerCloneRootV1::Branch(source))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        self.checkpoint()?;
        let result = operation(source);
        self.checkpoint()?;
        result
    }

    /// Bind the measured source payload to the actual one owned scan copy.
    pub(crate) fn clone_logical_source(
        &self,
        source: &sf_core::ir::LogicalSource,
    ) -> Result<sf_core::ir::LogicalSource> {
        self.checkpoint()?;
        let measure = self
            .measure_root(CompilerCloneRootV1::LogicalSource(source))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(source.clone())
    }

    /// Reserve the scalar term-map payload immediately before its one copy.
    pub(crate) fn clone_term_map(
        &self,
        map: &sf_core::ir::TermMap,
    ) -> Result<sf_core::ir::TermMap> {
        self.checkpoint()?;
        let measure = self
            .measure_root(CompilerCloneRootV1::TermMap(map))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(map.clone())
    }

    /// Measure, reserve, and perform exactly one recursive IQ-condition clone.
    ///
    /// The source slice remains bound to its exact measurement and the one clone,
    /// so normalization cannot charge a different condition forest or reuse one
    /// reservation for multiple Union arms.
    pub(crate) fn clone_iq_conditions(&self, conditions: &[IqCond]) -> Result<Vec<IqCond>> {
        self.checkpoint()?;
        let measure = self
            .measure_collection(CompilerCloneCollectionV1::IqConditions(conditions))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(conditions.to_vec())
    }

    /// Measure, reserve, and perform exactly one recursive IQ-node collection clone.
    pub(crate) fn clone_iq_nodes(&self, nodes: &[IqNode]) -> Result<Vec<IqNode>> {
        self.checkpoint()?;
        let measure = self
            .measure_collection(CompilerCloneCollectionV1::IqNodes(nodes))
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
        let measure = self
            .measure_root(CompilerCloneRootV1::IqNode(node))
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
        let measure = self
            .measure_collection(CompilerCloneCollectionV1::IqSubstitution(substitution))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(substitution.clone())
    }

    /// Measure, reserve, and perform exactly one IQ-variable collection clone.
    pub(crate) fn clone_variables(&self, variables: &[Var]) -> Result<Vec<Var>> {
        self.checkpoint()?;
        let measure = self
            .measure_collection(CompilerCloneCollectionV1::Variables(variables))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        Ok(variables.to_vec())
    }

    /// Reserve an actual AST payload copy before BUILD performs it. The typed
    /// BUILD adapter binds this root to its borrowed source and single clone.
    pub(crate) fn reserve_ast_copy(&self, root: CompilerCloneRootV1<'_>) -> Result<()> {
        self.checkpoint()?;
        let measure = self
            .measure_root(root)
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measure)?;
        self.checkpoint()
    }

    pub(crate) fn reject_build_resource(&self, reason: QueryControlError) -> Error {
        self.meter.control.terminate(reason).into()
    }

    /// Prepay a constant RDF-term comparison using both exact borrowed carriers.
    /// Measurement pays its own traversal; node/payload units bound the subsequent
    /// structural equality scan, not another clone or a physical heap estimate.
    pub(crate) fn constant_terms_equal(
        &self,
        left: &crate::iq::TermDef,
        right: &crate::iq::TermDef,
    ) -> Result<bool> {
        let (crate::iq::TermDef::Const(a), crate::iq::TermDef::Const(b)) = (left, right) else {
            return Ok(false);
        };
        self.checkpoint()?;
        let left = self
            .measure_root(CompilerCloneRootV1::TermDef(left))
            .map_err(|error| self.measurement_error(error))?;
        let right = self
            .measure_root(CompilerCloneRootV1::TermDef(right))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_checked_sum(&[left.deep_clone_work, right.deep_clone_work])?;
        self.checkpoint()?;
        let equal = a == b;
        self.checkpoint()?;
        Ok(equal)
    }

    fn measure_root(
        &self,
        root: CompilerCloneRootV1<'_>,
    ) -> std::result::Result<PlanMeasureV1, PlanMeasureError> {
        measure_compiler_clone_root_with_control(root, self.meter.control)
    }

    fn measure_collection(
        &self,
        root: CompilerCloneCollectionV1<'_>,
    ) -> std::result::Result<PlanMeasureV1, PlanMeasureError> {
        measure_compiler_clone_collection_with_control(root, self.meter.control)
    }

    fn reserve_measured_clone(&self, measure: &PlanMeasureV1) -> Result<u64> {
        let units = measure.deep_clone_work;
        self.meter.reserve_work(units)?;
        Ok(units)
    }

    fn measurement_error(&self, error: PlanMeasureError) -> Error {
        match error {
            PlanMeasureError::Control(cause) => self.meter.control.terminate(cause).into(),
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

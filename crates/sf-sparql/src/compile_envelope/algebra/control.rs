//! Request-controlled preparation before upstream recursive canonical Display.

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

use super::{AlgebraEnvelopeV1, CompileEnvelopeError, Validator};

#[derive(Debug)]
pub(super) enum WalkError {
    Envelope(CompileEnvelopeError),
    Control(QueryControlError),
}

impl From<CompileEnvelopeError> for WalkError {
    fn from(error: CompileEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<QueryControlError> for WalkError {
    fn from(error: QueryControlError) -> Self {
        Self::Control(error)
    }
}

pub(super) fn charge(
    control: Option<&dyn QueryControl>,
    units: usize,
) -> Result<(), QueryControlError> {
    let Some(control) = control else {
        return Ok(());
    };
    control.checkpoint()?;
    let units = u64::try_from(units)
        .map_err(|_| control.terminate(QueryControlError::AccountingOverflow))?;
    control.consume(QueryCharge::CompilerWork, units)?;
    control.checkpoint()
}

impl AlgebraEnvelopeV1 {
    /// Same structural envelope as `validate`, but work, collection iterations
    /// and logical stack target/relocation bytes are paid before use.
    pub(crate) fn validate_with_control(
        query: &spargebra::Query,
        control: &dyn QueryControl,
    ) -> Result<Self, QueryControlError> {
        Validator::run(query, Some(control)).map_err(|error| match error {
            WalkError::Control(cause) => cause,
            WalkError::Envelope(error) => control.terminate(match error {
                CompileEnvelopeError::AccountingOverflow => QueryControlError::AccountingOverflow,
                CompileEnvelopeError::AllocationFailed => {
                    QueryControlError::CompilerResourceExhausted
                }
                CompileEnvelopeError::LimitExceeded { .. } => {
                    QueryControlError::CompilerEnvelopeExceeded
                }
            }),
        })
    }

    /// spargebra 0.4.6's SparqlGraphRootPattern::fmt scans root modifiers,
    /// collects projection references, and folds Extends before its first write.
    /// Each Extend can scan the projection three times and walk all previously
    /// attached expressions, including EXISTS graphs. Those expressions occupy
    /// disjoint AST subtrees. N+S bounds one such walk; M+1 bounds each variable
    /// comparison. Across all roots, E and Q count actual Extends and projection
    /// slots, not arbitrary payload bytes. Empty projects/root reentry fit N.
    ///
    /// This deliberately conservative logical work charge does not replace the
    /// upstream infallible projection Vec or promise exact allocator capacity.
    /// Cancellation is checked around this finite precharge and writer calls,
    /// not inside the upstream scans. No raw canonical bytes are changed.
    pub(crate) fn charge_canonical_preparation(
        &self,
        control: &dyn QueryControl,
    ) -> Result<(), QueryControlError> {
        let units = (|| {
            let comparisons = self
                .project_slots
                .checked_mul(3)?
                .checked_add(self.algebra_nodes)?
                .checked_add(self.collection_slots)?;
            let scans = comparisons
                .checked_mul(self.extend_nodes)?
                .checked_mul(self.max_variable_bytes.checked_add(1)?)?;
            let projection = self.project_slots.checked_mul(std::mem::size_of::<(
                &spargebra::term::Variable,
                Option<&spargebra::algebra::Expression>,
            )>())?;
            self.algebra_nodes
                .checked_add(scans)?
                .checked_add(projection)
        })()
        .ok_or_else(|| control.terminate(QueryControlError::AccountingOverflow))?;
        charge(Some(control), units)
    }
}

//! Prospective logical work for per-row RDF term reconstruction.
//!
//! The mirror of `sf_sql::source_work::SourceWork` for the term-generation side
//! of a request: the same prepay-then-do discipline, expressed against
//! [`QueryControl`] alone so `sf-core` keeps its place at the bottom of the
//! crate graph (it cannot name `sf-sql`).
//!
//! Row *decoding* — turning driver cells into strings — is charged by the
//! backends under `SourceWork`. This type charges what happens *after* that:
//! building each solution's bound terms (templates, percent-encoded IRIs,
//! natural-datatype lexical forms). Both charge the same
//! [`QueryCharge::SourceWork`] counter, because both are per-source-row work
//! admitted by one request's source budget; the split is where the work runs,
//! not which limit governs it.
//!
//! Allocator overhead is not measured here: a charge bounds the logical output
//! a step may produce, not a native allocator's own bookkeeping.

use crate::query_control::{QueryCharge, QueryControl, QueryControlError};

/// A request's term-generation charging identity, or `None` for the explicit
/// uncontrolled path (raw/diagnostic APIs, mapping-side callers that are not
/// executing a governed request).
#[derive(Clone, Copy, Default)]
pub struct TermWork<'a>(Option<&'a dyn QueryControl>);

impl<'a> TermWork<'a> {
    /// `None` preserves the existing uncontrolled behavior exactly.
    pub const fn new(control: Option<&'a dyn QueryControl>) -> Self {
        Self(control)
    }

    /// The uncontrolled identity: every charge and checkpoint succeeds.
    pub const fn uncontrolled() -> Self {
        Self(None)
    }

    pub const fn control(self) -> Option<&'a dyn QueryControl> {
        self.0
    }

    /// Observe a sticky terminal cause (cancellation, deadline) without
    /// charging. The in-batch stop point: reconstruction calls this per row, so
    /// a terminated request stops inside a batch instead of only at the next
    /// batch boundary.
    pub fn checkpoint(self) -> Result<(), QueryControlError> {
        match self.0 {
            Some(control) => control.checkpoint(),
            None => Ok(()),
        }
    }

    /// Charge `units` of prospective work, checkpointing on both sides so a
    /// request that became terminal mid-row is observed before and after the
    /// counter moves.
    pub fn charge(self, units: usize) -> Result<(), QueryControlError> {
        let Some(control) = self.0 else {
            return Ok(());
        };
        control.checkpoint()?;
        let units = u64::try_from(units)
            .map_err(|_| control.terminate(QueryControlError::AccountingOverflow))?;
        control.consume(QueryCharge::SourceWork, units)?;
        control.checkpoint()
    }

    /// Charge `count * width`, terminating on overflow rather than wrapping to
    /// a smaller charge than the work actually costs.
    pub fn product(self, count: usize, width: usize) -> Result<(), QueryControlError> {
        let Some(control) = self.0 else {
            return Ok(());
        };
        let units = count
            .checked_mul(width)
            .ok_or_else(|| control.terminate(QueryControlError::AccountingOverflow))?;
        self.charge(units)
    }
}

#[cfg(test)]
#[path = "term_work/tests.rs"]
mod tests;

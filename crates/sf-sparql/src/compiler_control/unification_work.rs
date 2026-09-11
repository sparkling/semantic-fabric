//! Admission around the existing unifier, not an alternative unification law.
//!
//! Its call graph is linear in the two measured inputs: R2rmlBlank adds its
//! body and graph (not a product); plain_term_def copies each map once; template
//! split/prefix/alignment make a bounded number of scans; operand construction
//! emits one part per input segment; condition concatenation is additive. A
//! deliberately conservative 64 passes over carrier/slot/payload units plus
//! 512 fixed units covers those scans, copies, output logical units and fixed
//! messages. It is NOT a size_of/physical-allocation bound. Changes to unify or
//! its template/identity helpers must preserve this call-graph bound or update
//! this reservation. Raw API semantics and verdict order stay untouched.
//!
//! Both exact borrowed inputs first undergo the existing paid measurement and
//! its independent depth/size limits. The reservation then precedes raw unify.
//! There is no cancellation checkpoint *inside* that prepaid call; infallible
//! allocation/allocator overgrant and destruction require separate controls.
use super::CompileContext;
use crate::iq::TermDef;
use crate::plan_measure::clone_root::CompilerCloneRootV1;
use crate::unify::{unify, Unify};
use crate::Result;

impl CompileContext<'_> {
    pub(crate) fn unify_terms(&self, left: &TermDef, right: &TermDef) -> Result<Unify> {
        self.checkpoint()?;
        let a = self
            .measure_root(CompilerCloneRootV1::TermDef(left))
            .map_err(|e| self.measurement_error(e))?;
        let b = self
            .measure_root(CompilerCloneRootV1::TermDef(right))
            .map_err(|e| self.measurement_error(e))?;
        self.reserve_unification_work(a.deep_clone_work, b.deep_clone_work)?;
        self.checkpoint()?;
        let result = unify(left, right);
        self.checkpoint()?;
        Ok(result)
    }

    pub(super) fn reserve_unification_work(&self, left: u64, right: u64) -> Result<()> {
        let input = self.meter.checked_sum(&[left, right])?;
        let passes = input
            .checked_mul(64)
            .ok_or_else(|| self.meter.accounting_overflow())?;
        self.reserve_checked_sum(&[512, passes])?;
        self.checkpoint()
    }
}

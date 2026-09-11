//! Exact scalar copies introduced by nullable SubPlan OPTIONAL bindings.
use super::*;
use crate::iq::TermDef;

impl CompileContext<'_> {
    pub(crate) fn clone_optional_term_def(&self, source: &TermDef) -> Result<TermDef> {
        self.checkpoint()?;
        let measured = self
            .measure_root(CompilerCloneRootV1::TermDef(source))
            .map_err(|error| self.measurement_error(error))?;
        self.reserve_measured_clone(&measured)?;
        self.checkpoint()?;
        let copied = source.clone();
        self.checkpoint()?;
        Ok(copied)
    }
}

//! Internally controlled counterpart of the small raw BIND helper. Supported
//! arms charge actual visits, copies, ordered lookups and CONCAT slots. Only
//! unsupported Debug formatting remains one measured, prepaid raw call.
use super::CompileContext;
use crate::build::control::BuildWork;
use crate::iq::TermDef;
use crate::plan_measure::clone_root::CompilerCloneRootV1;
use crate::{CompilerWorkMode, Result};
use spargebra::algebra::{Expression, Function};
use std::collections::BTreeMap;

impl CompileContext<'_> {
    // Inner errors are the unchanged raw unsupported/unbound strings; the fold
    // retries these. Work/depth/cancellation errors must never be retried.
    pub(crate) fn bind_definition(
        &self,
        expression: &Expression,
        bindings: &BTreeMap<String, TermDef>,
    ) -> Result<std::result::Result<TermDef, String>> {
        let _span = tracing::debug_span!("sf.compiler.substitution_expression").entered();
        self.bind_node(
            expression,
            bindings,
            BuildWork::new(CompilerWorkMode::Metered(*self)),
        )
    }

    fn bind_node(
        &self,
        expression: &Expression,
        bindings: &BTreeMap<String, TermDef>,
        work: BuildWork<'_>,
    ) -> Result<std::result::Result<TermDef, String>> {
        let work = work.enter()?;
        let value = match expression {
            Expression::NamedNode(n) => {
                Ok(TermDef::Const(sf_core::Term::NamedNode(work.copied(n)?)))
            }
            Expression::Literal(l) => Ok(TermDef::Const(sf_core::Term::Literal(work.copied(l)?))),
            Expression::Variable(variable) => {
                let name = variable.as_str();
                let mut found = None;
                for (key, value) in bindings {
                    work.charge(1)?;
                    work.charge(key.len().min(name.len()))?;
                    match key.as_str().cmp(name) {
                        std::cmp::Ordering::Equal => {
                            found = Some(value);
                            break;
                        }
                        std::cmp::Ordering::Greater => break,
                        std::cmp::Ordering::Less => (),
                    }
                }
                match found {
                    Some(value) => Ok(self.clone_optional_term_def(value)?),
                    None => {
                        work.charge(1)?;
                        work.charge("BIND references unbound ?".len())?;
                        work.charge(name.len())?;
                        Err(format!("BIND references unbound ?{name}"))
                    }
                }
            }
            Expression::FunctionCall(Function::Concat, args) => {
                let mut parts = work.vector(args.len())?;
                for arg in args {
                    match self.bind_node(arg, bindings, work)? {
                        Ok(value) => parts.push(value),
                        Err(why) => return Ok(Err(why)),
                    }
                }
                Ok(TermDef::Concat(parts))
            }
            _ => {
                // Debug visits the complete unsupported expression, not bindings.
                // Sixteen logical passes cover UTF-8/debug escaping, enum labels
                // and delimiters; 256 fixed units cover the error prefix. This is
                // not a physical heap bound or preemption inside formatting.
                let measured = self
                    .measure_root(CompilerCloneRootV1::Expression(expression))
                    .map_err(|e| self.measurement_error(e))?;
                self.reserve_bind_error(measured.deep_clone_work)?;
                self.checkpoint()?;
                crate::unify::bind_term_def(expression, bindings)
            }
        };
        work.checkpoint()?;
        Ok(value)
    }

    pub(super) fn reserve_bind_error(&self, expression: u64) -> Result<()> {
        let units = expression
            .checked_mul(16)
            .ok_or_else(|| self.meter.accounting_overflow())?;
        self.reserve_checked_sum(&[256, units])?;
        self.checkpoint()
    }
}

//! Admission for raw FILTER construction only; source validation is separately
//! visited and charged. Each expression node makes a fixed number of binding
//! lookups/helper passes and copies at most its operands' mapped payload. The
//! expression × lookup-footprint product covers repeated lookups, typed
//! comparisons, the existing 64-pass unifier, Debug escaping and LIKE expansion.
//! 128 passes plus 1024 fixed logical units are not a physical heap bound.
//! Changes to filter_cond or its helpers must preserve this bound or update it.
//! Measurement enforces depth/size limits first; the prepaid raw call has no
//! internal cancellation checkpoints and retains its existing error order.
use super::CompileContext;
use crate::build::control::BuildWork;
use crate::iq::{SqlCond, TermDef};
use crate::plan_measure::clone_root::CompilerCloneRootV1;
use crate::{CompilerWorkMode, Error, Result};
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
        let binding_work = self.filter_binding_work(
            expression,
            bindings,
            BuildWork::new(CompilerWorkMode::Metered(*self)),
        )?;
        self.reserve_filter_work(expression_work.deep_clone_work, binding_work)?;
        self.checkpoint()?;
        let result = crate::unify::filter_cond(expression, bindings, dialect);
        self.checkpoint()?;
        result.map_err(Error::Unsupported)
    }

    /// Raw FILTER helpers use only variable get/contains_key lookups, never
    /// scan unrelated term definitions. Pay all key comparisons (a conservative
    /// upper bound on BTreeMap lookup), but measure only matched payloads.
    /// Repeated occurrences are counted separately, without a dedup allocation.
    fn filter_binding_work(
        &self,
        expression: &Expression,
        bindings: &BTreeMap<String, TermDef>,
        work: BuildWork<'_>,
    ) -> Result<u64> {
        let work = work.enter()?;
        let mut total = 0;
        match expression {
            Expression::Variable(variable) | Expression::Bound(variable) => {
                let name = variable.as_str();
                let mut matched = None;
                for (key, value) in bindings {
                    work.charge(1)?;
                    let bytes = key.len().min(name.len());
                    work.charge(bytes)?;
                    total = self.meter.checked_sum(&[total, 1, bytes as u64])?;
                    if key == name {
                        matched = Some(value);
                    }
                }
                if let Some(value) = matched {
                    let measured = self
                        .measure_root(CompilerCloneRootV1::TermDef(value))
                        .map_err(|e| self.measurement_error(e))?;
                    total = self.meter.checked_sum(&[total, measured.deep_clone_work])?;
                }
            }
            Expression::Or(a, b)
            | Expression::And(a, b)
            | Expression::Equal(a, b)
            | Expression::SameTerm(a, b)
            | Expression::Greater(a, b)
            | Expression::GreaterOrEqual(a, b)
            | Expression::Less(a, b)
            | Expression::LessOrEqual(a, b)
            | Expression::Add(a, b)
            | Expression::Subtract(a, b)
            | Expression::Multiply(a, b)
            | Expression::Divide(a, b) => {
                total = self.filter_binding_work(a, bindings, work)?;
                total = self
                    .meter
                    .checked_sum(&[total, self.filter_binding_work(b, bindings, work)?])?;
            }
            Expression::In(first, rest) => {
                total = self.filter_binding_work(first, bindings, work)?;
                for child in rest {
                    total = self
                        .meter
                        .checked_sum(&[total, self.filter_binding_work(child, bindings, work)?])?;
                }
            }
            Expression::UnaryPlus(child)
            | Expression::UnaryMinus(child)
            | Expression::Not(child) => {
                total = self.filter_binding_work(child, bindings, work)?;
            }
            Expression::If(condition, yes, no) => {
                for child in [condition, yes, no] {
                    total = self
                        .meter
                        .checked_sum(&[total, self.filter_binding_work(child, bindings, work)?])?;
                }
            }
            Expression::Coalesce(children) | Expression::FunctionCall(_, children) => {
                for child in children {
                    total = self
                        .meter
                        .checked_sum(&[total, self.filter_binding_work(child, bindings, work)?])?;
                }
            }
            // The raw helper rejects EXISTS; IQ Exists is delegated separately.
            // Expression measurement already pays all constant/error payload.
            Expression::NamedNode(_) | Expression::Literal(_) | Expression::Exists(_) => (),
        }
        work.checkpoint()?;
        Ok(total)
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

//! Logical admission for each actual rewrite invocation, not an input-only
//! polynomial bound: FILTER/UNION/EXISTS may invoke the same source repeatedly.
use std::collections::BTreeSet;

use sf_core::query_control::{QueryControl, QueryControlError};
use spargebra::algebra::GraphPattern;
use spargebra::term::Variable;
use spargebra::Query;

use crate::build::control::BuildWork;
use crate::compiler_control::CompileContext;
use crate::plan_measure::clone_root::CompilerCloneRootV1;
use crate::{CompilerWorkMode, Error, Result};

use super::env::{ComposedInfo, StarEnv};
use super::util::FreshVars;

/// Rewrite with the caller's existing request identity. Raw translation keeps
/// the uncontrolled entry; this function does not admit or parse a query.
pub fn rewrite_query_with_work_control(
    query: &Query,
    control: &dyn QueryControl,
) -> Result<(Query, StarEnv)> {
    rewrite_query_with_work_mode(
        query,
        CompilerWorkMode::Metered(CompileContext::new(control)),
    )
}

pub(crate) fn rewrite_query_with_work_mode(
    query: &Query,
    mode: CompilerWorkMode<'_>,
) -> Result<(Query, StarEnv)> {
    let work = StarWork::new(mode);
    // Measurement enforces the current input depth/node/payload envelope before
    // the recursive collector or any Query header/template copy can execute.
    work.inventory(CompilerCloneRootV1::Query(query))?;
    let mut names = FreshVars::with_work(super::collect_vars::collect_query_vars(query), work);
    work.local(CompilerCloneRootV1::Query(query))?;
    let result = super::top_level::rewrite_query_inner(query, &mut names);
    let terminal = work.checkpoint();
    result.and_then(|value| terminal.map(|()| value))
}

#[derive(Clone, Copy)]
pub(super) struct StarWork<'a>(pub BuildWork<'a>);

impl<'a> StarWork<'a> {
    pub fn new(mode: CompilerWorkMode<'a>) -> Self {
        Self(BuildWork::new(mode))
    }

    pub fn checkpoint(self) -> Result<()> {
        self.0.checkpoint()
    }

    pub fn charge(self, units: usize) -> Result<()> {
        self.0.charge(units)
    }

    pub fn product(self, factors: &[usize]) -> Result<()> {
        if let CompilerWorkMode::Metered(cx) = self.0.mode {
            cx.checkpoint()?;
            cx.reserve_checked_product(factors)?;
            cx.checkpoint()?;
        }
        Ok(())
    }

    /// A local copy envelope pays the ordinary carrier/slots/payload of this
    /// exact input. Children pay on *every* invocation; generated output and
    /// environment operations are additional. D is the existing logical clone
    /// metric, not physical heap bytes or an in-call CPU preemption claim.
    pub fn local(self, root: CompilerCloneRootV1<'_>) -> Result<()> {
        self.charge(1)?;
        if let CompilerWorkMode::Metered(cx) = self.0.mode {
            cx.reserve_ast_copy(root)?;
        }
        Ok(())
    }

    /// At most N variable inserts; each compares <=N names of total payload P.
    /// The D term pays traversal and copied payload, N*(P+N) comparisons and
    /// N logical slots. This block never covers recursive rewrite amplification.
    pub fn inventory(self, root: CompilerCloneRootV1<'_>) -> Result<()> {
        if let CompilerWorkMode::Metered(cx) = self.0.mode {
            let m = cx.measure_ast_work(root)?;
            let units = m
                .payload_bytes
                .checked_add(m.nodes)
                .and_then(|p| m.nodes.checked_mul(p))
                .and_then(|v| v.checked_add(m.deep_clone_work))
                .and_then(|v| v.checked_add(m.nodes))
                .ok_or_else(|| self.overflow())?;
            cx.reserve_checked_sum(&[units])?;
            cx.checkpoint()?;
        }
        Ok(())
    }

    pub fn pattern_vars(self, pattern: &GraphPattern) -> Result<BTreeSet<Variable>> {
        self.inventory(CompilerCloneRootV1::GraphPattern(pattern))?;
        let out = super::collect_vars::collect_pattern_vars(pattern);
        self.checkpoint()?;
        Ok(out)
    }

    pub fn lookup(self, count: usize, key: &Variable) -> Result<()> {
        self.product(&[
            count,
            key.as_str()
                .len()
                .checked_add(1)
                .ok_or_else(|| self.overflow())?,
        ])
    }

    pub fn variable(self, var: &Variable) -> Result<Variable> {
        self.charge(1)?;
        self.charge(var.as_str().len())?;
        let out = var.clone();
        self.checkpoint()?;
        Ok(out)
    }

    pub fn info(self, info: &ComposedInfo) -> Result<ComposedInfo> {
        Ok(ComposedInfo {
            s_var: self.variable(&info.s_var)?,
            p_var: self.variable(&info.p_var)?,
            o_var: self.variable(&info.o_var)?,
        })
    }

    pub fn env_get(self, env: &StarEnv, var: &Variable) -> Result<Option<ComposedInfo>> {
        self.lookup(env.len(), var)?;
        env.get(var).map(|info| self.info(info)).transpose()
    }

    pub fn env_clone(self, env: &StarEnv) -> Result<StarEnv> {
        self.charge(env.len())?;
        // Pay each key/value before the one linear B-tree clone. No shadow copy.
        for (var, info) in env {
            self.charge(5)?;
            for v in [var, &info.s_var, &info.p_var, &info.o_var] {
                self.charge(v.as_str().len())?;
            }
        }
        let out = env.clone();
        self.checkpoint()?;
        Ok(out)
    }

    pub fn env_remove(self, env: &mut StarEnv, var: &Variable) -> Result<()> {
        self.lookup(env.len(), var)?;
        env.remove(var);
        self.checkpoint()
    }

    /// Reserve an ordered two-set merge before its iterator compares keys.
    pub fn union_scan(self, left: &BTreeSet<Variable>, right: &BTreeSet<Variable>) -> Result<()> {
        let count = left
            .len()
            .checked_add(right.len())
            .ok_or_else(|| self.overflow())?;
        let mut payload = count;
        for var in left.iter().chain(right) {
            self.charge(1)?;
            payload = payload
                .checked_add(var.as_str().len())
                .ok_or_else(|| self.overflow())?;
        }
        self.product(&[count, payload])
    }

    pub fn fresh(self, count: usize, prefix: &str, ordinal: usize) -> Result<()> {
        let digits = if ordinal == 0 {
            1
        } else {
            ordinal.ilog10() as usize + 1
        };
        let bytes = prefix
            .len()
            .checked_add(digits)
            .ok_or_else(|| self.overflow())?;
        self.product(&[count, bytes.checked_add(1).ok_or_else(|| self.overflow())?])?;
        self.product(&[2, bytes])?;
        self.charge(1)
    }

    pub fn overflow(self) -> Error {
        match self.0.mode {
            CompilerWorkMode::Metered(cx) => {
                cx.reject_build_resource(QueryControlError::AccountingOverflow)
            }
            CompilerWorkMode::Uncontrolled => {
                Error::Unsupported("RDF-star work accounting overflow".into())
            }
        }
    }
}

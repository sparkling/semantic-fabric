//! Root-first, projection-guarded realization, preserving the frozen outer SQL
//! column remap. No mutation is rolled forward across a rejected nested guard.
use std::collections::BTreeMap;

use spargebra::term::Variable;

use crate::build::control::{BuildVec, BuildWork};
use crate::iq::{Branch, ColRef, TermDef};
use crate::plan_measure::clone_root::CompilerCloneRootV1 as Root;
use crate::{CompilerWorkMode, Result};

use super::control::StarWork;
use super::StarEnv;

pub(crate) fn binding_copy(
    name: &str,
    bindings: &BTreeMap<String, TermDef>,
    mode: CompilerWorkMode<'_>,
) -> Result<Option<TermDef>> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    StarWork(work).product(&[bindings.len(), name.len() + 1])?;
    bindings
        .get(name)
        .map(|def| copy_def(def, work))
        .transpose()
}

fn copy_def(def: &TermDef, work: BuildWork<'_>) -> Result<TermDef> {
    if let CompilerWorkMode::Metered(cx) = work.mode {
        cx.reserve_ast_copy(Root::TermDef(def))?;
    }
    let out = def.clone();
    work.checkpoint()?;
    Ok(out)
}

pub(crate) fn composed_term_def_with_work_mode(
    var: &Variable,
    env: &StarEnv,
    bindings: &BTreeMap<String, TermDef>,
    mode: CompilerWorkMode<'_>,
) -> Result<Option<TermDef>> {
    composed(var, env, bindings, BuildWork::new(mode))
}

fn composed(
    var: &Variable,
    env: &StarEnv,
    bindings: &BTreeMap<String, TermDef>,
    work: BuildWork<'_>,
) -> Result<Option<TermDef>> {
    let work = work.enter()?;
    StarWork(work).lookup(env.len(), var)?;
    let Some(info) = env.get(var) else {
        return Ok(None);
    };
    let component = |var: &Variable| -> Result<Option<TermDef>> {
        StarWork(work).lookup(env.len(), var)?;
        if env.contains_key(var) {
            composed(var, env, bindings, work)
        } else {
            binding_copy(var.as_str(), bindings, work.mode)
        }
    };
    // Preserve left-to-right absence and box timing. An absent component never
    // triggers evaluation of a later, possibly recursive component.
    let Some(subject) = component(&info.s_var)? else {
        return Ok(None);
    };
    let subject = work.boxed(subject)?;
    let Some(predicate) = component(&info.p_var)? else {
        return Ok(None);
    };
    let predicate = work.boxed(predicate)?;
    let Some(object) = component(&info.o_var)? else {
        return Ok(None);
    };
    let object = work.boxed(object)?;
    work.checkpoint()?;
    Ok(Some(TermDef::ComposedTriple {
        subject,
        predicate,
        object,
    }))
}

pub(crate) fn apply_composed_bindings_with_work_mode(
    branches: &mut [Branch],
    env: &StarEnv,
    mode: CompilerWorkMode<'_>,
) -> Result<()> {
    let _span = tracing::info_span!(target: sf_core::TELEMETRY_TARGET,
        "sf.compiler.star_realization", operation = "bindings")
    .entered();
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    for branch in branches {
        apply(branch, env, work, false)?;
    }
    work.checkpoint()
}

pub(super) fn apply(
    branch: &mut Branch,
    env: &StarEnv,
    work: BuildWork<'_>,
    guarded: bool,
) -> Result<()> {
    let work = work.enter()?;
    // No composed definitions means no updates: the guard is identically true.
    // Still visit descendants and propagate DISTINCT in the original order.
    let before = if guarded && !env.is_empty() {
        Some(projection(branch, &[], work)?)
    } else {
        None
    };
    let mut updates = BuildVec::new(Vec::new());
    for var in env.keys() {
        work.charge(1)?;
        if let Some(def) = composed(var, env, &branch.bindings, work)? {
            let name = work.string(var.as_str())?;
            work.push(&mut updates, (name, def))?;
        }
    }
    if let Some(before) = before {
        let after = projection(branch, &updates.values, work)?;
        if !same_columns(&before, &after, work)? {
            return Ok(());
        }
    }
    for (name, def) in updates.into_inner() {
        StarWork(work).product(&[branch.bindings.len(), name.len() + 1])?;
        work.charge(1)?;
        branch.bindings.insert(name, def);
        work.checkpoint()?;
    }
    for subplan in &mut branch.subplan_joins {
        work.charge(1)?;
        super::env::propagate_single_branch_distinct(&mut subplan.plan);
        for inner in &mut subplan.plan.branches {
            apply(inner, env, work, true)?;
        }
    }
    work.checkpoint()
}

fn same_columns(left: &[ColRef], right: &[ColRef], work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (a, b) in left.iter().zip(right) {
        work.charge(1)?;
        work.charge(a.column.len().min(b.column.len()))?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}

/// One finite raw projection block, never a whole-realization precharge. Typed
/// measurement pays its own bounded traversal. N nodes bound emitted columns;
/// P payload bounds string comparisons. D pays copies, 4*N*N carrier bytes pays
/// temporary columns/geometric growth at every recursive columns() level, and
/// 2*N*(N+P) pays sorted merge plus inner/outer dedup comparisons. Repeated
/// branches/updates are charged on every call.
/// This is logical prospective admission, not mid-Clone/allocator preemption.
fn projection(
    branch: &Branch,
    updates: &[(String, TermDef)],
    work: BuildWork<'_>,
) -> Result<Vec<ColRef>> {
    if let CompilerWorkMode::Metered(cx) = work.mode {
        let mut m = cx.measure_ast_work(Root::Branch(branch))?;
        let overflow = || StarWork(work).overflow();
        for (name, def) in updates {
            work.charge(1)?;
            let extra = cx.measure_ast_work(Root::TermDef(def))?;
            m.nodes = m
                .nodes
                .checked_add(extra.nodes)
                .and_then(|n| n.checked_add(1))
                .ok_or_else(overflow)?;
            m.payload_bytes = m
                .payload_bytes
                .checked_add(extra.payload_bytes)
                .and_then(|p| p.checked_add(name.len() as u64))
                .ok_or_else(overflow)?;
            m.deep_clone_work = m
                .deep_clone_work
                .checked_add(extra.deep_clone_work)
                .ok_or_else(overflow)?;
        }
        let scans = m
            .nodes
            .checked_add(m.payload_bytes)
            .and_then(|p| p.checked_mul(m.nodes))
            .and_then(|s| s.checked_mul(2))
            .ok_or_else(overflow)?;
        let carriers = m
            .nodes
            .checked_mul(m.nodes)
            .and_then(|n| n.checked_mul(4 * std::mem::size_of::<ColRef>() as u64))
            .ok_or_else(overflow)?;
        cx.reserve_checked_sum(&[scans, carriers, m.deep_clone_work])?;
        work.checkpoint()?;
    }
    let out = super::env::projection_with_binding_updates(branch, updates);
    work.checkpoint()?;
    Ok(out)
}

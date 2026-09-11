//! Paid realization of composed variables. Each recursive occurrence is charged
//! independently; the initial rewrite's input envelope does not fund this phase.
use std::collections::HashSet;

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};

use crate::build::control::{BuildVec, BuildWork};
use crate::{CompilerWorkMode, Result};

use super::control::StarWork;
use super::StarEnv;

pub(crate) fn substitute_construct_template_with_work_mode(
    template: &[TriplePattern],
    env: &StarEnv,
    mode: CompilerWorkMode<'_>,
) -> Result<Vec<TriplePattern>> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    let mut out = work.vector(template.len())?;
    for tp in template {
        out.push(TriplePattern {
            subject: substitute(&tp.subject, env, work)?,
            predicate: work.copied(&tp.predicate)?,
            object: substitute(&tp.object, env, work)?,
        });
    }
    work.checkpoint()?;
    Ok(out)
}

fn substitute(term: &TermPattern, env: &StarEnv, work: BuildWork<'_>) -> Result<TermPattern> {
    let work = work.enter()?;
    if let TermPattern::Variable(var) = term {
        return substitute_variable(var, env, work);
    }
    // Match the raw API: an already explicit triple is copied, not substituted.
    work.copied(term)
}

fn substitute_variable(var: &Variable, env: &StarEnv, work: BuildWork<'_>) -> Result<TermPattern> {
    let work = work.enter()?;
    StarWork(work).lookup(env.len(), var)?;
    match env.get(var) {
        Some(info) => Ok(TermPattern::Triple(work.boxed(TriplePattern {
            subject: substitute_variable(&info.s_var, env, work)?,
            predicate: NamedNodePattern::Variable(StarWork(work).variable(&info.p_var)?),
            object: substitute_variable(&info.o_var, env, work)?,
        })?)),
        None => Ok(TermPattern::Variable(StarWork(work).variable(var)?)),
    }
}

fn contains(values: &[String], value: &str, work: BuildWork<'_>) -> Result<bool> {
    for existing in values {
        work.charge(1)?;
        work.charge(existing.len().min(value.len()))?;
        if existing == value {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn expand_projection_for_cascade_with_work_mode(
    vars: &[String],
    env: &StarEnv,
    mode: CompilerWorkMode<'_>,
) -> Result<Vec<String>> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    let mut initial = work.vector(vars.len())?;
    for var in vars {
        initial.push(work.string(var)?);
    }
    let mut out = BuildVec::new(initial);
    let mut changed = true;
    while changed {
        changed = false;
        for (var, info) in env {
            work.charge(1)?;
            if contains(&out.values, var.as_str(), work)? {
                for component in [&info.s_var, &info.p_var, &info.o_var] {
                    if !contains(&out.values, component.as_str(), work)? {
                        let name = work.string(component.as_str())?;
                        work.push(&mut out, name)?;
                        changed = true;
                    }
                }
            }
        }
    }
    work.checkpoint()?;
    Ok(out.into_inner())
}

pub(crate) fn all_component_var_names_with_work_mode(
    env: &StarEnv,
    mode: CompilerWorkMode<'_>,
) -> Result<HashSet<String>> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    let slots = env
        .len()
        .checked_mul(3)
        .ok_or_else(|| StarWork(work).overflow())?;
    // A fixed requested capacity bounds every probe independently of randomized
    // hash placement. Pay keys/hash bytes and the worst-case logical comparisons
    // before each insertion, not actual iteration order or allocator overgrant.
    StarWork(work).product(&[slots, std::mem::size_of::<String>() + 1])?;
    let mut out = HashSet::new();
    out.try_reserve(slots).map_err(|_| match mode {
        CompilerWorkMode::Metered(cx) => cx.reject_build_resource(
            sf_core::query_control::QueryControlError::CompilerResourceExhausted,
        ),
        CompilerWorkMode::Uncontrolled => {
            crate::Error::Unsupported("RDF-star keep allocation".into())
        }
    })?;
    for info in env.values() {
        work.charge(1)?;
        for var in [&info.s_var, &info.p_var, &info.o_var] {
            let name = var.as_str();
            work.charge(name.len())?;
            StarWork(work).product(&[slots, name.len() + 1])?;
            out.insert(work.string(name)?);
            work.checkpoint()?;
        }
    }
    work.checkpoint()?;
    Ok(out)
}

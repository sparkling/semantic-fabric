//! Construction projection decisions and ordered relational output ownership.
//! Retained definitions move in place; no payload is cloned to decide membership.
use crate::build::control::{BuildVec, BuildWork};
use crate::iq::node::Var;
use crate::iq::{Branch, TermDef};
use crate::{CompilerWorkMode, Result};
use std::collections::{BTreeMap, HashSet};

pub(super) fn retain_bindings(
    bindings: &mut BTreeMap<String, TermDef>,
    project: &[Var],
    extra_keep: &HashSet<String>,
    work: BuildWork<'_>,
) -> Result<()> {
    let _span = tracing::debug_span!("sf.compiler.projection_retention").entered();
    if matches!(work.mode, CompilerWorkMode::Uncontrolled) {
        bindings.retain(|key, _| {
            project.iter().any(|p| p.as_ref() == key.as_str()) || extra_keep.contains(key)
        });
        return Ok(());
    }
    work.charge(1)?;
    let mut decisions = work.vector(bindings.len())?;
    for key in bindings.keys() {
        work.charge(1)?;
        // Preserve project-first short-circuiting, including duplicate names.
        let keep = work.contains(project, key)? || extra_contains(extra_keep, key, work)?;
        decisions.push(keep);
    }
    // The map cannot change between this decision pass and retain. Both visit
    // keys in ascending order. Prepay the bounded second pass; its predicate
    // only consumes booleans, with no queries, payload copies or comparisons.
    // Removal/destruction and hash-table physical internals are not qualified.
    work.charge(bindings.len())?;
    let mut decisions = decisions.into_iter();
    bindings.retain(|_, _| decisions.next().expect("one decision per original binding"));
    assert!(decisions.next().is_none(), "projection decision drift");
    work.checkpoint()
}

fn extra_contains(extra: &HashSet<String>, key: &str, work: BuildWork<'_>) -> Result<bool> {
    // Iteration can scan spare table capacity. Admit that reported capacity
    // before starting, not just len(); this is logical table admission, not
    // an exact bucket/probe/allocator-byte model. Supplied slack is observable
    // work. Scan every element so randomized iteration order cannot change it.
    work.charge(extra.capacity())?;
    let mut found = false;
    for candidate in extra {
        work.charge(1)?;
        if candidate.len() == key.len() {
            work.charge(key.len())?;
            found |= candidate == key;
        }
    }
    Ok(found)
}

pub(super) fn construction_output(len: usize, work: BuildWork<'_>) -> Result<Vec<Branch>> {
    let _span = tracing::debug_span!("sf.compiler.construction_output").entered();
    work.vector(len)
}

pub(super) fn append_union(
    out: &mut BuildVec<Branch>,
    branches: Vec<Branch>,
    work: BuildWork<'_>,
) -> Result<()> {
    let _span = tracing::debug_span!("sf.compiler.union_output").entered();
    work.charge(1)?;
    for branch in branches {
        work.push(out, branch)?;
    }
    Ok(())
}

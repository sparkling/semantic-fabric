//! Controlled lifting of RESOLVE markers; raw dedup_scope remains the oracle.
use crate::build::control::{BuildVec, BuildWork};
use crate::finalization_remap as remap;
use crate::iq::{Branch, TermDef};
use crate::plan_measure::clone_root::CompilerCloneRootV1 as Root;
use crate::unfold::DedupMarker;
use crate::{cascade, CompilerWorkMode, DedupScope, Error, PlanForm, Result};

#[cfg(test)]
#[path = "dedup_scope_work_tests.rs"]
mod tests;
use sf_core::ir::{TermMap, TermType};
use std::collections::{BTreeMap, HashMap};

fn fail(message: &'static str, work: BuildWork<'_>) -> Result<Error> {
    work.unsupported(message, || message.to_owned())
}
fn impure(work: BuildWork<'_>) -> Result<Error> {
    fail(
        "shared term-dedup requires a pure single-source branch or unary SubPlan chain -> 501",
        work,
    )
}
fn malformed(work: BuildWork<'_>) -> Result<Error> {
    fail("shared term-dedup key metadata is malformed -> 501", work)
}
fn text_equal(a: &str, b: &str, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    work.charge(a.len().min(b.len()))?;
    Ok(a == b)
}
fn put(
    out: &mut BTreeMap<String, TermDef>,
    name: String,
    def: TermDef,
    work: BuildWork<'_>,
) -> Result<()> {
    for existing in out.keys() {
        text_equal(existing, &name, work)?;
    }
    work.charge(std::mem::size_of::<(String, TermDef)>())?;
    out.insert(name, def);
    work.checkpoint()
}
fn copy_bindings(
    bindings: &BTreeMap<String, TermDef>,
    work: BuildWork<'_>,
) -> Result<BTreeMap<String, TermDef>> {
    let mut out = BTreeMap::new();
    for (key, value) in bindings {
        work.charge(1)?;
        let key = work.string(key)?;
        if let CompilerWorkMode::Metered(cx) = work.mode {
            cx.reserve_ast_copy(Root::TermDef(value))?;
        }
        put(&mut out, key, value.clone(), work)?;
    }
    Ok(out)
}
fn lookup<'a>(
    bindings: &'a BTreeMap<String, TermDef>,
    name: &str,
    work: BuildWork<'_>,
) -> Result<Option<&'a TermDef>> {
    for (key, value) in bindings {
        if text_equal(key, name, work)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
fn reaches(branch: &Branch, alias: usize, work: BuildWork<'_>) -> Result<bool> {
    let work = work.enter()?;
    if cascade::relation_scan_with_work(branch, alias, work)?.is_some() {
        return Ok(true);
    }
    for wrapper in &branch.subplan_joins {
        work.charge(1)?;
        for nested in &wrapper.plan.branches {
            if reaches(nested, alias, work)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(crate) fn lift(
    branches: &[Branch],
    source: &HashMap<usize, DedupMarker>,
    work: BuildWork<'_>,
) -> Result<Vec<Option<DedupScope>>> {
    work.checkpoint()?;
    if source.is_empty() {
        return Ok(Vec::new());
    }
    work.charge(source.capacity())?;
    let mut markers: Vec<(usize, &DedupMarker)> = work.vector(source.len())?;
    for (alias, marker) in source {
        work.charge(1)?;
        markers.push((*alias, marker));
    }
    // Fixed-comparison selection avoids hash-seed-dependent work and error order.
    // The inventory is borrowed; each comparison and swap is interruptible/paid.
    for index in 0..markers.len() {
        let mut smallest = index;
        for next in index + 1..markers.len() {
            work.charge(1)?;
            if markers[next].0 < markers[smallest].0 {
                smallest = next;
            }
        }
        work.charge(1)?;
        markers.swap(index, smallest);
    }
    for (index, (_, marker)) in markers.iter().enumerate() {
        work.charge(1)?;
        if marker.key_bindings.is_empty() {
            return Err(malformed(work)?);
        }
        // Key-name consistency can be checked against borrowed prior metadata;
        // no copied key sets or hash-dependent lookup work is needed.
        for (_, prior) in &markers[..index] {
            work.charge(1)?;
            if prior.group_id == marker.group_id {
                if prior.key_bindings.len() != marker.key_bindings.len() {
                    return Err(malformed(work)?);
                }
                for (a, b) in prior.key_bindings.keys().zip(marker.key_bindings.keys()) {
                    if !text_equal(a, b, work)? {
                        return Err(malformed(work)?);
                    }
                }
            }
        }
    }
    let mut owned = work.vector(markers.len())?;
    owned.resize(markers.len(), 0usize);
    let mut out = work.vector(branches.len())?;
    for branch in branches {
        out.push(branch_scope(branch, &markers, &mut owned, work)?);
    }
    for count in owned {
        work.charge(1)?;
        if count != 1 {
            return Err(fail(
                "shared term-dedup marker is not owned by exactly one executable branch -> 501",
                work,
            )?);
        }
    }
    for (_, marker) in &markers {
        work.charge(1)?;
        let mut count = 0;
        for scope in &out {
            work.charge(1)?;
            if scope
                .as_ref()
                .is_some_and(|s| s.group_id == marker.group_id)
            {
                count += 1;
            }
        }
        if count < 2 {
            return Err(fail(
                "shared term-dedup group no longer spans two executable branches -> 501",
                work,
            )?);
        }
    }
    work.checkpoint()?;
    Ok(out)
}

fn branch_scope(
    branch: &Branch,
    markers: &[(usize, &DedupMarker)],
    owned: &mut [usize],
    work: BuildWork<'_>,
) -> Result<Option<DedupScope>> {
    let work = work.enter()?;
    let mut reachable = BuildVec::new(Vec::new());
    for (index, (alias, _)) in markers.iter().enumerate() {
        work.charge(1)?;
        if reaches(branch, *alias, work)? {
            work.push(&mut reachable, index)?;
        }
    }
    if reachable.values.is_empty() {
        return Ok(None);
    }
    let first = reachable.values[0];
    let group = markers[first].1.group_id;
    for index in &reachable.values {
        work.charge(1)?;
        if markers[*index].1.group_id != group {
            return Err(fail(
                "one executable branch reaches multiple shared term-dedup groups -> 501",
                work,
            )?);
        }
    }
    if branch.path.is_some()
        || branch.agg.is_some()
        || branch.limit.is_some()
        || branch.offset > 0
        || branch.nps
    {
        return Err(impure(work)?);
    }
    // Compare separate lengths instead of an unchecked aggregate addition.
    let pure = branch.opts.is_empty()
        && ((branch.core.len() == 1 && branch.subplan_joins.is_empty())
            || (branch.core.is_empty() && branch.subplan_joins.len() == 1));
    if !pure {
        return Err(impure(work)?);
    }
    if let Some(scan) = branch.core.first() {
        if reachable.values.len() != 1 || markers[first].0 != scan.alias {
            return Err(impure(work)?);
        }
        work.charge(1)?;
        owned[first] = owned[first]
            .checked_add(1)
            .ok_or_else(|| Error::Unsupported("shared term-dedup ownership overflow".into()))?;
        return Ok(Some(DedupScope {
            group_id: group,
            key_bindings: copy_bindings(&markers[first].1.key_bindings, work)?,
        }));
    }
    let wrapper = &branch.subplan_joins[0];
    let nested = &wrapper.plan;
    if wrapper.left
        || !wrapper.on.is_empty()
        || nested.branches.len() != 1
        || nested.limit.is_some()
        || nested.offset > 0
        || nested.rust_group.is_some()
        || !matches!(nested.form, PlanForm::Select { .. })
        || !nested.dedup_scopes.is_empty()
    {
        return Err(fail(
            "shared term-dedup cannot cross a multi-branch or modifier-bearing SQL SubPlan -> 501",
            work,
        )?);
    }
    let Some(scope) = branch_scope(&nested.branches[0], markers, owned, work)? else {
        return Err(impure(work)?);
    };
    if scope.group_id != group {
        return Err(impure(work)?);
    }
    let PlanForm::Select { vars } = &nested.form else {
        unreachable!("checked Select");
    };
    for key in scope.key_bindings.keys() {
        let mut found = false;
        for variable in vars {
            if text_equal(variable, key, work)? {
                found = true;
                break;
            }
        }
        if !found {
            return Err(fail(
                "shared term-dedup cannot cross a nested projection that removes its key -> 501",
                work,
            )?);
        }
    }
    let mut prepared = work.mode.clone_branch_forest(&nested.branches)?;
    let branch = &mut prepared[0];
    work.charge(1)?;
    branch.distinct = nested.distinct;
    if nested.order.is_empty() {
        branch.limit = nested.limit;
        branch.offset = nested.offset;
    }
    for (key, expected) in &scope.key_bindings {
        let Some(actual) = lookup(&branch.bindings, key, work)? else {
            return Err(fail("shared term-dedup cannot cross a wrapper that does not physically emit its key -> 501", work)?);
        };
        if !remap::same(actual, expected, work)? {
            return Err(fail("shared term-dedup cannot cross a wrapper that does not physically emit its key -> 501", work)?);
        }
    }
    validate_distinct(branch, work)?;
    let projection = super::projection(branch, work)?;
    let mut bindings = BTreeMap::new();
    for (key, def) in &scope.key_bindings {
        let value = remap::term(def, &projection, wrapper.alias, work)?;
        let key = work.string(key)?;
        put(&mut bindings, key, value, work)?;
    }
    Ok(Some(DedupScope {
        group_id: group,
        key_bindings: bindings,
    }))
}

fn validate_distinct(branch: &Branch, work: BuildWork<'_>) -> Result<()> {
    work.charge(1)?;
    if !branch.distinct {
        return Ok(());
    }
    let mut noninjective = false;
    for def in branch.bindings.values() {
        if !cascade::binding_injective_with_work(def, work)? {
            noninjective = true;
            break;
        }
    }
    if !noninjective {
        return Ok(());
    }
    // branch_scope has already established the pure single-source shape.
    for def in branch.bindings.values() {
        if cascade::binding_injective_with_work(def, work)? {
            continue;
        }
        let map = match def {
            TermDef::Derived { term_map, .. } | TermDef::R2rmlBlank { term_map, .. } => {
                Some(term_map)
            }
            _ => None,
        };
        let safe = map.is_some_and(|map| {
            matches!(map, TermMap::Column(_, spec) if spec.term_type == TermType::Iri)
                || crate::iq::term_map_type(map) != Some(TermType::Iri)
        });
        if !safe {
            return Err(fail("SELECT DISTINCT over a non-injective term cannot be pushed to raw SQL DISTINCT soundly -> 501 (ADR-0025 C.3)", work)?);
        }
    }
    Ok(())
}

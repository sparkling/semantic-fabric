//! Prospective work control for the final CONSTRUCT/DESCRIBE projection proof.
//! Optimizer passes and execution are separate boundaries.
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::build::control::{BuildVec, BuildWork};
use crate::iq::Branch;
use crate::{PlanForm, Result};

#[cfg(test)]
#[path = "finalization_graph_tests.rs"]
mod tests;

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

fn insert(vars: &mut BuildVec<String>, value: &str, work: BuildWork<'_>) -> Result<()> {
    if !contains(&vars.values, value, work)? {
        let value = work.string(value)?;
        work.push(vars, value)?;
    }
    Ok(())
}

fn collect(term: &TermPattern, vars: &mut BuildVec<String>, work: BuildWork<'_>) -> Result<()> {
    let work = work.enter()?;
    match term {
        TermPattern::Variable(var) => insert(vars, var.as_str(), work)?,
        TermPattern::Triple(triple) => collect_triple(triple, vars, work)?,
        _ => {}
    }
    Ok(())
}

fn collect_triple(
    triple: &TriplePattern,
    vars: &mut BuildVec<String>,
    work: BuildWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    collect(&triple.subject, vars, work)?;
    work.charge(1)?;
    if let NamedNodePattern::Variable(var) = &triple.predicate {
        insert(vars, var.as_str(), work)?;
    }
    collect(&triple.object, vars, work)
}

fn variables(template: &[TriplePattern], work: BuildWork<'_>) -> Result<Vec<String>> {
    let mut vars = BuildVec::new(Vec::new());
    for triple in template {
        collect_triple(triple, &mut vars, work)?;
    }
    Ok(vars.into_inner())
}

fn blank(term: &TermPattern, work: BuildWork<'_>) -> Result<bool> {
    let work = work.enter()?;
    Ok(match term {
        TermPattern::BlankNode(_) => true,
        TermPattern::Triple(triple) => {
            blank(&triple.subject, work)? || blank(&triple.object, work)?
        }
        _ => false,
    })
}

fn subset(vars: &[String], branch: &Branch, work: BuildWork<'_>) -> Result<bool> {
    for variable in vars {
        let mut found = false;
        // Borrowed linear lookup exposes every comparison without assuming
        // implementation-specific BTree search counts.
        for key in branch.bindings.keys() {
            work.charge(1)?;
            work.charge(key.len().min(variable.len()))?;
            if key == variable {
                found = true;
                break;
            }
        }
        if !found {
            return Ok(false);
        }
    }
    Ok(true)
}

fn retain(branch: &mut Branch, vars: &[String], work: BuildWork<'_>) -> Result<()> {
    let mut remove = BuildVec::new(Vec::new());
    for key in branch.bindings.keys() {
        work.charge(1)?;
        if !contains(vars, key, work)? {
            let key = work.string(key)?;
            work.push(&mut remove, key)?;
        }
    }
    for key in remove.into_inner() {
        // At most one visit per extant key bounds the actual BTree removal;
        // pay its comparisons before any binding is removed.
        for existing in branch.bindings.keys() {
            work.charge(1)?;
            work.charge(existing.len().min(key.len()))?;
        }
        work.charge(1)?;
        branch.bindings.remove(&key);
        work.checkpoint()?;
    }
    Ok(())
}

pub(crate) fn construct(
    branches: &mut [Branch],
    template: &[TriplePattern],
    work: BuildWork<'_>,
) -> Result<bool> {
    work.checkpoint()?;
    for triple in template {
        work.charge(1)?;
        if blank(&triple.subject, work)? || blank(&triple.object, work)? {
            return Ok(false);
        }
    }
    let vars = variables(template, work)?;
    if vars.is_empty() {
        return Ok(false);
    }
    let mut drops_some = false;
    'outer: for branch in &*branches {
        work.charge(1)?;
        for key in branch.bindings.keys() {
            work.charge(1)?;
            if !contains(&vars, key, work)? {
                drops_some = true;
                break 'outer;
            }
        }
    }
    for branch in branches {
        work.charge(1)?;
        if branch.path.is_some()
            || branch.agg.is_some()
            || !branch.order.is_empty()
            || branch.limit.is_some()
            || branch.offset != 0
        {
            continue;
        }
        if !subset(&vars, branch, work)? || vars.len() >= branch.bindings.len() {
            continue;
        }
        retain(branch, &vars, work)?;
        branch.distinct = true;
    }
    work.checkpoint()?;
    Ok(drops_some)
}

pub(crate) fn describe(
    branches: &mut [Branch],
    form: &PlanForm,
    work: BuildWork<'_>,
) -> Result<bool> {
    work.charge(1)?;
    let fail = |message| work.unsupported(message, || message.to_owned());
    let PlanForm::Construct { template } = form else {
        return Err(fail("DESCRIBE lowering did not produce a construct plan")?);
    };
    let [triple] = template.as_slice() else {
        return Err(fail(
            "DESCRIBE graph-set proof requires exactly one outgoing triple template",
        )?);
    };
    if !matches!(
        &triple.subject,
        TermPattern::NamedNode(_) | TermPattern::Variable(_)
    ) || !matches!(&triple.predicate, NamedNodePattern::Variable(_))
        || !matches!(&triple.object, TermPattern::Variable(_))
    {
        return Err(fail(
            "DESCRIBE graph-set proof requires the one-hop outgoing template",
        )?);
    }
    let vars = variables(template, work)?;
    for branch in &mut *branches {
        work.charge(1)?;
        if branch.path.is_some()
            || branch.agg.is_some()
            || !branch.order.is_empty()
            || branch.limit.is_some()
            || branch.offset != 0
            || !subset(&vars, branch, work)?
        {
            return Err(fail(
                "DESCRIBE graph-set projection is not proved for this plan shape",
            )?);
        }
        retain(branch, &vars, work)?;
        branch.distinct = true;
    }
    Ok(branches.len() > 1 && !crate::unfold::all_pairwise_disjoint_with_work(branches, work)?)
}

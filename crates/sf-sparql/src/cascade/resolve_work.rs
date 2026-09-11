//! Controlled source authority capture for IQ RESOLVE; raw capture is its oracle.
use super::{PoolSourceAuthority, PoolSourceAuthorityEntry};
use crate::build::control::{BuildVec, BuildWork};
use crate::iq::{Branch, Scan, SqlCond};
use crate::{CompilerWorkMode, Result};

pub(crate) fn relation_scan<'a>(
    branch: &'a Branch,
    alias: usize,
    work: BuildWork<'_>,
) -> Result<Option<&'a Scan>> {
    fn condition<'a>(
        cond: &'a SqlCond,
        alias: usize,
        work: BuildWork<'_>,
    ) -> Result<Option<&'a Scan>> {
        let work = work.enter()?;
        match cond {
            SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
                for scan in scans {
                    work.charge(1)?;
                    if scan.alias == alias {
                        return Ok(Some(scan));
                    }
                }
                for cond in conds {
                    if let Some(scan) = condition(cond, alias, work)? {
                        return Ok(Some(scan));
                    }
                }
            }
            SqlCond::And(conds) | SqlCond::Or(conds) => {
                for cond in conds {
                    if let Some(scan) = condition(cond, alias, work)? {
                        return Ok(Some(scan));
                    }
                }
            }
            SqlCond::Not(cond) => return condition(cond, alias, work),
            _ => (),
        }
        Ok(None)
    }
    work.checkpoint()?;
    for scan in branch
        .core
        .iter()
        .chain(branch.opts.iter().map(|opt| &opt.scan))
    {
        work.charge(1)?;
        if scan.alias == alias {
            return Ok(Some(scan));
        }
    }
    for cond in branch.where_conds.iter().chain(
        branch
            .opts
            .iter()
            .flat_map(|opt| opt.on.iter().chain(&opt.extra)),
    ) {
        if let Some(scan) = condition(cond, alias, work)? {
            return Ok(Some(scan));
        }
    }
    Ok(None)
}

pub(crate) fn incompatible_sql_types(left: &str, right: &str, work: BuildWork<'_>) -> Result<bool> {
    fn float_type(text: &str, work: BuildWork<'_>) -> Result<bool> {
        work.charge(1)?;
        if text.len() == 4 {
            work.charge(4)?;
            if text.eq_ignore_ascii_case("real") {
                return Ok(true);
            }
        }
        // ASCII-only folding is byte-equivalent to the old lowercase String
        // followed by contains, including non-ASCII surrounding text.
        for needle in [b"float".as_slice(), b"double".as_slice()] {
            for candidate in text.as_bytes().windows(needle.len()) {
                work.charge(1)?;
                work.charge(needle.len())?;
                if candidate.eq_ignore_ascii_case(needle) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    work.charge(1)?;
    if left.len() == right.len() {
        work.charge(left.len())?;
        if left.eq_ignore_ascii_case(right) {
            return Ok(false);
        }
    }
    Ok(float_type(left, work)? || float_type(right, work)?)
}

/// Borrow ordered column recipes without the recursive temporary vectors built
/// by TermDef::columns. Only a blank node's graph contribution is deduplicated,
/// against that blank node's own identifier/graph columns, never sibling terms.
pub(crate) fn columns<'a>(
    definition: &'a crate::iq::TermDef,
    work: BuildWork<'_>,
) -> Result<Vec<(usize, &'a str)>> {
    fn append<'a>(
        alias: usize,
        name: &'a str,
        unique_from: Option<usize>,
        work: BuildWork<'_>,
        out: &mut BuildVec<(usize, &'a str)>,
    ) -> Result<()> {
        if let Some(start) = unique_from {
            for (other_alias, other_name) in &out.values[start..] {
                work.charge(1)?;
                if *other_alias == alias {
                    work.charge(other_name.len().min(name.len()))?;
                    if *other_name == name {
                        return Ok(());
                    }
                }
            }
        }
        work.push(out, (alias, name))
    }
    fn map<'a>(
        term: &'a sf_core::ir::TermMap,
        alias: usize,
        unique_from: Option<usize>,
        work: BuildWork<'_>,
        out: &mut BuildVec<(usize, &'a str)>,
    ) -> Result<()> {
        use sf_core::ir::{Segment, TermMap};
        work.charge(1)?;
        match term {
            TermMap::Constant(_) => (),
            TermMap::Column(name, _) => append(alias, name, unique_from, work, out)?,
            TermMap::Template(template, _) => {
                for segment in template.segments() {
                    work.charge(1)?;
                    if let Segment::Column(name) = segment {
                        append(alias, name, unique_from, work, out)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn visit<'a>(
        definition: &'a crate::iq::TermDef,
        work: BuildWork<'_>,
        out: &mut BuildVec<(usize, &'a str)>,
    ) -> Result<()> {
        use crate::iq::{R2rmlGraphScope, TermDef};
        let work = work.enter()?;
        match definition {
            TermDef::Const(_) => (),
            TermDef::Derived { term_map, alias } => map(term_map, *alias, None, work, out)?,
            TermDef::R2rmlBlank {
                term_map,
                alias,
                graph,
            } => {
                let start = out.values.len();
                map(term_map, *alias, None, work, out)?;
                if let R2rmlGraphScope::Mapped { term_map, alias } = graph {
                    map(term_map, *alias, Some(start), work, out)?;
                }
            }
            TermDef::Coalesce(left, right) => {
                visit(left, work, out)?;
                visit(right, work, out)?;
            }
            TermDef::Concat(parts) => {
                for part in parts {
                    visit(part, work, out)?;
                }
            }
            TermDef::Agg { col, .. } => append(col.alias, &col.column, None, work, out)?,
            TermDef::ComposedTriple {
                subject,
                predicate,
                object,
            } => {
                visit(subject, work, out)?;
                visit(predicate, work, out)?;
                visit(object, work, out)?;
            }
        }
        Ok(())
    }
    let mut out = BuildVec::new(Vec::new());
    visit(definition, work, &mut out)?;
    Ok(out.into_inner())
}

fn keep_contains(
    keep: &std::collections::HashSet<String>,
    name: &str,
    work: BuildWork<'_>,
) -> Result<bool> {
    work.charge(name.len())?; // Hash bytes, before the lookup.
    for candidate in keep {
        work.charge(1)?;
        work.charge(candidate.len().min(name.len()))?;
    }
    Ok(keep.contains(name))
}

pub(super) fn injective(def: &crate::iq::TermDef, work: BuildWork<'_>) -> Result<bool> {
    use crate::iq::{R2rmlGraphScope, TermDef};
    fn pay(map: &sf_core::ir::TermMap, work: BuildWork<'_>) -> Result<()> {
        if let sf_core::ir::TermMap::Template(template, _) = map {
            for segment in template.segments() {
                work.charge(1)?;
                if let sf_core::ir::Segment::Literal(text) = segment {
                    work.charge(text.len())?;
                }
            }
        }
        Ok(())
    }
    work.charge(1)?;
    match def {
        TermDef::Derived { term_map, .. } => pay(term_map, work)?,
        TermDef::R2rmlBlank {
            term_map, graph, ..
        } => {
            pay(term_map, work)?;
            if let R2rmlGraphScope::Mapped { term_map, .. } = graph {
                pay(term_map, work)?;
            }
        }
        _ => (),
    }
    let result = super::binding_is_injective(def);
    work.checkpoint()?;
    Ok(result)
}

pub(crate) fn can_fallback(
    branches: &[Branch],
    keep: &std::collections::HashSet<String>,
    work: BuildWork<'_>,
) -> Result<bool> {
    work.checkpoint()?;
    if branches.len() < 2 {
        return Ok(false);
    }
    for branch in branches {
        work.charge(1)?;
        if branch.path.is_some()
            || branch.agg.is_some()
            || branch.core.len() + branch.opts.len() + branch.subplan_joins.len() > 1
        {
            return Ok(false);
        }
        for name in keep {
            for key in branch.bindings.keys() {
                work.charge(1)?;
                work.charge(name.len().min(key.len()))?;
            }
            if !branch.bindings.contains_key(name.as_str()) {
                return Ok(false);
            }
        }
        for (name, definition) in &branch.bindings {
            if !keep_contains(keep, name, work)? || injective(definition, work)? {
                continue;
            }
            let map = match definition {
                crate::iq::TermDef::Derived { term_map, .. }
                | crate::iq::TermDef::R2rmlBlank { term_map, .. } => term_map,
                _ => return Ok(false),
            };
            use sf_core::ir::{TermMap, TermType};
            if !matches!(map, TermMap::Column(_, spec) if spec.term_type == TermType::Iri)
                && crate::iq::term_map_type(map) == Some(TermType::Iri)
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(crate) fn needs_resolved_iri(
    branches: &[Branch],
    keep: &std::collections::HashSet<String>,
    work: BuildWork<'_>,
) -> Result<bool> {
    if !can_fallback(branches, keep, work)? {
        return Ok(false);
    }
    let mut offender = false;
    for branch in branches {
        work.charge(1)?;
        for (name, definition) in &branch.bindings {
            if keep_contains(keep, name, work)? && !injective(definition, work)? {
                offender = true;
            }
        }
    }
    if !offender {
        return Ok(false);
    }
    for branch in branches {
        work.charge(1)?;
        for (name, definition) in &branch.bindings {
            if keep_contains(keep, name, work)?
                && matches!(definition,
                crate::iq::TermDef::Derived { term_map: sf_core::ir::TermMap::Column(_, spec), .. }
                if spec.term_type == sf_core::ir::TermType::Iri && spec.base.is_some())
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn capture(branch: &Branch, work: BuildWork<'_>) -> Result<PoolSourceAuthority> {
    work.checkpoint()?;
    let mut sources = BuildVec::new(Vec::new());
    for scan in &branch.core {
        visit_scan(scan, true, work, &mut sources)?;
    }
    for opt in &branch.opts {
        visit_scan(&opt.scan, false, work, &mut sources)?;
    }
    for condition in &branch.where_conds {
        visit_condition(condition, work, &mut sources)?;
    }
    Ok(PoolSourceAuthority {
        sources: sources.into_inner(),
    })
}

pub(crate) fn narrow(
    members: &mut [Branch],
    keep: &std::collections::HashSet<String>,
    work: BuildWork<'_>,
) -> Result<()> {
    work.checkpoint()?;
    for branch in members {
        work.charge(1)?;
        // Retain cannot return a control error: decide membership first, in
        // exactly the same ordered-map iteration order, before any removals.
        let mut decisions = work.vector(branch.bindings.len())?;
        for name in branch.bindings.keys() {
            decisions.push(keep_contains(keep, name, work)?);
        }
        let mut decisions = decisions.into_iter();
        branch
            .bindings
            .retain(|_, _| decisions.next().expect("one decision per binding"));
        branch.distinct = true;
        work.checkpoint()?;
        for scan in &mut branch.core {
            work.charge(1)?;
            let crate::iq::ScanSource::Projection {
                columns,
                distinct,
                native_keys,
                lexical_keys,
                ..
            } = &mut scan.source
            else {
                continue;
            };
            if !native_keys.is_empty() {
                continue;
            }
            let mut iri = false;
            for key in lexical_keys {
                work.charge(1)?;
                if matches!(key.mode, crate::iq::scan::LexicalMode::Iri { .. }) {
                    iri = true;
                    break;
                }
            }
            if !iri {
                continue;
            }
            let mut raw_columns = true;
            for (name, term) in columns {
                work.charge(1)?;
                if let sf_core::ir::TermMap::Column(raw, _) = term {
                    work.charge(raw.len().min(name.len()))?;
                    if raw == name {
                        continue;
                    }
                }
                raw_columns = false;
                break;
            }
            if raw_columns {
                *distinct = false;
            }
            work.checkpoint()?;
        }
    }
    Ok(())
}

fn visit_scan(
    scan: &Scan,
    is_core: bool,
    work: BuildWork<'_>,
    sources: &mut BuildVec<PoolSourceAuthorityEntry>,
) -> Result<()> {
    work.charge(1)?;
    let Some(source) = scan.source.logical() else {
        return Ok(());
    };
    for existing in &sources.values {
        work.charge(1)?;
        if existing.alias == scan.alias {
            return Ok(());
        }
    }
    let source = match work.mode {
        CompilerWorkMode::Uncontrolled => source.clone(),
        CompilerWorkMode::Metered(cx) => cx.clone_logical_source(source)?,
    };
    work.push(
        sources,
        PoolSourceAuthorityEntry {
            alias: scan.alias,
            source,
            is_core,
        },
    )?;
    work.checkpoint()
}

fn visit_condition(
    condition: &SqlCond,
    work: BuildWork<'_>,
    sources: &mut BuildVec<PoolSourceAuthorityEntry>,
) -> Result<()> {
    let work = work.enter()?;
    match condition {
        SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
            for scan in scans {
                visit_scan(scan, false, work, sources)?;
            }
            for condition in conds {
                visit_condition(condition, work, sources)?;
            }
        }
        SqlCond::Not(condition) => visit_condition(condition, work, sources)?,
        SqlCond::And(conditions) | SqlCond::Or(conditions) => {
            for condition in conditions {
                visit_condition(condition, work, sources)?;
            }
        }
        _ => (),
    }
    Ok(())
}

#[cfg(test)]
#[path = "resolve_work_tests.rs"]
mod tests;

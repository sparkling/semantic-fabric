//! Paid positional remapping for finalization; no optimizer decision is changed.
use sf_core::ir::{Segment, Template, TermMap, TermSpec};

use crate::build::control::BuildWork;
use crate::iq::{ColRef, R2rmlGraphScope, TermDef};
use crate::plan_measure::clone_root::CompilerCloneRootV1 as Root;
use crate::{CompilerWorkMode, Error, Result};

pub(crate) fn equal(a: &ColRef, b: &ColRef, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    if a.alias != b.alias {
        return Ok(false);
    }
    work.charge(a.column.len().min(b.column.len()))?;
    Ok(a.column == b.column)
}

pub(crate) fn position(
    alias: usize,
    column: &str,
    projection: &[ColRef],
    work: BuildWork<'_>,
) -> Result<Option<usize>> {
    for (index, candidate) in projection.iter().enumerate() {
        work.charge(1)?;
        if candidate.alias == alias {
            work.charge(candidate.column.len().min(column.len()))?;
            if candidate.column.as_ref() == column {
                return Ok(Some(index));
            }
        }
    }
    Ok(None)
}

pub(crate) fn name(
    prefix: &str,
    index: usize,
    width: usize,
    work: BuildWork<'_>,
) -> Result<String> {
    // Decimal formatting is bounded by the actual digits, not the value of the index.
    let digits = if index == 0 {
        1
    } else {
        index.ilog10() as usize + 1
    };
    work.charge(prefix.len())?;
    work.charge(digits.max(width))?;
    let value = format!("{prefix}{index:0width$}");
    work.checkpoint()?;
    Ok(value)
}

fn spec(value: &TermSpec, work: BuildWork<'_>) -> Result<TermSpec> {
    if let CompilerWorkMode::Metered(cx) = work.mode {
        cx.reserve_ast_copy(Root::TermSpec(value))?;
    }
    Ok(value.clone())
}

fn column_name(
    alias: usize,
    column: &str,
    projection: &[ColRef],
    work: BuildWork<'_>,
) -> Result<Box<str>> {
    let Some(pos) = position(alias, column, projection, work)? else {
        return Err(work.unsupported("SubPlan remap: column not in inner projection → 501", || {
            format!("SubPlan remap: column '{column}' on alias {alias} not in inner projection → 501")
        })?);
    };
    Ok(name("c", pos, 0, work)?.into_boxed_str())
}

pub(crate) fn colref(
    value: &ColRef,
    projection: &[ColRef],
    alias: usize,
    work: BuildWork<'_>,
) -> Result<ColRef> {
    Ok(ColRef {
        alias,
        column: column_name(value.alias, &value.column, projection, work)?,
    })
}

fn term_map(
    value: &TermMap,
    alias: usize,
    projection: &[ColRef],
    work: BuildWork<'_>,
) -> Result<TermMap> {
    work.charge(1)?;
    Ok(match value {
        TermMap::Constant(_) => {
            if let CompilerWorkMode::Metered(cx) = work.mode {
                cx.reserve_ast_copy(Root::TermMap(value))?;
            }
            value.clone()
        }
        TermMap::Column(column, specification) => TermMap::Column(
            column_name(alias, column, projection, work)?,
            spec(specification, work)?,
        ),
        TermMap::Template(template, specification) => {
            let mut segments = work.vector(template.segments().len())?;
            for segment in template.segments() {
                work.charge(1)?;
                segments.push(match segment {
                    Segment::Literal(value) => {
                        Segment::Literal(work.string(value)?.into_boxed_str())
                    }
                    Segment::Column(column) => {
                        Segment::Column(column_name(alias, column, projection, work)?)
                    }
                });
            }
            // A valid Template cannot contain an empty segment list.
            let template =
                Template::from_segments(segments).map_err(|e| Error::Sql(e.to_string()))?;
            TermMap::Template(template, spec(specification, work)?)
        }
    })
}

pub(crate) fn term(
    value: &TermDef,
    projection: &[ColRef],
    alias: usize,
    work: BuildWork<'_>,
) -> Result<TermDef> {
    let work = work.enter()?;
    let result = match value {
        TermDef::Const(_) => {
            if let CompilerWorkMode::Metered(cx) = work.mode {
                cx.reserve_ast_copy(Root::TermDef(value))?;
            }
            value.clone()
        }
        TermDef::Derived {
            term_map: map,
            alias: original,
        } => TermDef::Derived {
            term_map: term_map(map, *original, projection, work)?,
            alias,
        },
        TermDef::R2rmlBlank {
            term_map: map,
            alias: original,
            graph,
        } => {
            let map = term_map(map, *original, projection, work)?;
            let graph = match graph {
                R2rmlGraphScope::Default => R2rmlGraphScope::Default,
                R2rmlGraphScope::Mapped {
                    term_map: map,
                    alias: original,
                } => R2rmlGraphScope::Mapped {
                    term_map: term_map(map, *original, projection, work)?,
                    alias,
                },
            };
            TermDef::R2rmlBlank {
                term_map: map,
                alias,
                graph,
            }
        }
        TermDef::Coalesce(left, right) => TermDef::Coalesce(
            work.boxed(term(left, projection, alias, work)?)?,
            work.boxed(term(right, projection, alias, work)?)?,
        ),
        TermDef::Concat(parts) => {
            let mut out = work.vector(parts.len())?;
            for part in parts {
                out.push(term(part, projection, alias, work)?);
            }
            TermDef::Concat(out)
        }
        TermDef::Agg {
            col,
            kind,
            operand,
            fixed_type,
        } => TermDef::Agg {
            col: colref(col, projection, alias, work)?,
            kind: *kind,
            operand: operand
                .as_ref()
                .map(|c| colref(c, projection, alias, work))
                .transpose()?,
            fixed_type: *fixed_type,
        },
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => TermDef::ComposedTriple {
            subject: work.boxed(term(subject, projection, alias, work)?)?,
            predicate: work.boxed(term(predicate, projection, alias, work)?)?,
            object: work.boxed(term(object, projection, alias, work)?)?,
        },
    };
    work.checkpoint()?;
    Ok(result)
}

/// Compare the complete recipe, not an emitted lexical value. All variants are
/// field-complete; unlike Debug-string equality this does not allocate strings.
pub(crate) fn same(left: &TermDef, right: &TermDef, work: BuildWork<'_>) -> Result<bool> {
    let work = work.enter()?;
    fn map(a: &TermMap, b: &TermMap, work: BuildWork<'_>) -> Result<bool> {
        if let CompilerWorkMode::Metered(cx) = work.mode {
            let a = cx.measure_ast_work(Root::TermMap(a))?;
            let b = cx.measure_ast_work(Root::TermMap(b))?;
            cx.reserve_checked_sum(&[a.deep_clone_work, b.deep_clone_work])?;
        }
        work.checkpoint()?;
        let result = a == b;
        work.checkpoint()?;
        Ok(result)
    }
    fn graph(a: &R2rmlGraphScope, b: &R2rmlGraphScope, work: BuildWork<'_>) -> Result<bool> {
        work.charge(1)?;
        Ok(match (a, b) {
            (R2rmlGraphScope::Default, R2rmlGraphScope::Default) => true,
            (
                R2rmlGraphScope::Mapped {
                    term_map: a,
                    alias: x,
                },
                R2rmlGraphScope::Mapped {
                    term_map: b,
                    alias: y,
                },
            ) => x == y && map(a, b, work)?,
            _ => false,
        })
    }
    Ok(match (left, right) {
        (TermDef::Const(a), TermDef::Const(b)) => {
            if let CompilerWorkMode::Metered(cx) = work.mode {
                let a = cx.measure_ast_work(Root::TermDef(left))?;
                let b = cx.measure_ast_work(Root::TermDef(right))?;
                cx.reserve_checked_sum(&[a.deep_clone_work, b.deep_clone_work])?;
            }
            work.checkpoint()?;
            let result = a == b;
            work.checkpoint()?;
            result
        }
        (
            TermDef::Derived {
                term_map: a,
                alias: x,
            },
            TermDef::Derived {
                term_map: b,
                alias: y,
            },
        ) => x == y && map(a, b, work)?,
        (
            TermDef::R2rmlBlank {
                term_map: a,
                alias: x,
                graph: ga,
            },
            TermDef::R2rmlBlank {
                term_map: b,
                alias: y,
                graph: gb,
            },
        ) => x == y && map(a, b, work)? && graph(ga, gb, work)?,
        (TermDef::Coalesce(a, b), TermDef::Coalesce(c, d)) => {
            same(a, c, work)? && same(b, d, work)?
        }
        (TermDef::Concat(a), TermDef::Concat(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (a, b) in a.iter().zip(b) {
                if !same(a, b, work)? {
                    return Ok(false);
                }
            }
            true
        }
        (
            TermDef::Agg {
                col: a,
                kind: ak,
                operand: ao,
                fixed_type: at,
            },
            TermDef::Agg {
                col: b,
                kind: bk,
                operand: bo,
                fixed_type: bt,
            },
        ) => {
            if ak != bk || at != bt || !equal(a, b, work)? {
                return Ok(false);
            }
            match (ao, bo) {
                (Some(a), Some(b)) => equal(a, b, work)?,
                (None, None) => true,
                _ => false,
            }
        }
        (
            TermDef::ComposedTriple {
                subject: a,
                predicate: b,
                object: c,
            },
            TermDef::ComposedTriple {
                subject: x,
                predicate: y,
                object: z,
            },
        ) => same(a, x, work)? && same(b, y, work)? && same(c, z, work)?,
        _ => false,
    })
}

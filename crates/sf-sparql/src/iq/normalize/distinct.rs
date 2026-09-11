use super::control::RowWork;
use super::values::same_var_set;
use crate::iq::node::{IqNode, Var};
use crate::iq::TermDef;
use crate::{CompilerWorkMode, Result};

type ValuesRows = Vec<Vec<Option<TermDef>>>;

// ---- (e) Distinct-over-Values dedup (ADR-0023 optimizer-residue Wave C; Ontop
// ValuesNodeOptimization::test3normalizationDistinct) -----------------------------

/// Dedup a literal `Values` table directly when a `Distinct` sits over it (through
/// the same identity-projection `Construction` wrapper `normalize_slice` handles),
/// instead of carrying the `Distinct` down to LOWER (where `SELECT DISTINCT` already
/// produces the right *answer*, but every duplicate row still lowers to its own
/// branch first — the cosmetic cost this rule removes). Any other child shape keeps
/// the `Distinct` node as-is.
pub(super) fn normalize_distinct(child: IqNode, mode: CompilerWorkMode<'_>) -> Result<IqNode> {
    let work = RowWork::new(mode);
    work.charge(1)?;
    let result = match child {
        // `dedup_rows` declining (a non-Const cell) must NOT silently drop the
        // `Distinct` requirement itself — only discard the node when dedup actually
        // ran, else the duplicates it left behind would reach LOWER unguarded (a
        // wrong answer, not merely a missed optimization).
        IqNode::Values { vars, rows } => match dedup_rows(rows, work)? {
            Ok(deduped) => IqNode::Values {
                vars,
                rows: deduped,
            },
            Err(rows) => IqNode::Distinct {
                child: work.boxed(IqNode::Values { vars, rows })?,
            },
        },
        // SAFETY: only when `project` is the SAME variable set as the Values leaf's
        // own `vars` (a pure identity/reorder wrapper) — never a narrowing. DISTINCT
        // in SPARQL algebra applies AFTER Project (18.2.5): if `project` drops a
        // column, rows that differ only in the dropped column must collapse into
        // ONE post-projection row, but deduping the Values leaf's FULL (pre-
        // projection) tuples here would keep them as two — an `=_bag` violation an
        // adversarial review caught (`VALUES (?x ?y) {(1 2)(1 3)(1 2)} SELECT DISTINCT
        // ?x` must yield 1 row, not 2). Declining is always sound: `Distinct` still
        // runs (correctly) at LOWER/exec, same as before this rule existed.
        IqNode::Construction {
            child: inner,
            subst,
            project,
        } if matches!(&*inner, IqNode::Values { vars, .. } if same_var_set(&project, vars, work)?) => {
            IqNode::Construction {
                child: work.boxed(normalize_distinct(*inner, mode)?)?,
                subst,
                project,
            }
        }
        // Ontop `ValuesNodeOptimization::test9DistinctUnionValuesNonValues`: dedup
        // each Values-shaped arm's OWN internal duplicates in place, leaving any
        // other arm (and the outer `Distinct` itself — cross-arm duplicates are NOT
        // provable statically) untouched. `dedup_one_arm` re-applies the SAME
        // narrowing-projection guard `same_var_set` enforces above (an arm's own
        // declared columns must exactly match the Union's `project`, no narrowing)
        // — the identical `=_bag` hazard that guard exists for applies per-arm here
        // too, just checked arm-by-arm instead of once at the top.
        IqNode::Union { children, project } => {
            let mut out = work.vector(children.len())?;
            for arm in children {
                out.push(dedup_one_arm(arm, &project, work)?);
            }
            IqNode::Distinct {
                child: work.boxed(IqNode::Union {
                    children: out,
                    project,
                })?,
            }
        }
        child => IqNode::Distinct {
            child: work.boxed(child)?,
        },
    };
    work.checkpoint()?;
    Ok(result)
}

/// Dedup ONE `Union` arm's own internal duplicate rows (a bare `Values` leaf, or a
/// lone `VALUES`-as-union-arm wrapped in the builder's identity-projection
/// `Construction` — the same shape `static_row_count` recognizes, but returning the
/// (possibly unchanged) `IqNode` here rather than an extracted row list, since a
/// non-Values-shaped arm must be handed back completely untouched). Declines (the
/// arm unchanged) when its own declared columns aren't EXACTLY the Union's
/// `project` (no narrowing — the outer `IqNode::Union` dispatch in
/// `normalize_distinct` documents why), when `dedup_rows` itself declines (a
/// non-Const cell), or when nothing was actually duplicated.
fn dedup_one_arm(arm: IqNode, project: &[Var], work: RowWork<'_>) -> Result<IqNode> {
    work.charge(1)?;
    let result = match arm {
        IqNode::Values { vars, rows } if work.vars_equal(&vars, project)? => {
            let n = rows.len();
            match dedup_rows(rows, work)? {
                Ok(deduped) if deduped.len() < n => IqNode::Values {
                    vars,
                    rows: deduped,
                },
                Ok(rows) | Err(rows) => IqNode::Values { vars, rows },
            }
        }
        IqNode::Construction {
            child,
            subst,
            project: arm_project,
        } if subst.is_empty()
            && work.vars_equal(&arm_project, project)?
            && matches!(&*child, IqNode::Values { vars, .. } if work.vars_equal(vars, project)?) =>
        {
            let IqNode::Values { vars, rows } = *child else {
                unreachable!("matched above")
            };
            let n = rows.len();
            let deduped_child = match dedup_rows(rows, work)? {
                Ok(deduped) if deduped.len() < n => IqNode::Values {
                    vars,
                    rows: deduped,
                },
                Ok(rows) | Err(rows) => IqNode::Values { vars, rows },
            };
            IqNode::Construction {
                child: work.boxed(deduped_child)?,
                subst,
                project: arm_project,
            }
        }
        other => other,
    };
    work.checkpoint()?;
    Ok(result)
}

/// Remove duplicate rows, keeping the first occurrence's order (SPARQL DISTINCT:
/// multiset → set). `oxrdf::Term` already derives structural `PartialEq`/`Hash`
/// (`TermDef` itself does not), so comparison goes by each cell's underlying `Term`
/// — a `None` (UNDEF) cell compares equal to another `None`. Declines (returns
/// `rows` unchanged — still correct, `Distinct`/`SELECT DISTINCT` still runs at
/// LOWER as before) the moment any cell isn't a plain `Const`: a `Concat`/`Coalesce`/
/// `Agg`/`Derived` `TermDef` has no reconstruction-time-only comparable form here.
fn dedup_rows(
    rows: ValuesRows,
    work: RowWork<'_>,
) -> Result<std::result::Result<ValuesRows, ValuesRows>> {
    for row in &rows {
        work.charge(1)?;
        for cell in row {
            work.charge(1)?;
            if !matches!(cell, None | Some(TermDef::Const(_))) {
                return Ok(Err(rows));
            }
        }
    }

    let mut rows = rows;
    let mut unique_prefix = 0;
    while unique_prefix < rows.len() {
        work.charge(1)?;
        let mut duplicate = false;
        for seen in &rows[..unique_prefix] {
            work.charge(1)?;
            if const_rows_equal(seen, &rows[unique_prefix], work)? {
                duplicate = true;
                break;
            }
        }
        if duplicate {
            work.moved::<Vec<Option<TermDef>>>(rows.len() - unique_prefix - 1)?;
            rows.remove(unique_prefix);
            work.checkpoint()?;
        } else {
            unique_prefix += 1;
        }
    }
    work.checkpoint()?;
    Ok(Ok(rows))
}

fn const_rows_equal(
    left: &[Option<TermDef>],
    right: &[Option<TermDef>],
    work: RowWork<'_>,
) -> Result<bool> {
    work.charge(1)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.iter().zip(right) {
        work.charge(1)?;
        let equal = match (left, right) {
            (None, None) => true,
            (Some(left), Some(right)) => work.term_equal(left, right)?,
            _ => false,
        };
        if !equal {
            return Ok(false);
        }
    }
    work.checkpoint()?;
    Ok(true)
}

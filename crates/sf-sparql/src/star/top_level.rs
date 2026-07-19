//! The whole-query entry point ([`rewrite_query`]) and the narrow, provably-
//! sound top-level relaxation of the uniform-composed-ness law
//! ([`rewrite_top_level_pattern`]) — a SELECT query's OWN top-level `Union`
//! or single mixed-shape `Values` may disagree/mix on a variable's
//! composed-ness, PROVIDED nothing but the projection could ever observe the
//! disagreement (see [`rewrite_top_level_pattern`]'s own doc comment for the
//! full soundness argument). [`rewrite_union`] is the uniform-composed-ness
//! check itself — shared by this relaxed top-level entry (`top_level: true`,
//! skips the check) and [`super::walk::rewrite_pattern`]'s ordinary,
//! unconditional `Union` arm (`top_level: false`).

use spargebra::algebra::GraphPattern;
use spargebra::term::{GroundTerm, Variable};
use spargebra::Query;

use crate::{Error, Result};

use super::collect_vars::collect_pattern_vars;
use super::env::StarEnv;
use super::walk::rewrite_pattern;

/// Rewrite a whole query's WHERE pattern (rules R1-R7 plus the ADR-0032 D3
/// composed-variable extensions), threading one whole-query fresh-variable
/// counter (shared by `super::util::fresh_var` and `super::util::fresh_empty_var`
/// — never the per-clause `__sf_ord` pattern, which would collide across
/// sibling BGPs/UNION arms/EXISTS bodies) and one whole-query [`StarEnv`]. The
/// returned env records every variable the rewrite determined to be
/// triple-term-valued; `lib.rs` consults it to realize the native decode at
/// projection and to pre-substitute the CONSTRUCT template
/// (`super::env::substitute_construct_template`) — the CONSTRUCT template
/// itself (a separate `Vec<TriplePattern>`, not a `GraphPattern`) is
/// untouched HERE.
pub fn rewrite_query(query: &Query) -> Result<(Query, StarEnv)> {
    let mut n = 0usize;
    let mut env = StarEnv::new();
    let rewritten = match query {
        // SELECT gets ONE extra rewrite option beyond `rewrite_pattern`: the
        // narrow, provably-sound top-level relaxation of the uniform-
        // composed-ness law — see [`rewrite_top_level_pattern`]'s doc
        // comment. CONSTRUCT/DESCRIBE/ASK are NOT routed through it (kept on
        // the ordinary, unchanged `rewrite_pattern`): CONSTRUCT has its OWN
        // static `env`-consulting consumer
        // (`super::env::substitute_construct_template`) the relaxation's
        // soundness argument does not cover, and DESCRIBE/ASK do not share
        // SELECT's "the query's only consumer is the projection" shape.
        Query::Select {
            dataset,
            pattern,
            base_iri,
        } => Query::Select {
            dataset: dataset.clone(),
            pattern: rewrite_top_level_pattern(pattern, &mut n, &mut env)?,
            base_iri: base_iri.clone(),
        },
        Query::Construct {
            template,
            dataset,
            pattern,
            base_iri,
        } => Query::Construct {
            template: template.clone(),
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n, &mut env)?,
            base_iri: base_iri.clone(),
        },
        Query::Describe {
            dataset,
            pattern,
            base_iri,
        } => Query::Describe {
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n, &mut env)?,
            base_iri: base_iri.clone(),
        },
        Query::Ask {
            dataset,
            pattern,
            base_iri,
        } => Query::Ask {
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n, &mut env)?,
            base_iri: base_iri.clone(),
        },
    };
    Ok((rewritten, env))
}

/// **Ledger closeout, boundary A** — a narrow, provably-sound relaxation of
/// the uniform-composed-ness law for a SELECT query's OWN top-level pattern.
/// [`rewrite_union`]'s disagreement check (and `super::decompose::decompose_column`'s
/// mixed-VALUES-column check) exist because [`StarEnv`] is a WHOLE-QUERY map: a
/// variable's composed-ness, once resolved, is looked up STATICALLY (one
/// answer for the whole query) by exactly three consumers —
/// `super::expr::rewrite_and_check_composed` (SUBJECT/PREDICATE/OBJECT/isTRIPLE,
/// `=`/`sameTerm`) and `super::env::substitute_composed_term` (CONSTRUCT templates).
/// If a UNION's two arms — or a single VALUES column's rows — disagree on
/// whether a shared variable composes, any of those three consumers would
/// silently pick ONE answer and be WRONG for whichever arm/rows did not
/// match it (e.g. `FILTER(isTRIPLE(?t))` resolving to a constant `true`
/// that is actually false for half the union's rows) — genuinely unsound,
/// not merely a missed optimization (confirmed by reading `iq/normalize.rs`:
/// the TREE path's Filter/Join-DOES distribute over a `Union`, which would
/// make a PER-ARM answer available downstream — but only if this file
/// hadn't already collapsed it to one static answer BEFORE `iq::normalize`
/// ever runs).
///
/// This function closes exactly the case where NONE of those three
/// consumers can possibly exist: when the query's ENTIRE pattern, modulo
/// pass-through `SLICE`/`DISTINCT`/`REDUCED`/`PROJECT` wrappers (the only
/// wrappers a plain `SELECT ... WHERE { ... }` — no `FILTER`, `BIND`,
/// further join, `GROUP BY`, or `ORDER BY`-with-an-expression — produces),
/// is a bare `Union` or a single mixed-shape VALUES column. Reconstruction
/// is per-[`crate::iq::Branch`] and branch-local
/// (`super::env::apply_composed_bindings`'s `None`-on-absence doc comment;
/// each top-level `Plan` branch executes as its OWN SQL statement and is
/// reconstructed independently — `exec_core::run_branches`, never a single
/// SQL-level `UNION` requiring uniform column arity across arms), so a
/// disagreeing/mixed variable that is ONLY ever projected (never touched by
/// a sensitive consumer, because there is nothing else in the query to do
/// the touching) is safe.
///
/// A `Union`/mixed-VALUES reached ANY OTHER way — nested under a
/// `FILTER`/`BIND`/further join/`GROUP BY`/`ORDER BY`-expression/CONSTRUCT
/// template, or as a NESTED arm inside a larger `Union` spine (a documented,
/// not-yet-attempted generalization — only the OUTERMOST `Union` pair gets
/// this relaxation) — falls through to the ordinary, UNCHANGED
/// `rewrite_pattern` / `rewrite_union(top_level: false)` /
/// `super::decompose::rewrite_values`, the original 501.
/// `CONSTRUCT`/`DESCRIBE`/`ASK` never call this function at all (see
/// [`rewrite_query`]'s own doc comment).
pub(super) fn rewrite_top_level_pattern(
    gp: &GraphPattern,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    match gp {
        GraphPattern::Slice {
            inner,
            start,
            length,
        } => Ok(GraphPattern::Slice {
            inner: Box::new(rewrite_top_level_pattern(inner, n, env)?),
            start: *start,
            length: *length,
        }),
        GraphPattern::Distinct { inner } => Ok(GraphPattern::Distinct {
            inner: Box::new(rewrite_top_level_pattern(inner, n, env)?),
        }),
        GraphPattern::Reduced { inner } => Ok(GraphPattern::Reduced {
            inner: Box::new(rewrite_top_level_pattern(inner, n, env)?),
        }),
        GraphPattern::Project { inner, variables } => Ok(GraphPattern::Project {
            inner: Box::new(rewrite_top_level_pattern(inner, n, env)?),
            variables: variables.clone(),
        }),
        GraphPattern::Union { left, right } => rewrite_union(left, right, n, env, true),
        GraphPattern::Values {
            variables,
            bindings,
        } if is_single_column_mixed_values(variables, bindings) => {
            // Row-partition the ONE mixed column into two uniform VALUES
            // blocks, unioned — reduces VALUES-mixed to the union-mixed case
            // above, reusing the SAME `top_level` relaxation (the ticket's
            // own item 4: "a row-partitioned rewrite into UNION of two
            // VALUES could [close it]"). Each half is now uniform, so
            // `decompose_column`'s mixed check no longer fires for either.
            let (triple_rows, plain_rows) = partition_values_by_triple_shape(bindings);
            let left = GraphPattern::Values {
                variables: variables.clone(),
                bindings: triple_rows,
            };
            let right = GraphPattern::Values {
                variables: variables.clone(),
                bindings: plain_rows,
            };
            rewrite_union(&left, &right, n, env, true)
        }
        other => rewrite_pattern(other, n, env),
    }
}

/// [`rewrite_top_level_pattern`]'s VALUES-splitting precondition: exactly one
/// VALUES variable, whose column mixes a ground-triple-term cell with a
/// non-triple (or UNDEF) cell — the same mixed-shape condition
/// `super::decompose::decompose_column` itself checks, computed here BEFORE
/// decomposition so the split can run instead of the 501. Multi-column
/// VALUES is left entirely alone (falls through to the ordinary, unchanged
/// `super::decompose::rewrite_values`) — row-partitioning multiple
/// independently-mixed columns at once is a harder problem this ledger item
/// does not attempt.
fn is_single_column_mixed_values(
    variables: &[Variable],
    bindings: &[Vec<Option<GroundTerm>>],
) -> bool {
    let [_] = variables else {
        return false;
    };
    let has_triple = bindings
        .iter()
        .any(|row| matches!(row[0], Some(GroundTerm::Triple(_))));
    let has_non_triple = bindings
        .iter()
        .any(|row| matches!(&row[0], Some(g) if !matches!(g, GroundTerm::Triple(_))));
    has_triple && has_non_triple
}

/// One VALUES block's row set — [`partition_values_by_triple_shape`]'s
/// result shape (a pair of them), factored out for clippy's
/// `type_complexity` lint (mirrors `super::expr::ComposedComponents`'s own
/// reason for existing).
type ValuesRows = Vec<Vec<Option<GroundTerm>>>;

/// Partition a single-column VALUES block's rows by shape: a row whose cell
/// is a ground triple term goes to the first (`triple`) partition; a row
/// whose cell is UNDEF or an ordinary (non-triple) term goes to the second
/// (`plain`) partition. Both partitions are non-empty whenever
/// [`is_single_column_mixed_values`] was true (each is witnessed by the
/// cell that made `has_triple`/`has_non_triple` true).
pub(super) fn partition_values_by_triple_shape(
    bindings: &[Vec<Option<GroundTerm>>],
) -> (ValuesRows, ValuesRows) {
    let mut triple_rows = Vec::new();
    let mut plain_rows = Vec::new();
    for row in bindings {
        if matches!(row[0], Some(GroundTerm::Triple(_))) {
            triple_rows.push(row.clone());
        } else {
            plain_rows.push(row.clone());
        }
    }
    (triple_rows, plain_rows)
}

/// ADR-0032 D3 item 2 — the uniform-composed-ness law: a `Union`'s two arms
/// are checked for composed-ness agreement on any variable they BOTH
/// syntactically mention (collected from the ORIGINAL, pre-rewrite patterns —
/// [`collect_pattern_vars`]). Env lookup-before-mint means a variable
/// composed by one arm and REUSED (not re-composed) by the other is fine
/// (e.g. both arms independently reify the same `?t`) — checked here by
/// whether EACH arm's OWN rewritten output actually binds that variable's
/// `s_var` (not by "which arm minted it first", which would false-positive
/// on exactly that reuse case). A shared variable composed in one arm's
/// output but not the other's would make it observably "sometimes a triple
/// term" depending on which arm produced a given row — never allowed
/// silently (R5) UNLESS `top_level` (ledger closeout boundary A — see
/// [`rewrite_top_level_pattern`]'s doc comment for the soundness argument):
/// the check is skipped entirely in that case, since by construction nothing
/// in the query could observe the disagreement other than the projection.
/// `top_level` is `true` only from [`rewrite_top_level_pattern`]'s own Union/
/// mixed-VALUES arms; the ordinary recursive call from
/// [`super::walk::rewrite_pattern`] always passes `false`, so a `Union`
/// reached any other way (nested under a FILTER/BIND/further join/GROUP
/// BY/ORDER-BY-expression, or as an inner arm of a larger Union spine) keeps
/// the original, unconditional check.
pub(super) fn rewrite_union(
    left: &GraphPattern,
    right: &GraphPattern,
    n: &mut usize,
    env: &mut StarEnv,
    top_level: bool,
) -> Result<GraphPattern> {
    let rw_left = rewrite_pattern(left, n, env)?;
    let rw_right = rewrite_pattern(right, n, env)?;

    if !top_level {
        let left_vars = collect_pattern_vars(left);
        let right_vars = collect_pattern_vars(right);
        let shared = left_vars.intersection(&right_vars);
        for v in shared {
            let Some(info) = env.get(v).cloned() else {
                continue; // not composed anywhere ⇒ nothing to reconcile
            };
            let left_composes = collect_pattern_vars(&rw_left).contains(&info.s_var);
            let right_composes = collect_pattern_vars(&rw_right).contains(&info.s_var);
            if left_composes != right_composes {
                return Err(Error::Unsupported(format!(
                    "UNION arms disagree on whether ?{} is a triple term (composed by one arm, \
                     an ordinary binding in the other) → 501 (ADR-0032 D3 uniform-composed-ness \
                     law)",
                    v.as_str()
                )));
            }
        }
    }

    Ok(GraphPattern::Union {
        left: Box::new(rw_left),
        right: Box::new(rw_right),
    })
}

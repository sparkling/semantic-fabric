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
//!
//! Run 4 Wave B2 widens the boundary from "bare Union/Values" to "a single
//! static consumer (FILTER's function-call/equality machinery) DIRECTLY
//! wrapping one" — [`rewrite_filter_over_union`] — reusing the SAME
//! per-arm-composes signal [`composed_agreement`] factors out of
//! [`rewrite_union`]'s own check. `rewrite_top_level_pattern` is also now
//! called from [`super::expr::rewrite_expr`]'s `Exists` arm: an EXISTS/NOT
//! EXISTS body is the SAME kind of boundary (only a boolean escapes), so it
//! gets the identical relaxation.

use spargebra::algebra::{Expression, GraphPattern};
use spargebra::term::{GroundTerm, Variable};
use spargebra::Query;

use crate::{Error, Result};

use super::collect_vars::collect_pattern_vars;
use super::env::StarEnv;
use super::expr::rewrite_expr;
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
/// the uniform-composed-ness law for a SELECT query's OWN top-level pattern
/// (Run 4 Wave B2: and for an EXISTS/NOT EXISTS body — see
/// `super::expr::rewrite_expr`'s `Exists` arm — the same kind of boundary).
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
/// This function closes the case where NONE of those three consumers can
/// possibly exist: when the query's ENTIRE pattern, modulo pass-through
/// `SLICE`/`DISTINCT`/`REDUCED`/`PROJECT` wrappers, is a bare `Union` or a
/// single mixed-shape VALUES column. Reconstruction is per-[`crate::iq::Branch`]
/// and branch-local (`super::env::apply_composed_bindings`'s `None`-on-absence
/// doc comment; each top-level `Plan` branch executes as its OWN SQL
/// statement and is reconstructed independently — `exec_core::run_branches`,
/// never a single SQL-level `UNION` requiring uniform column arity across
/// arms), so a disagreeing/mixed variable that is ONLY ever projected (never
/// touched by a sensitive consumer, because there is nothing else in the
/// query to do the touching) is safe.
///
/// **Run 4 Wave B2** widens this to ONE MORE shape: a `FILTER` DIRECTLY
/// wrapping the `Union`/mixed-VALUES (`rewrite_filter_over_union`, below).
/// The first of the three static consumers CAN exist here — but instead of
/// resolving it once against one whole-query answer, it is resolved TWICE,
/// once per arm, each seeing ONLY that arm's own local composed-ness (a
/// FORK of `env` with the OTHER arm's disagreeing entries stripped) — the
/// exact per-arm answer `iq/normalize.rs`'s later, structurally-identical
/// `Filter(Union) ⇒ Union(Filter,Filter)` distribution would make available
/// downstream anyway, just resolved here, BEFORE composed-ness collapses to
/// one static answer, instead of too late to matter. Still sound by the
/// SAME argument: this is still reached ONLY at a boundary (SELECT's own
/// top-level pattern, or an EXISTS/NOT EXISTS body) beyond which nothing
/// else in the query observes the union's per-row shape.
///
/// A `Union`/mixed-VALUES reached ANY OTHER way — nested under `BIND`/a
/// further join/`GROUP BY`/`ORDER BY`-expression/CONSTRUCT template, wrapped
/// in a FILTER that is itself nested (not the pattern's own top-level node),
/// or as a NESTED arm inside a larger `Union` spine (a documented,
/// not-yet-attempted generalization — only the OUTERMOST `Union` pair gets
/// this relaxation) — falls through to the ordinary, UNCHANGED
/// `rewrite_pattern` / `rewrite_union(top_level: false)` /
/// `super::decompose::rewrite_values`, the original 501. CONSTRUCT's own
/// template substitution (`super::env::substitute_construct_template`) is a
/// whole-`Plan` static rewrite with no per-`Branch` counterpart to fork —
/// genuinely the "single consumer cross-branch uniformity" case neither this
/// function nor `rewrite_filter_over_union` can express, so `CONSTRUCT`
/// (for its OWN top-level pattern — an EXISTS body nested inside one still
/// gets the relaxation, same as anywhere else) never calls this function at
/// all (see [`rewrite_query`]'s own doc comment); `DESCRIBE`/`ASK` likewise.
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
        // Run 4 Wave B2: a FILTER directly wrapping the union/mixed-VALUES —
        // see `rewrite_filter_over_union`'s doc comment for the per-arm
        // resolution this needs beyond the bare-Union/Values cases above.
        GraphPattern::Filter { expr, inner } => match inner.as_ref() {
            GraphPattern::Union { left, right } => {
                rewrite_filter_over_union(expr, left, right, n, env)
            }
            GraphPattern::Values {
                variables,
                bindings,
            } if is_single_column_mixed_values(variables, bindings) => {
                let (triple_rows, plain_rows) = partition_values_by_triple_shape(bindings);
                let left = GraphPattern::Values {
                    variables: variables.clone(),
                    bindings: triple_rows,
                };
                let right = GraphPattern::Values {
                    variables: variables.clone(),
                    bindings: plain_rows,
                };
                rewrite_filter_over_union(expr, &left, &right, n, env)
            }
            _ => rewrite_pattern(gp, n, env),
        },
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
        for (v, left_composes, right_composes) in
            composed_agreement(left, right, &rw_left, &rw_right, env)
        {
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

/// For every variable BOTH `left`'s and `right`'s ORIGINAL (pre-rewrite)
/// patterns mention (collected via [`collect_pattern_vars`]) that `env` holds
/// [`super::env::ComposedInfo`] for, whether EACH arm's OWN rewritten output
/// (`rw_left`/`rw_right`) actually binds that variable's `s_var` — the
/// per-arm "does this arm actually compose it" signal both [`rewrite_union`]'s
/// strict check and [`rewrite_filter_over_union`]'s per-arm `expr`
/// resolution need. Factored out of `rewrite_union`'s own former inline loop
/// (Run 4 Wave B2) so both share ONE definition of "agree". A variable `env`
/// has no entry for at all is skipped — not composed anywhere, nothing to
/// reconcile.
fn composed_agreement(
    left: &GraphPattern,
    right: &GraphPattern,
    rw_left: &GraphPattern,
    rw_right: &GraphPattern,
    env: &StarEnv,
) -> Vec<(Variable, bool, bool)> {
    let left_vars = collect_pattern_vars(left);
    let right_vars = collect_pattern_vars(right);
    left_vars
        .intersection(&right_vars)
        .filter_map(|v| {
            let info = env.get(v)?;
            let left_composes = collect_pattern_vars(rw_left).contains(&info.s_var);
            let right_composes = collect_pattern_vars(rw_right).contains(&info.s_var);
            Some((v.clone(), left_composes, right_composes))
        })
        .collect()
}

/// **Run 4 Wave B2** — the static-consumer analog of the bare-Union/mixed-
/// VALUES top-level relaxation above: a `FILTER` DIRECTLY wrapping a `Union`
/// (or a mixed-VALUES already split into one by [`rewrite_top_level_pattern`]'s
/// `Filter` arm). When every variable both arms mention AGREES (including
/// "both arms independently compose the SAME var", which the ordinary
/// sequential left-then-right rewrite below already makes share ONE set of
/// component vars via env's lookup-before-mint), this is byte-for-byte what
/// the ordinary `rewrite_pattern`/`rewrite_union(top_level: false)` path
/// already produces for this shape (`expr` resolved once, `Filter` stays on
/// top of the `Union`) — reached here only because this file intercepts the
/// SHAPE before falling through to `rewrite_top_level_pattern`'s catch-all,
/// not because the answer differs.
///
/// On a genuine DISAGREEMENT — the shape that used to be an unconditional
/// 501 — `expr` is instead resolved TWICE: once per arm, each against a
/// clone of `env` with the OTHER arm's disagreeing entries removed, so e.g.
/// `isTRIPLE(?t)` resolves the constant `true` in the composing arm's own
/// copy and `false` in the other's (never one static answer wrong for half
/// the rows), and the `Filter` is distributed into each arm —
/// `Union{Filter{expr_L,L}, Filter{expr_R,R}}` — the SAME structural shape
/// `iq/normalize.rs`'s later `Filter(Union) ⇒ Union(Filter,Filter)`
/// distribution (design §4.16) produces, just built here, BEFORE
/// composed-ness has collapsed to one static answer, so each per-arm copy
/// can be resolved correctly in the first place. `env` is left holding the
/// ordinary (unfiltered) union of both arms' mints for the caller — safe by
/// the same branch-local-absence reasoning [`rewrite_top_level_pattern`]'s
/// doc comment gives for the bare-Union case; the temporary per-arm clones
/// here are local to resolving `expr` only.
fn rewrite_filter_over_union(
    expr: &Expression,
    left: &GraphPattern,
    right: &GraphPattern,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    let rw_left = rewrite_pattern(left, n, env)?;
    let rw_right = rewrite_pattern(right, n, env)?;
    let agreement = composed_agreement(left, right, &rw_left, &rw_right, env);

    if agreement.iter().all(|(_, l, r)| l == r) {
        return Ok(GraphPattern::Filter {
            expr: rewrite_expr(expr, n, env)?,
            inner: Box::new(GraphPattern::Union {
                left: Box::new(rw_left),
                right: Box::new(rw_right),
            }),
        });
    }

    let mut env_left = env.clone();
    let mut env_right = env.clone();
    for (v, left_composes, right_composes) in &agreement {
        if !left_composes {
            env_left.remove(v);
        }
        if !right_composes {
            env_right.remove(v);
        }
    }
    let expr_left = rewrite_expr(expr, n, &mut env_left)?;
    let expr_right = rewrite_expr(expr, n, &mut env_right)?;

    Ok(GraphPattern::Union {
        left: Box::new(GraphPattern::Filter {
            expr: expr_left,
            inner: Box::new(rw_left),
        }),
        right: Box::new(GraphPattern::Filter {
            expr: expr_right,
            inner: Box::new(rw_right),
        }),
    })
}

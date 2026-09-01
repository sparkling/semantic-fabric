//! Backend-independent GROUP BY and aggregate evaluation.

// ---------------------------------------------------------------------------
// Rust-level GROUP BY (multi-branch inner, SPARQL §11)
// ---------------------------------------------------------------------------

/// Group a collected multiset of inner solutions by `rg.keys`, compute each
/// aggregate in `rg.aggs`, then apply the plan's ORDER BY + OFFSET/LIMIT to the
/// grouped rows (SPARQL §15: order, then slice). Returns the final result rows in
/// emit order.
///
/// Shared by the SQLite ([`rust_group_execute`]) and PostgreSQL
/// ([`crate::exec_pg`]) multi-branch GROUP BY paths (ADR-0007): the
/// grouping/aggregation semantics are backend-independent — only the collection of
/// the inner solutions (SQLite `Connection` vs live PostgreSQL cursor) differs.
pub(crate) fn rust_group_result_rows(
    plan: &Plan,
    rg: &RustGroup,
    inner_rows: Vec<Bindings>,
) -> Result<Vec<Bindings>> {
    // Group by the key variable values, preserving insertion order for stable output.
    // Use a Vec for ordering + a HashMap for O(1) group lookup.
    type GroupKey = Vec<Option<Term>>;
    type GroupRows = Vec<Bindings>;
    #[allow(clippy::type_complexity)]
    let mut groups: Vec<(GroupKey, GroupRows)> = Vec::new();
    let mut key_index: std::collections::HashMap<Vec<Option<Term>>, usize> =
        std::collections::HashMap::new();

    for row in inner_rows {
        let key: Vec<Option<Term>> = rg.keys.iter().map(|k| row.get(k).cloned()).collect();
        if let Some(&idx) = key_index.get(&key) {
            groups[idx].1.push(row);
        } else {
            let idx = groups.len();
            key_index.insert(key.clone(), idx);
            groups.push((key, vec![row]));
        }
    }

    // Implicit grouping (no key variables): always produce exactly one group,
    // even over an empty inner (COUNT(*) ⇒ 0, AVG/MIN/MAX ⇒ UNBOUND — §11).
    if rg.keys.is_empty() && groups.is_empty() {
        groups.push((vec![], vec![]));
    }

    // Every result row below binds the SAME `rg.keys`/agg `out_var`s/post-expr
    // `out_var`s, once per GROUP — interned ONCE here, not per group (Run 4
    // Wave C1, the same "once per row stream" idiom `intern_bindings` uses
    // for a branch's own vars): a shared `Arc<str>` clone beats a fresh
    // `String` allocation per group.
    let key_names: Vec<Arc<str>> = rg.keys.iter().map(|k| Arc::from(k.as_str())).collect();
    let agg_names: Vec<Arc<str>> = rg
        .aggs
        .iter()
        .map(|a| Arc::from(a.out_var.as_str()))
        .collect();
    let post_names: Vec<Arc<str>> = rg
        .post_exprs
        .iter()
        .map(|(v, _)| Arc::from(v.as_str()))
        .collect();

    // Materialise the result row (key vars + aggregates) for every group.
    let mut result_rows: Vec<Bindings> = Vec::with_capacity(groups.len());
    for (key_vals, group_rows) in &groups {
        let mut result = Bindings::new();
        for (name, val) in key_names.iter().zip(key_vals.iter()) {
            if let Some(t) = val {
                result.insert(name.clone(), t.clone());
            }
        }
        for (agg_spec, name) in rg.aggs.iter().zip(&agg_names) {
            if let Some(t) = rust_agg(agg_spec, group_rows)? {
                result.insert(name.clone(), t);
            }
        }
        // ADR-0025 Tier-2 gap 5: post-GROUP-BY expressions over the aggregate outputs
        // (e.g. `COUNT(?x) * 2`). Evaluate each over the row's now-materialised aggregate +
        // group-key bindings via the shared `eval_expr`; an unbound reference yields no
        // binding (SPARQL: the value is unbound), never a wrong answer.
        for ((_, expr), name) in rg.post_exprs.iter().zip(&post_names) {
            if let Some(t) = eval_expr(expr, &result) {
                result.insert(name.clone(), t);
            }
        }
        result_rows.push(result);
    }

    // ORDER BY over the grouped rows (if requested), then OFFSET/LIMIT. Schwartzian
    // transform (ADR-0024/M4 perf, see `order_cmp_precomputed`): precompute each
    // row's sort keys once, sort a permutation of INDICES by them (keeps the keys'
    // borrow of `result_rows` and the final move out of it both sound), then
    // reorder `result_rows` by that permutation — moving each row exactly once
    // (`Option::take`), never cloning it.
    if !plan.order.is_empty() {
        let keys: Vec<Vec<Option<TermSortKey>>> = result_rows
            .iter()
            .map(|r| precompute_order_keys(&plan.order, r))
            .collect();
        let mut idx: Vec<usize> = (0..result_rows.len()).collect();
        idx.sort_by(|&i, &j| order_cmp_precomputed(&plan.order, &keys[i], &keys[j]));
        drop(keys);
        let mut slots: Vec<Option<Bindings>> = result_rows.into_iter().map(Some).collect();
        result_rows = idx
            .into_iter()
            .map(|i| {
                slots[i]
                    .take()
                    .expect("permutation index used exactly once")
            })
            .collect();
    }
    let take = plan.limit.unwrap_or(usize::MAX);
    Ok(result_rows
        .into_iter()
        .skip(plan.offset)
        .take(take)
        .collect())
}

/// Compute one aggregate over a group of solutions. Returns `None` for
/// UNBOUND (AVG/MIN/MAX over an empty multiset — SPARQL §11).
fn rust_agg(agg: &RustAgg, rows: &[Bindings]) -> Result<Option<Term>> {
    match agg.kind {
        AggKind::Count => {
            let count = match &agg.arg_var {
                // COUNT(DISTINCT *) — count DISTINCT whole solutions in the group. A row's
                // canonical key is its (var, term) pairs via `canonical_pairs` (Run 4 Wave C1:
                // `Bindings` preserves insertion order, not the old `BTreeMap`'s sorted-key
                // order, so the key must be canonicalized explicitly — see its doc comment;
                // `BTreeMap` used to give this order-independence for free). `oxrdf::Term`
                // derives `Hash`/`Eq` (already relied on elsewhere in this file, e.g.
                // `seen_tuples` above), and N-Triples serialisation is injective, so a
                // `&Term`-keyed dedup set yields the IDENTICAL classes a `Term::to_string()`-
                // keyed one would — without the per-value allocation (ADR-0025 Tier-2 gap 3;
                // ADR-0024/M4 perf).
                None if agg.distinct => {
                    let mut seen: std::collections::HashSet<Vec<(&str, &Term)>> =
                        std::collections::HashSet::new();
                    rows.iter()
                        .filter(|r| seen.insert(canonical_pairs(r)))
                        .count()
                }
                None => rows.len(), // COUNT(*)
                Some(var) => {
                    if agg.distinct {
                        let mut seen: std::collections::HashSet<&Term> =
                            std::collections::HashSet::new();
                        rows.iter()
                            .filter_map(|r| r.get(var))
                            .filter(|t| seen.insert(*t))
                            .count()
                    } else {
                        rows.iter().filter(|r| r.contains_key(var.as_str())).count()
                    }
                }
            };
            Ok(Some(Term::Literal(Literal::new_typed_literal(
                count.to_string(),
                sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
            ))))
        }
        AggKind::Sum => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            // ADR-0025 C.5: SUM/AVG/MIN/MAX PROPAGATE an unbound operand — a NON-empty group
            // with ANY row whose operand var is unbound ⇒ the whole aggregate is UNBOUND
            // (SPARQL §11; spareval-confirmed). Only COUNT filters errors. This extends C.4,
            // which handled only the all-unbound case for AVG; the mixed bound+unbound group
            // (and SUM over all-unbound) was still wrongly computed over just the bound rows.
            if !rows.is_empty() && rows.iter().any(|r| r.get(var).is_none()) {
                return Ok(None);
            }
            let vals: Vec<&Term> = dedup_if_distinct(
                rows.iter().filter_map(|r| r.get(var)).collect(),
                agg.distinct,
            );
            if vals.is_empty() {
                // SUM over empty multiset ⇒ "0"^^xsd:integer (SPARQL §11).
                return Ok(Some(Term::Literal(Literal::new_typed_literal(
                    "0",
                    sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
                ))));
            }
            let nums: Vec<f64> = vals.iter().filter_map(|t| numeric_term(t)).collect();
            if nums.len() < vals.len() {
                return Ok(None); // non-numeric operand ⇒ UNBOUND (type error)
            }
            let sum: f64 = nums.iter().sum();
            // SPARQL §11.4 / XPath numeric type promotion: any xsd:double operand ⇒ double
            // result; else all-integer ⇒ integer; else decimal. (C.6b: rust_agg previously
            // always emitted integer-or-decimal, losing xsd:double — a datatype =_bag
            // divergence exposed once C.6 routed nullable double-operand SUMs here.)
            if vals.iter().any(|t| is_xsd_double(t)) {
                Ok(Some(double_term(sum)?))
            } else if vals.iter().all(|t| is_xsd_integer(t)) {
                Ok(Some(integer_term(sum as i64)))
            } else {
                Ok(Some(decimal_term(sum)?))
            }
        }
        AggKind::Avg => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            // ADR-0025 C.5 (see SUM): any unbound operand row in a non-empty group ⇒ UNBOUND.
            if !rows.is_empty() && rows.iter().any(|r| r.get(var).is_none()) {
                return Ok(None);
            }
            let vals: Vec<&Term> = dedup_if_distinct(
                rows.iter().filter_map(|r| r.get(var)).collect(),
                agg.distinct,
            );
            if vals.is_empty() {
                // ADR-0025 C.4: AVG over no bound values. If the GROUP is genuinely EMPTY
                // (0 rows — e.g. implicit grouping over an unmatched pattern), AVG ⇒
                // "0"^^xsd:integer (SPARQL §11, like SUM; spareval-confirmed). But if the
                // group HAS rows and the operand is simply UNBOUND in every one of them
                // (e.g. `AVG(?missing)` over a UNION arm that never binds it), there are no
                // numeric values to average ⇒ the result is UNBOUND, NOT 0. The old
                // `vals.is_empty()` conflated these two — discriminate on `rows`.
                return if rows.is_empty() {
                    Ok(Some(integer_term(0)))
                } else {
                    Ok(None)
                };
            }
            // SPARQL §11.4: AVG of xsd:double values stays xsd:double (else decimal). See the
            // SUM promotion note above (C.6b) — mirrors the SQL path's `avg_result_code`.
            //
            // ADR-0025 C.10: both arms below gate on `nums.len() < vals.len()`, NOT
            // `nums.is_empty()` — the same non-numeric-operand check `AggKind::Sum` already
            // uses above. The `is_empty()` form only caught an ALL-non-numeric group; a group
            // MIXING numeric and non-numeric operands (e.g. a UNION arm binding the same var
            // to a plain string) had `numeric_term`/`decimal_term_value`'s `filter_map` quietly
            // drop the non-numeric ones and average just the numeric-parseable SUBSET — a real
            // `=_bag` wrong answer per SPARQL §11 (Avg via Sum: ANY non-numeric operand errors
            // the whole aggregate, spareval-confirmed), previously tracked as a deliberate,
            // separate residue (ADR-0025 progress log, 2026-07-18 addendum). SUM never had this
            // gap; AVG's two branches independently repeat the mistake.
            if vals.iter().any(|t| is_xsd_double(t)) {
                let nums: Vec<f64> = vals.iter().filter_map(|t| numeric_term(t)).collect();
                if nums.len() < vals.len() {
                    return Ok(None); // non-numeric operand ⇒ UNBOUND (type error, §11)
                }
                let avg = nums.iter().sum::<f64>() / nums.len() as f64;
                Ok(Some(double_term(avg)?))
            } else {
                // M3 fix 1: every remaining operand is xsd:integer/xsd:decimal (the
                // xsd:double case already returned above), so accumulate with
                // `oxsdatatypes::Decimal` — exact i128 fixed-point, NEVER `f64` — instead of
                // the old `nums.iter().sum::<f64>() / len`. A non-terminating quotient (e.g.
                // 11/3) rendered as an f64 artifact ("3.6666666666666665") that diverged from
                // the spareval oracle's own exact decimal AVG ("3.666666666666666666"): same
                // `oxsdatatypes::Decimal` type on both sides ⇒ =_bag equality.
                let nums: Vec<Decimal> =
                    vals.iter().filter_map(|t| decimal_term_value(t)).collect();
                if nums.len() < vals.len() {
                    return Ok(None); // non-numeric operand ⇒ UNBOUND (type error, §11)
                }
                let Some(sum) = nums
                    .iter()
                    .try_fold(Decimal::from(0_i64), |acc, &d| acc.checked_add(d))
                else {
                    return Ok(None); // FOAR0002 overflow ⇒ UNBOUND (never a wrong answer)
                };
                match sum.checked_div(nums.len() as i64) {
                    Some(avg) => Ok(Some(decimal_term_exact(avg)?)),
                    None => Ok(None), // FOAR0001/FOAR0002 ⇒ UNBOUND
                }
            }
        }
        AggKind::Min | AggKind::Max => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            // ADR-0025 C.5 (see SUM): any unbound operand row in a non-empty group ⇒ UNBOUND.
            if !rows.is_empty() && rows.iter().any(|r| r.get(var).is_none()) {
                return Ok(None);
            }
            // NOTE (ADR-0025 C.8): `agg.distinct` is deliberately NOT applied here — deduping
            // the multiset before MIN/MAX cannot change the result (the minimum/maximum of a
            // set equals that of the multiset it came from), unlike SUM/AVG (see
            // `dedup_if_distinct` below), so `MIN(DISTINCT ?v)`/`MAX(DISTINCT ?v)` are already
            // correct without special-casing `distinct`.
            let vals: Vec<&Term> = rows.iter().filter_map(|r| r.get(var)).collect();
            if vals.is_empty() {
                return Ok(None); // UNBOUND for empty multiset (§11)
            }
            let result = if agg.kind == AggKind::Min {
                vals.iter().min_by(|a, b| cmp_term(a, b))
            } else {
                vals.iter().max_by(|a, b| cmp_term(a, b))
            };
            Ok(result.map(|t| (*t).clone()))
        }
    }
}

/// ADR-0025 C.8: dedup a `rust_agg` operand multiset when `SUM(DISTINCT ?v)`/`AVG(DISTINCT
/// ?v)` requires it (SPARQL §11 — DISTINCT reduces the aggregate's input multiset to a SET
/// before applying the set function). `RustAgg.distinct` was previously read only by `Count`;
/// `Sum`/`Avg` silently ignored it and double-counted duplicate rows, a real `=_bag` wrong
/// answer (the SQL-pushdown sibling, `emit.rs`'s `agg_expr_sql`, already renders `SUM(DISTINCT
/// col)` correctly — only this in-process path had the gap). Canonicalises on `Term`'s own
/// `Hash`/`Eq` (structural equality agrees with N-Triples lexical equality, so this is the
/// SAME dedup key the `COUNT(DISTINCT …)` branches above use, just without their `to_string()`
/// allocation — ADR-0024/M4 perf), so dedup is order-independent and consistent across every
/// aggregate. No-op (returns `vals` unchanged) when `distinct` is false.
fn dedup_if_distinct(vals: Vec<&Term>, distinct: bool) -> Vec<&Term> {
    if !distinct {
        return vals;
    }
    let mut seen: std::collections::HashSet<&Term> = std::collections::HashSet::new();
    vals.into_iter().filter(|t| seen.insert(*t)).collect()
}

/// Extract the `f64` numeric value of an RDF term (returns `None` for
/// non-numeric-typed literals and non-literals).
fn numeric_term(t: &Term) -> Option<f64> {
    match t {
        Term::Literal(l) => numeric_value(l),
        _ => None,
    }
}

/// Extract the EXACT `oxsdatatypes::Decimal` value of an RDF term (M3 fix 1):
/// the same "numeric literal" gate as [`numeric_term`] (so a non-numeric operand
/// is rejected identically), but parsed WITHOUT ever going through `f64` —
/// `Decimal::from_str` reads the literal's own lexical digits directly, so a
/// non-terminating AVG quotient stays exact end to end. Only reached once the
/// `xsd:double`/`xsd:float` case has already been handled elsewhere, so the
/// lexical form here is always plain-digit (never `E`-notation).
fn decimal_term_value(t: &Term) -> Option<Decimal> {
    match t {
        Term::Literal(l) if numeric_value(l).is_some() => l.value().parse().ok(),
        _ => None,
    }
}

/// Whether an RDF term is an `xsd:integer`-typed literal.
fn is_xsd_integer(t: &Term) -> bool {
    match t {
        Term::Literal(l) => l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#integer",
        _ => false,
    }
}

/// Whether an RDF term is an `xsd:double` (or `xsd:float`, which this codebase folds into
/// double) literal — the promotion signal for SUM/AVG result typing (SPARQL §11.4 / XPath
/// numeric type promotion: any `double` operand makes the aggregate result `double`).
fn is_xsd_double(t: &Term) -> bool {
    match t {
        Term::Literal(l) => matches!(
            l.datatype().as_str(),
            "http://www.w3.org/2001/XMLSchema#double" | "http://www.w3.org/2001/XMLSchema#float"
        ),
        _ => false,
    }
}

/// Build a canonical `xsd:double` literal from an `f64` (via the shared canonicaliser — the
/// oracle's `oxsdatatypes` library — so the lexical form matches, e.g. `1.0E1`).
fn double_term(n: f64) -> Result<Term> {
    natural_literal(&format!("{n}"), XsdTypeCode::Double)
}

/// Build an `xsd:integer` literal from an `i64`.
fn integer_term(n: i64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        n.to_string(),
        sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
    ))
}

/// Build an `xsd:decimal` literal from an `f64`.
fn decimal_term(n: f64) -> Result<Term> {
    // Compact (non-scientific) representation, then run it through the shared XSD-decimal
    // canonicaliser (`oxsdatatypes::Decimal`, the SAME library the oxigraph oracle uses) so
    // the lexical form is canonical and oracle-matching: an integral value renders "30", not
    // "30.0" (RDF term equality is by lexical form, so "30.0"^^decimal ≠ "30"^^decimal would
    // be a =_bag divergence), and trailing zeros trim ("1.50" → "1.5"). Consistent with the
    // SUM / `natural_literal` reconstruction path.
    let raw = if n.fract() == 0.0 {
        format!("{n:.1}")
    } else {
        format!("{n}")
    };
    natural_literal(&raw, XsdTypeCode::Decimal)
}

/// Build an `xsd:decimal` literal from an EXACT `oxsdatatypes::Decimal` (M3 fix 1's
/// AVG accumulator — never route through `f64`, which cannot represent a non-terminating
/// quotient like 11/3 exactly). `Decimal`'s own `Display` is ALREADY XSD-canonical
/// (`sf-core/datatype.rs` module doc), so this round-trips through `natural_literal`
/// exactly like [`decimal_term`]/[`double_term`], for consistency, not because
/// canonicalisation is needed here.
fn decimal_term_exact(d: Decimal) -> Result<Term> {
    natural_literal(&d.to_string(), XsdTypeCode::Decimal)
}
use std::sync::Arc;

use oxsdatatypes::Decimal;
use sf_core::datatype::XsdTypeCode;
use sf_core::{Literal, Term};

use crate::iq::{AggKind, RustAgg, RustGroup};
use crate::{Plan, Result};

use super::expression::eval_expr;
use super::order::{
    cmp_term, numeric_value, order_cmp_precomputed, precompute_order_keys, TermSortKey,
};
use super::row::{canonical_pairs, natural_literal, Bindings};

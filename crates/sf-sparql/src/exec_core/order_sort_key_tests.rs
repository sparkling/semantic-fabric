//! Locks the Schwartzian-transform ORDER BY refactor (`TermSortKey` /
//! `cmp_sort_key` / `order_cmp_precomputed`): a mixed vector of every term
//! kind — IRIs, literals, blank nodes, quoted triples (RDF-star; only same-kind
//! pairs of THESE ever reach the allocating tie-break) — sorts IDENTICALLY
//! through the reference [`cmp_term`] (which allocates a fallback string per
//! comparison it needs one) and the new precomputed path (which allocates it
//! once per term, ever).
use std::cmp::Ordering;
use std::sync::Arc;

use sf_core::{BlankNode, Literal, NamedNode, Term, Triple};

use crate::iq::OrderKey;

use super::order::{
    cmp_sort_key, cmp_term, compact_to_window, order_cmp_precomputed, precompute_order_keys,
    sorted_indices, term_sort_key, TermSortKey,
};
use super::row::Bindings;

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s))
}
fn lit(s: &str) -> Term {
    Term::Literal(Literal::new_simple_literal(s))
}
fn typed(value: &str, local: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(
        value,
        NamedNode::new_unchecked(format!("http://www.w3.org/2001/XMLSchema#{local}")),
    ))
}
fn bnode(s: &str) -> Term {
    Term::BlankNode(BlankNode::new_unchecked(s))
}
fn triple(s: &str) -> Term {
    Term::Triple(Box::new(Triple::new(
        NamedNode::new_unchecked(s),
        NamedNode::new_unchecked("http://ex.org/p"),
        NamedNode::new_unchecked("http://ex.org/o"),
    )))
}

fn mixed_terms() -> Vec<Term> {
    vec![
        iri("http://b.example/2"),
        lit("zzz"),
        bnode("b2"),
        triple("http://s/2"),
        iri("http://a.example/1"),
        lit("aaa"),
        bnode("b1"),
        triple("http://s/1"),
        triple("http://s/1"), // duplicate — exercises stability
        lit("aaa"),           // duplicate literal
    ]
}

#[test]
fn precomputed_sort_matches_cmp_term_reference() {
    let terms = mixed_terms();

    // Reference: the original per-comparison comparator, unchanged.
    let mut via_cmp_term = terms.clone();
    via_cmp_term.sort_by(cmp_term);

    // New: precompute each term's sort key ONCE, then sort via the keys.
    let keys: Vec<TermSortKey> = terms.iter().map(term_sort_key).collect();
    let mut idx: Vec<usize> = (0..terms.len()).collect();
    idx.sort_by(|&i, &j| cmp_sort_key(&keys[i], &keys[j]));
    let via_precomputed: Vec<Term> = idx.into_iter().map(|i| terms[i].clone()).collect();

    assert_eq!(via_cmp_term, via_precomputed);
}

/// The same equivalence one layer up, at [`order_cmp_precomputed`] — the
/// actual multi-row, `OrderKey`-driven machinery `run_branches` /
/// `rust_group_result_rows` call — including an UNBOUND row (no "v" binding),
/// exercising the `None`-placement arms `cmp_sort_key` alone doesn't cover.
/// The reference comparator is `order_cmp`'s original body, reimplemented
/// here directly over the unchanged [`cmp_term`] — `order_cmp` itself was
/// superseded (both its call sites now use the precomputed path) and removed,
/// so this stands in for it per the fix's own "reimplement the old comparator
/// in the test" instruction.
#[test]
fn order_cmp_precomputed_matches_reference_over_solutions() {
    let order = vec![OrderKey {
        var: "v".to_owned(),
        descending: false,
        expr: None,
    }];
    let mut rows: Vec<Bindings> = mixed_terms()
        .into_iter()
        .map(|t| {
            let mut m = Bindings::new();
            m.insert(Arc::from("v"), t);
            m
        })
        .collect();
    rows.push(Bindings::new()); // UNBOUND — no "v" key

    let reference_cmp = |a: &Bindings, b: &Bindings| {
        for key in &order {
            let ord = match (a.get(&key.var), b.get(&key.var)) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Less,
                (Some(_), None) => Ordering::Greater,
                (Some(x), Some(y)) => cmp_term(x, y),
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    };
    let mut via_reference = rows.clone();
    via_reference.sort_by(reference_cmp);

    let keys: Vec<Vec<Option<TermSortKey>>> = rows
        .iter()
        .map(|r| precompute_order_keys(&order, r))
        .collect();
    let mut idx: Vec<usize> = (0..rows.len()).collect();
    idx.sort_by(|&i, &j| order_cmp_precomputed(&order, &keys[i], &keys[j]));
    let via_precomputed: Vec<Bindings> = idx.into_iter().map(|i| rows[i].clone()).collect();

    assert_eq!(via_reference, via_precomputed);
}

#[test]
fn repeated_window_compaction_matches_one_stable_full_sort_at_tie_boundary() {
    let order = vec![OrderKey {
        var: "v".to_owned(),
        descending: false,
        expr: None,
    }];
    let mut rows = Vec::new();
    for (branch, value) in ["b", "a", "a", "c", "a", "b", "a"].into_iter().enumerate() {
        let mut bindings = Bindings::new();
        bindings.insert(Arc::from("v"), lit(value));
        rows.push((branch, bindings));
    }
    let full_order = sorted_indices(&rows, &order);
    let expected: Vec<usize> = full_order.into_iter().take(3).map(|i| rows[i].0).collect();

    let mut bounded = Vec::new();
    for chunk in rows.chunks(2) {
        bounded.extend_from_slice(chunk);
        compact_to_window(&mut bounded, &order, 3);
    }
    assert_eq!(
        bounded
            .iter()
            .map(|(branch, _)| *branch)
            .collect::<Vec<_>>(),
        expected,
        "the earliest arrivals inside the equal-key boundary must survive"
    );
}

#[test]
fn numeric_order_does_not_alias_values_beyond_f64_integer_precision() {
    let lower = typed("9007199254740992", "integer");
    let higher = typed("9007199254740993", "integer");
    assert_eq!(cmp_term(&lower, &higher), Ordering::Less);
    assert_eq!(cmp_term(&higher, &lower), Ordering::Greater);
}

#[test]
fn numeric_order_keeps_arbitrary_precision_decimal_fraction_digits() {
    let lower = typed("0.1000000000000000000000000000000000001", "decimal");
    let higher = typed("0.1000000000000000000000000000000000002", "decimal");
    assert_eq!(cmp_term(&lower, &higher), Ordering::Less);
}

#[test]
fn numeric_nan_has_a_deterministic_total_extension() {
    let number = typed("1", "integer");
    let nan = typed("NaN", "double");
    assert_eq!(cmp_term(&number, &nan), Ordering::Less);
    assert_eq!(cmp_term(&nan, &number), Ordering::Greater);
    assert_eq!(cmp_term(&nan, &nan), Ordering::Equal);
}

#[test]
fn typed_literal_values_do_not_fall_back_to_lexical_order() {
    assert_eq!(
        cmp_term(&typed("false", "boolean"), &typed("1", "boolean")),
        Ordering::Less
    );
    assert_eq!(
        cmp_term(
            &typed("2020-01-01T00:30:00+01:00", "dateTime"),
            &typed("2020-01-01T00:00:00Z", "dateTime"),
        ),
        Ordering::Less
    );
    assert_eq!(
        cmp_term(&typed("P2D", "duration"), &typed("P10D", "duration")),
        Ordering::Less
    );
}

fn value_domain_terms() -> Vec<Term> {
    vec![
        typed("false", "boolean"),
        typed("1", "boolean"),
        typed("-INF", "double"),
        typed("1.4E-45", "float"),
        typed("5E-324", "double"),
        typed("-0", "float"),
        typed("0.0000000000000000001", "decimal"),
        typed("16777216.5", "decimal"),
        typed("16777216", "float"),
        typed("9007199254740993", "integer"),
        typed("INF", "double"),
        typed("NaN", "float"),
        typed("2019-12-31T23:30:00Z", "dateTime"),
        typed("2020-01-01T00:30:00+01:00", "dateTimeStamp"),
        typed("2020-01-01", "date"),
        typed("2019-12-31-01:00", "date"),
        typed("00:30:00+01:00", "time"),
        typed("23:30:00Z", "time"),
        typed("P1M", "yearMonthDuration"),
        typed("P30D", "dayTimeDuration"),
        typed("P1M1D", "duration"),
        lit("15"),
        typed("not-a-number", "integer"),
    ]
}

#[test]
fn literal_order_is_antisymmetric_and_transitive_across_value_domains() {
    let terms = value_domain_terms();
    for left in &terms {
        for right in &terms {
            assert_eq!(
                cmp_term(left, right),
                cmp_term(right, left).reverse(),
                "antisymmetry failed for {left} and {right}"
            );
            for third in &terms {
                if cmp_term(left, right) != Ordering::Greater
                    && cmp_term(right, third) != Ordering::Greater
                {
                    assert_ne!(
                        cmp_term(left, third),
                        Ordering::Greater,
                        "transitivity failed for {left}, {right}, {third}"
                    );
                }
            }
        }
    }
}

#[test]
fn repeated_window_compaction_matches_full_sort_for_mixed_value_domains() {
    let order = vec![OrderKey {
        var: "v".to_owned(),
        descending: true,
        expr: None,
    }];
    let rows: Vec<(usize, Bindings)> = value_domain_terms()
        .into_iter()
        .enumerate()
        .map(|(arrival, term)| {
            let mut bindings = Bindings::new();
            bindings.insert(Arc::from("v"), term);
            (arrival, bindings)
        })
        .collect();
    let expected: Vec<usize> = sorted_indices(&rows, &order)
        .into_iter()
        .take(7)
        .map(|index| rows[index].0)
        .collect();

    for chunk_size in 1..=rows.len() {
        let mut bounded = Vec::new();
        for chunk in rows.chunks(chunk_size) {
            bounded.extend_from_slice(chunk);
            compact_to_window(&mut bounded, &order, 7);
        }
        assert_eq!(
            bounded
                .iter()
                .map(|(arrival, _)| *arrival)
                .collect::<Vec<_>>(),
            expected,
            "chunk size {chunk_size}"
        );
    }
}

#[test]
fn calendar_offsets_with_the_same_value_compare_equal() {
    for (local, left, right) in [
        (
            "dateTime",
            "2020-01-01T00:00:00Z",
            "2020-01-01T01:00:00+01:00",
        ),
        ("date", "2020-01-01Z", "2020-01-01+00:00"),
        ("time", "00:00:00Z", "01:00:00+01:00"),
    ] {
        assert_eq!(
            cmp_term(&typed(left, local), &typed(right, local)),
            Ordering::Equal,
            "{local}"
        );
    }
}

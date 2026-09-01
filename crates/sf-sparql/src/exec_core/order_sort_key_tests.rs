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
    cmp_sort_key, cmp_term, order_cmp_precomputed, precompute_order_keys, term_sort_key,
    TermSortKey,
};
use super::row::Bindings;

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s))
}
fn lit(s: &str) -> Term {
    Term::Literal(Literal::new_simple_literal(s))
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

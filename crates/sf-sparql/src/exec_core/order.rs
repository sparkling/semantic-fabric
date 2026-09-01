//! SPARQL term ordering and precomputed ORDER BY keys.

/// SPARQL term order extended to a total order for sorting: blank node < IRI <
/// literal; within a kind by value.
pub(super) fn cmp_term(a: &Term, b: &Term) -> Ordering {
    match (a, b) {
        (Term::BlankNode(x), Term::BlankNode(y)) => x.as_str().cmp(y.as_str()),
        (Term::NamedNode(x), Term::NamedNode(y)) => x.as_str().cmp(y.as_str()),
        (Term::Literal(x), Term::Literal(y)) => cmp_literal(x, y),
        _ => term_rank(a)
            .cmp(&term_rank(b))
            .then_with(|| a.to_string().cmp(&b.to_string())),
    }
}

/// [`cmp_term`]'s kind ordering, factored out so [`term_sort_key`] shares it.
fn term_rank(t: &Term) -> u8 {
    match t {
        Term::BlankNode(_) => 0,
        Term::NamedNode(_) => 1,
        Term::Literal(_) => 2,
        // Quoted triple (RDF-star / ADR-0032 D2's `Term::Triple`, including a
        // reconstructed `TermDef::ComposedTriple`) — SPARQL §15.1: triple
        // terms are the HIGHEST category, and order AMONG them is spec-
        // undefined. This engine's choice — sort last (this rank), by lexical
        // form (`cmp_term`'s wildcard tie-break / `TermSortKey::Other`) — is
        // therefore a PERMISSIBLE, merely DETERMINISTIC one, not a spec
        // requirement: ordering AMONG values sharing this rank is by the
        // triple's own `Display` (N-Triples-like) text, stable and repeatable
        // across runs (no hashing / no non-deterministic input anywhere in
        // this comparison), never mixed with a non-triple-term rank (a
        // `TermDef::ComposedTriple`-composed variable and an ordinary
        // variable can never land in the same result column — the
        // uniform-composedness law, `star::rewrite_union`'s doc comment — so
        // "highest category" never needs to interleave with another kind's
        // ordering here). See `differential_star.rs`'s
        // `order_by_composed_var_is_deterministic_across_runs` for the
        // end-to-end proof over a REAL env-composed `?t`.
        _ => 3,
    }
}

/// A [`Term`]'s [`cmp_term`]-relevant shape, precomputed ONCE per term rather than
/// re-derived on every comparison a sort makes (Schwartzian transform, ADR-0024/M4
/// perf). `BlankNode`/`NamedNode` borrow their `&str`; `Literal` borrows the whole
/// literal (its own comparison, `cmp_literal`, is already allocation-free). `Other`
/// (any kind besides those three — currently only a quoted triple, RDF-star) is the
/// ONLY variant that allocates, and does so HERE, once, instead of inside
/// `cmp_term`'s wildcard tie-break on every comparison it participates in.
pub(super) enum TermSortKey<'a> {
    BlankNode(&'a str),
    NamedNode(&'a str),
    Literal(&'a Literal),
    Other(String),
}

/// Build a term's [`TermSortKey`]. See its doc comment for why this is where the
/// (possibly) allocating work happens.
pub(super) fn term_sort_key(t: &Term) -> TermSortKey<'_> {
    match t {
        Term::BlankNode(n) => TermSortKey::BlankNode(n.as_str()),
        Term::NamedNode(n) => TermSortKey::NamedNode(n.as_str()),
        Term::Literal(l) => TermSortKey::Literal(l),
        other => TermSortKey::Other(other.to_string()),
    }
}

/// [`cmp_term`], comparing precomputed [`TermSortKey`]s instead of the `Term`s
/// directly. Byte-identical order to `cmp_term` by construction: the SAME three
/// same-kind arms (borrowed, not cloned data), and the SAME rank-then-lexical
/// wildcard tie-break — `term_rank` assigns each kind a UNIQUE rank except for
/// `Other`, so two keys only ever reach the tie-break when BOTH are `Other`
/// (whatever concrete non-Blank/Named/Literal `Term` variant they came from),
/// exactly the one case `cmp_term`'s own wildcard allocates for.
pub(super) fn cmp_sort_key(a: &TermSortKey, b: &TermSortKey) -> Ordering {
    fn rank(k: &TermSortKey) -> u8 {
        match k {
            TermSortKey::BlankNode(_) => 0,
            TermSortKey::NamedNode(_) => 1,
            TermSortKey::Literal(_) => 2,
            TermSortKey::Other(_) => 3,
        }
    }
    match (a, b) {
        (TermSortKey::BlankNode(x), TermSortKey::BlankNode(y)) => x.cmp(y),
        (TermSortKey::NamedNode(x), TermSortKey::NamedNode(y)) => x.cmp(y),
        (TermSortKey::Literal(x), TermSortKey::Literal(y)) => cmp_literal(x, y),
        _ => rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
            (TermSortKey::Other(x), TermSortKey::Other(y)) => x.cmp(y),
            // Same rank implies the same variant among Blank/Named/Literal/Other
            // (each of the first three has a UNIQUE rank, handled above), so a
            // tie here is only ever reached by two `Other`s.
            _ => unreachable!("equal TermSortKey rank implies both are Other"),
        }),
    }
}

/// Precompute one row's ORDER BY sort keys — one [`TermSortKey`] per [`OrderKey`]
/// in `order`, `None` for an unbound one — see [`order_cmp_precomputed`].
pub(super) fn precompute_order_keys<'a>(
    order: &[OrderKey],
    bindings: &'a Bindings,
) -> Vec<Option<TermSortKey<'a>>> {
    order
        .iter()
        .map(|key| bindings.get(&key.var).map(term_sort_key))
        .collect()
}

/// Compare two solutions' PRECOMPUTED ORDER BY keys ([`precompute_order_keys`],
/// SPARQL §15.1), honoring each key's direction with explicit UNBOUND placement:
/// an unbound key sorts FIRST for ASC and LAST for DESC — matching the SQL `NULLS
/// FIRST/LAST` the single-branch path emits, so single- and multi-branch orderings
/// agree. Bound terms order blank-node < IRI < literal; numeric-typed literals
/// compare by value (so xsd:integer 2 < 10, not lexical "10" < "2") — see
/// [`cmp_sort_key`]. Used by the buffered ORDER BY sorts
/// ([`run_branches`]/[`rust_group_result_rows`]) with keys precomputed once per
/// row (a Schwartzian transform, ADR-0024/M4 perf), so an `n`-row sort computes
/// each term's (possibly allocating) fallback string O(n) times, not O(n log n).
pub(super) fn order_cmp_precomputed(
    order: &[OrderKey],
    a: &[Option<TermSortKey>],
    b: &[Option<TermSortKey>],
) -> Ordering {
    for ((key, ka), kb) in order.iter().zip(a.iter()).zip(b.iter()) {
        let ord = match (ka, kb) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => {
                if key.descending {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (Some(_), None) => {
                if key.descending {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (Some(x), Some(y)) => {
                let c = cmp_sort_key(x, y);
                if key.descending {
                    c.reverse()
                } else {
                    c
                }
            }
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// Compare two literals: numerically when both carry a numeric XSD datatype, else
/// by lexical value, then datatype IRI, then language tag.
fn cmp_literal(x: &Literal, y: &Literal) -> Ordering {
    if let (Some(nx), Some(ny)) = (numeric_value(x), numeric_value(y)) {
        return nx.partial_cmp(&ny).unwrap_or(Ordering::Equal);
    }
    x.value()
        .cmp(y.value())
        .then_with(|| x.datatype().as_str().cmp(y.datatype().as_str()))
        .then_with(|| x.language().unwrap_or("").cmp(y.language().unwrap_or("")))
}

/// The `f64` value of a numeric-XSD-typed literal, else `None` (a non-numeric
/// datatype is ordered lexically, never coerced).
pub(super) fn numeric_value(l: &Literal) -> Option<f64> {
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
    let local = l.datatype().as_str().strip_prefix(XSD)?;
    let numeric = matches!(
        local,
        "integer"
            | "decimal"
            | "double"
            | "float"
            | "long"
            | "int"
            | "short"
            | "byte"
            | "nonNegativeInteger"
            | "nonPositiveInteger"
            | "negativeInteger"
            | "positiveInteger"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
    );
    if numeric {
        l.value().parse::<f64>().ok()
    } else {
        None
    }
}
use std::cmp::Ordering;

use sf_core::{Literal, Term};

use crate::iq::OrderKey;

use super::row::Bindings;

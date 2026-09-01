//! `canonical_pairs` and `Bindings::insert` unit-level locks (Run 5 W5b).
//! The W5 test-soundness audit found NO e2e/differential fixture can force
//! either property to matter: every `Bindings` the production pipeline
//! builds arrives via [`reconstruct`]/[`intern_bindings`], which iterate
//! `Branch::bindings` (a `BTreeMap<String, TermDef>`) — ALREADY
//! alphabetical — so a row's insertion order coincides with
//! `canonical_pairs`'s sorted order on every real input today; the sort is
//! a no-op an integration/differential test cannot distinguish from
//! "absent". Likewise, no query shape re-binds an already-bound variable
//! on the SAME `Bindings` end to end (each var is written once, by
//! exactly one branch column), so `insert`'s replace-on-existing-key arm
//! (`BTreeMap::insert`-compatible, the Wave C1 refactor's own compatibility
//! contract) never actually fires through a real query. Both are locked
//! directly at unit level instead of relying on an e2e fixture that cannot
//! see them.
use std::sync::Arc;

use sf_core::{Literal, Term};

use super::row::{canonical_pairs, Bindings};

fn lit(s: &str) -> Term {
    Term::Literal(Literal::new_simple_literal(s))
}

#[test]
fn canonical_pairs_is_independent_of_insertion_order() {
    // The SAME three (var, term) pairs, inserted in two DIFFERENT
    // non-alphabetical orders — `canonical_pairs` must sort both down to
    // the IDENTICAL var-name-order output.
    let mut forward = Bindings::new();
    forward.insert(Arc::from("b"), lit("2"));
    forward.insert(Arc::from("c"), lit("3"));
    forward.insert(Arc::from("a"), lit("1"));

    let mut reverse = Bindings::new();
    reverse.insert(Arc::from("c"), lit("3"));
    reverse.insert(Arc::from("a"), lit("1"));
    reverse.insert(Arc::from("b"), lit("2"));

    assert_eq!(
        canonical_pairs(&forward),
        canonical_pairs(&reverse),
        "canonical_pairs must be independent of the ORDER the pairs were \
             inserted in — a dedup/COUNT(DISTINCT *) key built from it must \
             treat these as the SAME solution regardless of which branch/order \
             bound them"
    );
    assert_eq!(
        canonical_pairs(&forward),
        vec![("a", &lit("1")), ("b", &lit("2")), ("c", &lit("3"))],
        "canonical order is var-NAME-sorted, not insertion order — pins the \
             actual sort key, not just \"some\" order-independence"
    );
}

#[test]
fn insert_on_an_existing_key_replaces_not_appends() {
    let mut b = Bindings::new();
    b.insert(Arc::from("v"), lit("old"));
    assert_eq!(b.iter().count(), 1);
    assert_eq!(b.get("v"), Some(&lit("old")));

    // Re-insert the SAME key with a DIFFERENT term — `BTreeMap::insert`'s
    // contract (Wave C1's promised compatibility, see `insert`'s own doc
    // comment): the slot is OVERWRITTEN, never appended as a second entry
    // for the same var.
    b.insert(Arc::from("v"), lit("new"));
    assert_eq!(
        b.iter().count(),
        1,
        "re-inserting an existing key must NOT grow the binding count"
    );
    assert_eq!(
        b.get("v"),
        Some(&lit("new")),
        "re-inserting an existing key must overwrite the old value"
    );
}

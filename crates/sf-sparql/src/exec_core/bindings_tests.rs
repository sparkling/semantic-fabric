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

#[test]
fn execution_column_validation_includes_borrowed_hidden_keys() {
    use crate::emit::{validate_execution_columns, BindingView, ColumnCatalog};
    use crate::iq::{Branch, Scan, TermDef};
    use sf_core::ir::{LogicalSource, TermMap, TermSpec};
    use sf_sql::{source_work::SourceWork, Dialect};
    let source = LogicalSource::Table("hidden_key_source".into());
    let branch = Branch::single(Scan {
        alias: 0,
        source: source.clone().into(),
    });
    let scope = crate::DedupScope {
        group_id: 0,
        key_bindings: [(
            "hidden".into(),
            TermDef::Derived {
                alias: 0,
                term_map: TermMap::Column("missing".into(), TermSpec::iri()),
            },
        )]
        .into(),
    };
    let work = SourceWork::new(None);
    let mut catalog = ColumnCatalog::default();
    catalog.insert(&source, vec!["present".into()]);
    let view = BindingView::merged(&branch.bindings, Some(&scope.key_bindings), work).unwrap();
    assert_eq!(view.len(), 1);
    let run = |catalog: &ColumnCatalog| {
        validate_execution_columns(
            std::slice::from_ref(&branch),
            &[Some(scope.clone())],
            Dialect::Sqlite,
            catalog,
            work,
        )
    };
    assert!(run(&catalog).unwrap_err().to_string().contains("missing"));
    catalog.insert(&source, vec!["missing".into()]);
    run(&catalog).unwrap();
    assert!(branch.bindings.is_empty());
}

fn lit(s: &str) -> Term {
    Term::Literal(Literal::new_simple_literal(s))
}

#[test]
fn root_emission_charges_projection_and_metadata_twin_to_source_control() {
    use crate::emit::{emit_branch_controlled, BranchModifiers, ColumnCatalog};
    use crate::iq::{Branch, TermDef};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::{source_work::SourceWork, Dialect};
    let mut branch = Branch::empty();
    branch
        .bindings
        .insert("v".into(), TermDef::Const(lit("value")));
    let budget = |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
    let mut costs = Vec::new();
    for dialect in [Dialect::Postgres, Dialect::Sqlite] {
        let run = |control: &QueryBudget| {
            emit_branch_controlled(
                &branch,
                dialect,
                &ColumnCatalog::default(),
                BranchModifiers::stored(&branch),
                SourceWork::new(Some(control)),
            )
        };
        let measured = budget(u64::MAX);
        let expected = run(&measured).unwrap();
        let units = measured.consumed(QueryCharge::SourceWork);
        assert!(units > 0);
        assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
        assert_eq!(run(&budget(units)).unwrap().sql, expected.sql);
        assert!(matches!(
            run(&budget(units - 1)),
            Err(crate::Error::QueryControl(
                QueryControlError::SourceWorkExceeded
            ))
        ));
        costs.push(units);
    }
    assert_eq!(costs[1], costs[0] * 2);
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

#[cfg(test)]
mod equality_tests {
    use crate::exec_core::driver::source_prepare::recipes_same;
    use crate::iq::{AggKind, ColRef, R2rmlGraphScope, TermDef};
    use sf_core::ir::{TermMap, TermSpec};
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };
    use sf_core::{Literal, NamedNode, Term};
    use sf_sql::source_work::SourceWork;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn budget(units: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
    }
    fn leaf() -> TermDef {
        TermDef::Const(Literal::new_simple_literal("α").into())
    }
    fn fixtures() -> Vec<TermDef> {
        let mut out = vec![
            leaf(),
            TermDef::Const(NamedNode::new("urn:x").unwrap().into()),
            TermDef::Concat(vec![]),
            TermDef::Concat(vec![leaf()]),
            TermDef::Coalesce(Box::new(leaf()), Box::new(leaf())),
            TermDef::ComposedTriple {
                subject: Box::new(leaf()),
                predicate: Box::new(leaf()),
                object: Box::new(leaf()),
            },
        ];
        for alias in [0, 1] {
            for name in ["x", "y"] {
                let map = TermMap::Column(name.into(), TermSpec::iri());
                out.push(TermDef::Derived {
                    alias,
                    term_map: map.clone(),
                });
                for graph in [
                    R2rmlGraphScope::Default,
                    R2rmlGraphScope::Mapped {
                        alias,
                        term_map: map.clone(),
                    },
                ] {
                    out.push(TermDef::R2rmlBlank {
                        alias,
                        term_map: map.clone(),
                        graph,
                    });
                }
                for kind in [
                    AggKind::Count,
                    AggKind::Sum,
                    AggKind::Avg,
                    AggKind::Min,
                    AggKind::Max,
                ] {
                    for operand in [None, Some(ColRef::new(alias, name))] {
                        out.push(TermDef::Agg {
                            col: ColRef::new(alias, name),
                            kind,
                            operand,
                            fixed_type: None,
                        });
                    }
                }
            }
        }
        out
    }
    #[test]
    fn recipe_equality_matches_debug_oracle_and_exact_work() {
        for a in fixtures() {
            for b in fixtures() {
                let expected = format!("{a:?}") == format!("{b:?}");
                let measured = budget(u64::MAX);
                assert_eq!(
                    recipes_same(&a, &b, SourceWork::new(Some(&measured))).unwrap(),
                    expected
                );
                let total = measured.consumed(QueryCharge::SourceWork);
                assert_eq!(
                    recipes_same(&a, &b, SourceWork::new(Some(&budget(total)))).unwrap(),
                    expected
                );
                assert!(matches!(
                    recipes_same(&a, &b, SourceWork::new(Some(&budget(total - 1)))),
                    Err(crate::Error::QueryControl(
                        QueryControlError::SourceWorkExceeded
                    ))
                ));
            }
        }
    }
    #[test]
    fn borrowed_remap_matches_allocating_oracle_and_exact_work() {
        use crate::exec_core::driver::source_prepare::recipes_remapped_same;
        let mut values = fixtures();
        values.push(TermDef::Derived {
            alias: 9,
            term_map: TermMap::Template(
                sf_core::ir::Template::parse("urn:{a}/{b}/{a}").unwrap(),
                TermSpec::iri(),
            ),
        });
        for inner in values {
            let projection = inner.columns();
            let outer = crate::iq::lower::remap_termdef(&inner, &projection, 77).unwrap();
            let measured = budget(u64::MAX);
            assert!(recipes_remapped_same(
                &outer,
                &inner,
                &projection,
                77,
                SourceWork::new(Some(&measured))
            )
            .unwrap());
            let units = measured.consumed(QueryCharge::SourceWork);
            assert!(recipes_remapped_same(
                &outer,
                &inner,
                &projection,
                77,
                SourceWork::new(Some(&budget(units)))
            )
            .unwrap());
            assert!(matches!(
                recipes_remapped_same(
                    &outer,
                    &inner,
                    &projection,
                    77,
                    SourceWork::new(Some(&budget(units - 1)))
                ),
                Err(crate::Error::QueryControl(
                    QueryControlError::SourceWorkExceeded
                ))
            ));
            for wrong in fixtures() {
                assert_eq!(
                    recipes_remapped_same(&wrong, &inner, &projection, 77, SourceWork::new(None))
                        .unwrap(),
                    format!("{wrong:?}") == format!("{outer:?}")
                );
            }
        }
    }
    struct Stop {
        budget: QueryBudget,
        calls: AtomicUsize,
        at: usize,
        cause: QueryControlError,
    }
    #[test]
    fn borrowed_remap_validates_later_references_before_recipe_mismatch() {
        use crate::exec_core::driver::source_prepare::recipes_remapped_same;
        for tail in [
            TermDef::Derived {
                alias: 3,
                term_map: TermMap::Column("bad".into(), TermSpec::iri()),
            },
            TermDef::Derived {
                alias: 3,
                term_map: TermMap::Template(
                    sf_core::ir::Template::parse("{bad}").unwrap(),
                    TermSpec::iri(),
                ),
            },
            TermDef::Agg {
                col: ColRef::new(3, "bad"),
                kind: AggKind::Avg,
                operand: None,
                fixed_type: None,
            },
        ] {
            let inner = TermDef::Concat(vec![leaf(), tail]);
            let raw = crate::iq::lower::remap_termdef(&inner, &[], 7)
                .unwrap_err()
                .to_string();
            let actual = recipes_remapped_same(&leaf(), &inner, &[], 7, SourceWork::new(None))
                .unwrap_err()
                .to_string();
            assert_eq!(actual, raw);
        }
    }
    impl QueryControl for Stop {
        fn checkpoint(&self) -> Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(&self, kind: QueryCharge, units: u64) -> Result<(), QueryControlError> {
            self.budget.consume(kind, units)?;
            if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
                self.budget.terminate(self.cause);
            }
            Ok(())
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.budget.terminate(cause)
        }
    }
    #[test]
    fn recipe_equality_observes_every_cancellation_boundary() {
        let a = TermDef::Concat(fixtures());
        let count = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        assert!(recipes_same(&a, &a, SourceWork::new(Some(&count))).unwrap());
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=count.calls.load(Ordering::Relaxed) {
                let stop = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                assert!(matches!(recipes_same(&a, &a, SourceWork::new(Some(&stop))),
                    Err(crate::Error::QueryControl(actual)) if actual == cause));
            }
        }
    }
    #[test]
    fn borrowed_remap_observes_every_cancellation_boundary() {
        use crate::exec_core::driver::source_prepare::recipes_remapped_same;
        let inner = TermDef::Concat(fixtures());
        let projection = inner.columns();
        let outer = crate::iq::lower::remap_termdef(&inner, &projection, 7).unwrap();
        let count = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        assert!(recipes_remapped_same(
            &outer,
            &inner,
            &projection,
            7,
            SourceWork::new(Some(&count))
        )
        .unwrap());
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=count.calls.load(Ordering::Relaxed) {
                let stop = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                assert!(
                    matches!(recipes_remapped_same(&outer, &inner, &projection, 7, SourceWork::new(Some(&stop))),
                    Err(crate::Error::QueryControl(actual)) if actual == cause)
                );
            }
        }
    }
    #[test]
    fn recipe_equality_handles_deep_composition_on_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut a = leaf();
                let mut term: Term = Literal::new_simple_literal("x").into();
                for _ in 0..4096 {
                    a = TermDef::ComposedTriple {
                        subject: Box::new(leaf()),
                        predicate: Box::new(leaf()),
                        object: Box::new(a),
                    };
                    term = sf_core::Triple::new(
                        NamedNode::new("urn:s").unwrap(),
                        NamedNode::new("urn:p").unwrap(),
                        term,
                    )
                    .into();
                }
                let rdf = TermDef::Const(term);
                for value in [&a, &rdf] {
                    assert!(
                        crate::exec_core::driver::source_prepare::recipes_remapped_same(
                            value,
                            value,
                            &[],
                            0,
                            SourceWork::new(Some(&budget(u64::MAX)))
                        )
                        .unwrap()
                    );
                    let measured = budget(u64::MAX);
                    assert!(recipes_same(value, value, SourceWork::new(Some(&measured))).unwrap());
                    let total = measured.consumed(QueryCharge::SourceWork);
                    assert!(
                        recipes_same(value, value, SourceWork::new(Some(&budget(total)))).unwrap()
                    );
                    assert!(
                        recipes_same(value, value, SourceWork::new(Some(&budget(total - 1))))
                            .is_err()
                    );
                }
                // Recursive IQ/RDF destruction is a separate ownership obligation.
                while let TermDef::ComposedTriple { object, .. } = a {
                    a = *object;
                }
                let TermDef::Const(mut term) = rdf else {
                    unreachable!()
                };
                while let Term::Triple(triple) = term {
                    term = triple.object;
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

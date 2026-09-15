use super::*;
use crate::iq::Branch;
use sf_core::ir::TermSpec;

#[test]
fn effective_distinct_projection_matches_prepared_branch_without_mutation() {
    use crate::iq::{Scan, SqlCond};
    use sf_core::ir::LogicalSource;
    use sf_sql::Dialect;
    for stored in [false, true] {
        for effective in [false, true] {
            let mut branch = Branch::single(Scan {
                alias: 0,
                source: LogicalSource::Table("t".into()).into(),
            });
            branch.bindings.insert(
                "v".into(),
                TermDef::Derived {
                    alias: 0,
                    term_map: TermMap::Column("c0".into(), TermSpec::plain_literal()),
                },
            );
            branch.distinct = stored;
            branch
                .where_conds
                .push(SqlCond::IsNull(ColRef::new(0, "guard")));
            let original = format!("{branch:?}");
            let mut prepared = branch.clone();
            prepared.distinct = effective;
            for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
                assert_eq!(
                    crate::emit::projection_layout_with_distinct(&branch, dialect, effective)
                        .unwrap(),
                    crate::emit::projection_layout(&prepared, dialect).unwrap()
                );
            }
            assert_eq!(format!("{branch:?}"), original);
        }
    }
}

#[test]
fn controlled_overlay_decision_preserves_present_and_missing_keys() {
    use super::super::driver::source_prepare::requires_key_overlay;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let budget = |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
    let mut branch = Branch::empty();
    for name in ["α", "z"] {
        branch.bindings.insert(
            name.into(),
            TermDef::Const(Literal::new_simple_literal(name).into()),
        );
    }
    let scopes = vec![
        None,
        Some(crate::DedupScope {
            group_id: 1,
            key_bindings: branch.bindings.clone(),
        }),
    ];
    let mut branches = vec![Branch::empty(), branch];
    for missing in [false, true] {
        if missing {
            branches[1].bindings.remove("α");
        }
        let before = format!("{branches:?}");
        let run = |control: &QueryBudget| {
            requires_key_overlay(&branches, &scopes, SourceWork::new(Some(control)))
        };
        let measured = budget(u64::MAX);
        assert_eq!(run(&measured).unwrap(), missing);
        let units = measured.consumed(QueryCharge::SourceWork);
        assert_eq!(run(&budget(units)).unwrap(), missing);
        assert!(matches!(
            run(&budget(units - 1)),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            let stopped = budget(u64::MAX);
            stopped.terminate(cause);
            assert!(matches!(run(&stopped), Err(Error::QueryControl(actual)) if actual == cause));
        }
        assert_eq!(format!("{branches:?}"), before);
    }
    // Ordered key `z` is missing before `α` collides. A missing-key early
    // return must not bypass the old overlay's later collision rejection.
    branches[1].bindings.clear();
    branches[1].bindings.insert(
        "α".into(),
        TermDef::Const(Literal::new_simple_literal("wrong recipe").into()),
    );
    let before = format!("{branches:?}");
    let run = |control: &QueryBudget| {
        requires_key_overlay(&branches, &scopes, SourceWork::new(Some(control)))
    };
    let measured = budget(u64::MAX);
    assert!(
        matches!(run(&measured), Err(Error::Unsupported(message)) if message.contains("key collides"))
    );
    let units = measured.consumed(QueryCharge::SourceWork);
    assert!(
        matches!(run(&budget(units)), Err(Error::Unsupported(message)) if message.contains("key collides"))
    );
    assert!(matches!(
        run(&budget(units - 1)),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
    assert_eq!(format!("{branches:?}"), before);
}

#[test]
fn controlled_interning_propagates_every_stop_without_mutating_recipes() {
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Stop {
        budget: QueryBudget,
        calls: AtomicUsize,
        at: usize,
        cause: QueryControlError,
    }
    impl QueryControl for Stop {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            kind: QueryCharge,
            units: u64,
        ) -> std::result::Result<(), QueryControlError> {
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
    let stop = |at, cause| Stop {
        budget: QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX)),
        calls: AtomicUsize::new(0),
        at,
        cause,
    };
    let mut b = Branch::empty();
    for name in ["a".into(), "λ".repeat(4096)] {
        b.bindings.insert(
            name,
            TermDef::Const(Literal::new_simple_literal("value").into()),
        );
    }
    let before = format!("{:?}", b.bindings);
    let measured = stop(usize::MAX, QueryControlError::Cancelled);
    intern_bindings_controlled(&b, sf_sql::source_work::SourceWork::new(Some(&measured))).unwrap();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.calls.load(Ordering::Relaxed) {
            let control = stop(at, cause);
            assert!(
                matches!(intern_bindings_controlled(&b,sf_sql::source_work::SourceWork::new(Some(&control))),Err(Error::QueryControl(actual)) if actual==cause)
            );
            assert_eq!(control.budget.checkpoint(), Err(cause));
        }
    }
    assert_eq!(format!("{:?}", b.bindings), before);
    let scopes = [Some(crate::DedupScope {
        group_id: 1,
        key_bindings: b.bindings.clone(),
    })];
    let decision = |control: &Stop| {
        super::super::driver::source_prepare::requires_key_overlay(
            std::slice::from_ref(&b),
            &scopes,
            sf_sql::source_work::SourceWork::new(Some(control)),
        )
    };
    let measured = stop(usize::MAX, QueryControlError::Cancelled);
    assert!(!decision(&measured).unwrap());
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.calls.load(Ordering::Relaxed) {
            let control = stop(at, cause);
            assert!(
                matches!(decision(&control), Err(Error::QueryControl(actual)) if actual == cause)
            );
            assert_eq!(control.budget.checkpoint(), Err(cause));
        }
    }
    let schema: Vec<_> = (0..32)
        .rev()
        .map(|i| ColRef::new(i % 3, format!("key{}", i % 7)))
        .collect();
    let before = schema.clone();
    let measured = stop(usize::MAX, QueryControlError::Cancelled);
    build_col_index_controlled(
        &schema,
        sf_sql::source_work::SourceWork::new(Some(&measured)),
    )
    .unwrap();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.calls.load(Ordering::Relaxed) {
            let control = stop(at, cause);
            assert!(
                matches!(build_col_index_controlled(&schema,sf_sql::source_work::SourceWork::new(Some(&control))),Err(Error::QueryControl(actual)) if actual==cause)
            );
            assert_eq!(control.budget.checkpoint(), Err(cause));
        }
    }
    assert_eq!(schema, before);
}

#[test]
fn controlled_column_index_matches_first_occurrence_for_duplicate_permutations() {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let budget = |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
    let mut schema: Vec<_> = (0..40)
        .map(|i| ColRef::new(i % 3, if i % 2 == 0 { "α" } else { "z" }))
        .collect();
    for _ in 0..schema.len() {
        schema.rotate_left(1);
        let measured = budget(u64::MAX);
        let index = build_col_index_controlled(&schema, SourceWork::new(Some(&measured))).unwrap();
        assert_eq!(index, build_col_index(&schema));
        for alias in 0..4 {
            for name in ["α", "z", "missing"] {
                assert_eq!(
                    col_index_get(&index, alias, name),
                    schema
                        .iter()
                        .position(|c| c.alias == alias && c.column.as_ref() == name)
                );
            }
        }
        let total = measured.consumed(QueryCharge::SourceWork);
        assert_eq!(
            build_col_index_controlled(&schema, SourceWork::new(Some(&budget(total)))).unwrap(),
            index
        );
        assert!(matches!(
            build_col_index_controlled(&schema, SourceWork::new(Some(&budget(total - 1)))),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
    }
    let schema = vec![ColRef::new(0, "key"), ColRef::new(0, "key")];
    let index =
        build_col_index_controlled(&schema, SourceWork::new(Some(&budget(u64::MAX)))).unwrap();
    let row = RawRow {
        values: &[Some("first".into()), Some("second".into())],
        codes: &[Some(XsdTypeCode::String), Some(XsdTypeCode::Integer)],
        index: &index,
    };
    assert_eq!(
        AliasRow {
            raw: &row,
            alias: 0
        }
        .value("key"),
        Some("first")
    );
    assert_eq!(row.code_for(0, "key"), Some(XsdTypeCode::String));
}

#[test]
fn controlled_interning_preserves_order_and_borrowed_recipes_at_exact_work() {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let mut branch = Branch::empty();
    for name in ["z", "α", "a", ""] {
        branch.bindings.insert(
            name.into(),
            TermDef::Const(Literal::new_simple_literal(name).into()),
        );
    }
    let raw = intern_bindings(&branch);
    let budget = |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
    let measured = budget(u64::MAX);
    let run =
        |control: &QueryBudget| intern_bindings_controlled(&branch, SourceWork::new(Some(control)));
    let actual = run(&measured).unwrap();
    for ((a, ad), (b, bd)) in raw.iter().zip(&actual) {
        assert_eq!(a, b);
        assert!(std::ptr::eq(*ad, *bd));
    }
    let units = measured.consumed(QueryCharge::SourceWork);
    assert_eq!(run(&budget(units)).unwrap().len(), raw.len());
    assert!(matches!(
        run(&budget(units - 1)),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
}

#[test]
fn explicit_matching_datatype_uses_the_resolved_natural_constructor() {
    let schema = vec![ColRef::new(0, "value")];
    let index = build_col_index(&schema);
    for (code, lexical, expected) in [
        (XsdTypeCode::Decimal, "+0001.2300", "1.23"),
        (XsdTypeCode::Integer, "+0001", "1"),
        (XsdTypeCode::Boolean, "1", "true"),
        (XsdTypeCode::Double, "1", "1.0E0"),
        (
            XsdTypeCode::DateTime,
            "2024-03-15 00:00:00.120000",
            "2024-03-15T00:00:00.12",
        ),
    ] {
        let values = vec![Some(lexical.to_owned())];
        let codes = vec![Some(code)];
        let raw = RawRow {
            values: &values,
            codes: &codes,
            index: &index,
        };
        let term_map = TermMap::Column(
            "value".into(),
            TermSpec::typed_literal(code.iri().into_owned()),
        );
        let term = derived_term(&term_map, 0, &raw).unwrap().unwrap();
        assert_eq!(
            term,
            Term::Literal(Literal::new_typed_literal(expected, code.iri())),
            "{code:?}"
        );
        let natural = TermMap::Column("value".into(), TermSpec::plain_literal());
        assert_eq!(derived_term(&natural, 0, &raw).unwrap(), Some(term));
    }
}

fn blank(graph: R2rmlGraphScope, graph_value: &str) -> Term {
    let schema = vec![ColRef::new(0, "id"), ColRef::new(0, "graph")];
    let index = build_col_index(&schema);
    let values = vec![Some("shared".to_owned()), Some(graph_value.to_owned())];
    let codes = vec![None, None];
    let raw = RawRow {
        values: &values,
        codes: &codes,
        index: &index,
    };
    build_term(
        &TermDef::R2rmlBlank {
            term_map: TermMap::Column("id".into(), TermSpec::blank_node()),
            alias: 0,
            graph,
        },
        &raw,
    )
    .unwrap()
    .unwrap()
}

#[test]
fn generated_labels_are_injective_over_effective_graph_and_identifier() {
    let default = blank(R2rmlGraphScope::Default, "unused");
    let named = blank(
        R2rmlGraphScope::Mapped {
            term_map: TermMap::Constant(Term::NamedNode(sf_core::NamedNode::new_unchecked(
                "http://ex/g1",
            ))),
            alias: 0,
        },
        "unused",
    );
    let dynamic_named = blank(
        R2rmlGraphScope::Mapped {
            term_map: TermMap::Column("graph".into(), TermSpec::iri()),
            alias: 0,
        },
        "http://ex/g1",
    );
    let dynamic_default = blank(
        R2rmlGraphScope::Mapped {
            term_map: TermMap::Column("graph".into(), TermSpec::iri()),
            alias: 0,
        },
        RR_DEFAULT_GRAPH,
    );

    assert_eq!(named, dynamic_named);
    assert_eq!(default, dynamic_default);
    assert_ne!(default, named);
    assert!(matches!(default, Term::BlankNode(ref b) if b.as_str().starts_with("sfr1d_")));
    assert!(matches!(named, Term::BlankNode(ref b) if b.as_str().starts_with("sfr1n_")));
}

#[test]
fn explicit_rr_datatype_overrides_compatibility_type_for_value_two() {
    let schema = vec![ColRef::new(0, "flag")];
    let index = build_col_index(&schema);
    let values = vec![Some("2".to_owned())];
    let codes = vec![Some(XsdTypeCode::Boolean)];
    let raw = RawRow {
        values: &values,
        codes: &codes,
        index: &index,
    };
    let term_map = TermMap::Column(
        "flag".into(),
        TermSpec::typed_literal(sf_core::NamedNode::from(sf_core::vocab::xsd::INTEGER)),
    );

    let term = derived_term(&term_map, 0, &raw).unwrap().unwrap();
    let Term::Literal(literal) = term else {
        panic!("explicit rr:datatype did not produce a literal");
    };
    assert_eq!(literal.value(), "2");
    assert_eq!(literal.datatype(), sf_core::vocab::xsd::INTEGER);
}

#[test]
fn explicit_rr_datetime_preserves_adapter_canonical_lexical_form() {
    let schema = vec![ColRef::new(0, "observed_at")];
    let index = build_col_index(&schema);
    let values = vec![Some("2024-03-15T00:00:00".to_owned())];
    let codes = vec![Some(XsdTypeCode::DateTime)];
    let raw = RawRow {
        values: &values,
        codes: &codes,
        index: &index,
    };
    let term_map = TermMap::Column(
        "observed_at".into(),
        TermSpec::typed_literal(sf_core::NamedNode::from(sf_core::vocab::xsd::DATE_TIME)),
    );

    let term = derived_term(&term_map, 0, &raw).unwrap().unwrap();
    let Term::Literal(literal) = term else {
        panic!("explicit rr:datatype did not produce a literal");
    };
    assert_eq!(literal.value(), "2024-03-15T00:00:00");
    assert_eq!(literal.datatype(), sf_core::vocab::xsd::DATE_TIME);
}

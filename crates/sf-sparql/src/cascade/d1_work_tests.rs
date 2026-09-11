use super::*;
use crate::compiler_control::CompileContext;
use sf_core::ir::{Template, TermSpec};
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

#[test]
fn d1_rewrite_preserves_keyed_unkeyed_and_noninjective_paths() {
    for keyed in [false, true] {
        for template in ["http://ex/{id}", "{a}{b}"] {
            for dialect in [sf_sql::Dialect::Sqlite, sf_sql::Dialect::Postgres] {
                let mut table = TableSchema::new("input");
                if keyed {
                    table.primary_key = vec!["id".into()];
                }
                let schema = [table];
                let schema = super::super::build_schema_map(&schema);
                let mut source = Branch::single(Scan {
                    alias: 0,
                    source: LogicalSource::Table("input".into()).into(),
                });
                source.bindings.insert(
                    "s".into(),
                    TermDef::Derived {
                        alias: 0,
                        term_map: TermMap::Template(
                            Template::parse(template).unwrap(),
                            TermSpec::iri(),
                        ),
                    },
                );
                let mut expected = source.clone();
                super::super::apply_dup_safety_raw(&mut expected, &schema, dialect);
                let run = |units| {
                    let budget =
                        QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
                    let mut actual = source.clone();
                    let result = apply(
                        &mut actual,
                        &schema,
                        dialect,
                        BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(
                            &budget,
                        ))),
                    );
                    (result, actual, budget.consumed(QueryCharge::CompilerWork))
                };
                let (result, actual, units) = run(u64::MAX);
                result.unwrap();
                assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
                assert!(run(units).0.is_ok());
                assert!(matches!(
                    run(units - 1).0,
                    Err(crate::Error::QueryControl(
                        QueryControlError::CompilerWorkExceeded
                    ))
                ));
            }
        }
    }
}

#[test]
fn key_coverage_matches_raw_composite_unique_nullable_and_alias_rules() {
    for primary in [false, true] {
        for nullable in [false, true] {
            for covered in 0..=2 {
                for alias in [0, 1] {
                    let mut table = TableSchema::new("table");
                    table.columns = vec![
                        sf_sql::Column::new("a", "text", !nullable),
                        sf_sql::Column::new("b", "text", !nullable),
                    ];
                    let key = vec!["a".to_owned(), "b".to_owned()];
                    if primary {
                        table.primary_key = key;
                    } else {
                        table.unique = vec![vec![], key];
                    }
                    let mut bindings = std::collections::BTreeMap::new();
                    for name in ["a", "b"].into_iter().take(covered) {
                        bindings.insert(
                            name.to_owned(),
                            TermDef::Derived {
                                alias: 0,
                                term_map: TermMap::Column(name.into(), TermSpec::plain_literal()),
                            },
                        );
                    }
                    let run = |units| {
                        let budget =
                            QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
                        let result = key_covered(
                            &table,
                            alias,
                            &bindings,
                            BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(
                                &budget,
                            ))),
                        );
                        (result, budget.consumed(QueryCharge::CompilerWork))
                    };
                    let expected =
                        super::super::table_key_covered_by_bindings_raw(&table, alias, &bindings);
                    assert_eq!(
                        expected,
                        covered == 2 && alias == 0 && (primary || !nullable)
                    );
                    let (actual, units) = run(u64::MAX);
                    assert_eq!(actual.unwrap(), expected);
                    assert_eq!(run(units).0.unwrap(), expected);
                    assert!(matches!(
                        run(units - 1).0,
                        Err(crate::Error::QueryControl(
                            QueryControlError::CompilerWorkExceeded
                        ))
                    ));
                }
            }
        }
    }
}

#[test]
fn injectivity_preserves_base_iri_and_noninjective_fallback() {
    let maps = [
        TermMap::Column("id".into(), TermSpec::iri().with_base("http://ex/")),
        TermMap::Column("id".into(), TermSpec::plain_literal()),
        TermMap::Template(Template::parse("{a}{b}").unwrap(), TermSpec::iri()),
        TermMap::Template(Template::parse("http://ex/{id}").unwrap(), TermSpec::iri()),
    ];
    for map in maps {
        for alias in [0, 1] {
            let mut branch = Branch::empty();
            branch.bindings.insert(
                "s".into(),
                TermDef::Derived {
                    term_map: map.clone(),
                    alias: 0,
                },
            );
            let run = |units| {
                let control =
                    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
                let result = bindings_injective(
                    &branch,
                    alias,
                    BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(
                        &control,
                    ))),
                );
                (result, control.consumed(QueryCharge::CompilerWork))
            };
            let (actual, used) = run(u64::MAX);
            let expected = super::super::alias_bindings_injective_raw(&branch, alias);
            assert_eq!(actual.unwrap(), expected);
            assert_eq!(run(used).0.unwrap(), expected);
            assert!(matches!(
                run(used - 1).0,
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerWorkExceeded
                ))
            ));
        }
    }
}

#[test]
fn scan_projection_matches_raw_and_every_budget_refusal_is_atomic() {
    let mut source = Branch::single(Scan {
        alias: 0,
        source: LogicalSource::Table("input".into()).into(),
    });
    source.bindings.insert(
        "s".into(),
        TermDef::Derived {
            alias: 0,
            term_map: TermMap::Template(
                Template::parse("http://ex/{id}").unwrap(),
                TermSpec::iri(),
            ),
        },
    );
    source.where_conds = vec![SqlCond::NativeColEq(
        ColRef::new(0, "native"),
        ColRef::new(1, "other"),
    )];
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    let measured = budget(u64::MAX);
    fn work(b: &QueryBudget) -> BuildWork<'_> {
        BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(b)))
    }
    let columns = used_columns(&source, 0, work(&measured)).unwrap();
    assert_eq!(columns, super::super::alias_used_columns_raw(&source, 0));
    assert_eq!(
        columns,
        [Box::<str>::from("id"), Box::<str>::from("native")]
    );
    let mut expected = source.clone();
    super::super::wrap_scan_distinct_raw(&mut expected, 0, &columns, sf_sql::Dialect::Sqlite);
    let measured = budget(u64::MAX);
    let mut actual = source.clone();
    wrap_scan(&mut actual, 0, &columns, work(&measured)).unwrap();
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    let units = measured.consumed(QueryCharge::CompilerWork);
    wrap_scan(&mut source.clone(), 0, &columns, work(&budget(units))).unwrap();
    for allowed in 0..units {
        let mut refused = source.clone();
        assert!(matches!(
            wrap_scan(&mut refused, 0, &columns, work(&budget(allowed))),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
        assert_eq!(format!("{refused:?}"), format!("{source:?}"));
    }
}

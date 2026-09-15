use super::*;
use crate::iq::{CmpOp, LexicalKey, Scan, ScanSource};
use sf_core::ir::{Template, TermSpec};

#[test]
fn projection_scan_retains_source_admission() {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let source = LogicalSource::Table("items".into());
    let catalog = catalog(&source);
    let term = TermMap::Column("src".into(), TermSpec::plain_literal());
    let scan = Scan {
        alias: 1,
        source: ScanSource::Projection {
            input: Box::new(Scan {
                alias: 0,
                source: source.into(),
            }),
            columns: vec![("src".into(), term)],
            guards: vec![],
            distinct: false,
            native_keys: vec![],
            lexical_keys: vec![],
        },
    };
    let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
    let run = |control: &QueryBudget| {
        let work = SourceWork::new(Some(control));
        let dialect = Dialect::Postgres;
        scan_ref_controlled(&scan, dialect, &catalog, &mut vec![], &mut 0, work)
    };
    let measured = budget(u64::MAX);
    let sql = run(&measured).unwrap();
    assert_eq!(
        sql,
        scan_ref(&scan, Dialect::Postgres, &catalog, &mut vec![], &mut 0).unwrap()
    );
    let n = measured.consumed(QueryCharge::SourceWork);
    assert!(n > 2); // Includes projection metadata, not just the two scan entries.
    assert_eq!(run(&budget(n)).unwrap(), sql);
    assert!(matches!(
        run(&budget(n - 1)),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
}

#[test]
fn borrowed_binding_view_keeps_sorted_base_first_recipe_ownership() {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let map = |names: &[&str]| {
        names
            .iter()
            .map(|name| {
                (
                    name.to_string(),
                    TermDef::Const(sf_core::Literal::new_simple_literal(*name).into()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let base = map(&["b", "d", "α"]);
    let overlay = map(&["a", "b", "c", "e", "α", "🙂"]);
    let budget = |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
    let run = |control: &QueryBudget| {
        BindingView::merged(&base, Some(&overlay), SourceWork::new(Some(control)))
    };
    let measured = budget(u64::MAX);
    let view = run(&measured).unwrap();
    let names: Vec<_> = view.iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["a", "b", "c", "d", "e", "α", "🙂"]);
    assert_eq!(view.len(), names.len());
    for (name, actual) in view.iter() {
        assert!(std::ptr::eq(
            actual,
            base.get(name).or_else(|| overlay.get(name)).unwrap()
        ));
    }
    let units = measured.consumed(QueryCharge::SourceWork);
    assert_eq!(run(&budget(units)).unwrap().len(), view.len());
    assert!(matches!(
        run(&budget(units - 1)),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
    let direct = BindingView::merged(&base, None, SourceWork::new(Some(&budget(0)))).unwrap();
    assert!(matches!(direct, BindingView::Direct(_)));
    assert_eq!(
        direct.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        ["b", "d", "α"]
    );
}

#[test]
fn borrowed_root_modifiers_match_owned_emission_in_every_dialect() {
    let maps = sf_mapping::parse_r2rml(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#n> rr:logicalTable [rr:tableName "items"]; rr:subjectMap [rr:template "http://ex/{src}"];
rr:predicateObjectMap [rr:predicate <http://ex/edge>; rr:objectMap [rr:template "http://ex/{src}"]];
rr:predicateObjectMap [rr:predicate <http://ex/value>; rr:objectMap [rr:column "src"]];
rr:predicateObjectMap [rr:predicate <http://ex/noninjective>; rr:objectMap [rr:template "http://ex/{src}{tenant}"]]."#).unwrap();
    let queries = [
        "SELECT ?o WHERE { ?s <http://ex/value> ?o }",
        "SELECT (COUNT(?o) AS ?n) WHERE { ?s <http://ex/value> ?o }",
        "SELECT ?o WHERE { ?s <http://ex/edge>+ ?o }",
        "SELECT ?o WHERE { ?s <http://ex/noninjective> ?o }",
    ];
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        for (kind, query) in queries.iter().enumerate() {
            let plan = crate::parse_and_translate(query, &maps, dialect).unwrap();
            assert_eq!(plan.branches.len(), 1);
            let mut branch = plan.branches[0].clone();
            if kind == 1 {
                assert!(branch.agg.is_some());
            }
            if kind == 2 {
                assert!(branch.path.is_some());
            }
            if kind == 0 {
                branch.where_conds.push(SqlCond::NativeCmp(
                    ColRef::new(branch.core[0].alias, "tenant"),
                    CmpOp::Eq,
                    "allowed".into(),
                ));
            }
            let source = LogicalSource::Table("items".into());
            let mut catalog = catalog(&source);
            if dialect != Dialect::Postgres {
                catalog
                    .insert_live_result(
                        &source,
                        ["src", "tenant"]
                            .into_iter()
                            .map(|name| sf_sql::backend::ResultColumn {
                                name: name.into(),
                                natural_datatype: Some(sf_core::datatype::XsdTypeCode::String),
                                native_scalar: None,
                                text_key: Some(TextKey::Verbatim),
                                sqlite_decode: (dialect == Dialect::Sqlite).then_some(
                                    SqliteDecode {
                                        declared: Some(sf_core::datatype::XsdTypeCode::String),
                                        padding: None,
                                    },
                                ),
                            })
                            .collect(),
                    )
                    .unwrap();
            }
            let mut successful = 0;
            for mask in 0..32 {
                branch.distinct = mask & 1 != 0;
                branch.limit = Some(9);
                branch.offset = 4;
                let single = mask & 2 != 0;
                let distinct = mask & 4 != 0;
                let unordered = mask & 8 != 0;
                let limit = (mask & 16 != 0).then_some(2);
                let original = format!("{branch:?}");
                let modifiers =
                    BranchModifiers::prepared(&branch, single, distinct, unordered, limit, 1);
                let mut owned = branch.clone();
                if single {
                    owned.distinct = distinct;
                    if unordered {
                        owned.limit = limit;
                        owned.offset = 1;
                    }
                    // The actual nested emitter must agree with an explicitly
                    // prepared owned input, including clearing stale DISTINCT.
                    let mut nested = plan.clone();
                    nested.branches = vec![branch.clone()];
                    nested.distinct = distinct;
                    nested.limit = limit;
                    nested.offset = 1;
                    nested.order = if unordered {
                        vec![]
                    } else {
                        vec![crate::iq::OrderKey {
                            var: "unused-plan-order".into(),
                            descending: false,
                            expr: None,
                        }]
                    };
                    let mut prepared = nested.clone();
                    prepared.branches = nested.prepared_branches();
                    assert_eq!(
                        emit_subplan_sql(&nested, dialect, &catalog).map_err(|e| e.to_string()),
                        emit_subplan_sql(&prepared, dialect, &catalog).map_err(|e| e.to_string()),
                        "nested modifiers differ: {dialect:?}/{kind}/{mask}",
                    );
                }
                let actual = emit_branch_with_modifiers(&branch, dialect, &catalog, modifiers);
                let expected = emit_branch_with(&owned, dialect, &catalog);
                assert_eq!(format!("{branch:?}"), original);
                let (actual, expected) = match (actual, expected) {
                    (Ok(a), Ok(e)) => {
                        successful += 1;
                        (a, e)
                    }
                    (Err(a), Err(e)) => {
                        assert_eq!(a.to_string(), e.to_string());
                        continue;
                    }
                    _ => panic!("modifier outcome differs: {dialect:?}/{kind}/{mask}"),
                };
                assert_eq!(actual.sql, expected.sql, "{dialect:?}/{kind}/{mask}");
                assert_eq!(actual.projection, expected.projection);
                assert_eq!(actual.params, expected.params);
                assert_eq!(actual.metadata_sql, expected.metadata_sql);
                assert_eq!(actual.sqlite_character_keys, expected.sqlite_character_keys);
                assert_eq!(actual.sqlite_lexical_keys, expected.sqlite_lexical_keys);
                assert_eq!(format!("{branch:?}"), original);
            }
            assert!(
                successful > 0,
                "must exercise successful emission: {dialect:?}/{kind}"
            );
        }
    }
}

fn catalog(source: &LogicalSource) -> ColumnCatalog {
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            source,
            vec![
                sf_sql::backend::ResultColumn {
                    natural_datatype: None,
                    name: "src".into(),
                    native_scalar: Some(NativeScalarKey::PostgresNumeric),
                    text_key: None,
                    sqlite_decode: None,
                },
                sf_sql::backend::ResultColumn {
                    natural_datatype: None,
                    name: "tenant".into(),
                    native_scalar: None,
                    text_key: Some(TextKey::Verbatim),
                    sqlite_decode: None,
                },
            ],
        )
        .unwrap();
    catalog
}

#[test]
fn numeric_nested_distinct_union_preserves_decoder_keys() {
    let maps = sf_mapping::parse_r2rml(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#n> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/item>;
rr:predicateObjectMap [rr:predicate <http://ex/edge>; rr:objectMap [rr:template "http://ex/n/{src}"]]."#).unwrap();
    let plan = crate::parse_and_translate("SELECT ?o WHERE { { SELECT DISTINCT ?o WHERE { { ?s <http://ex/edge> ?o FILTER(?o = <http://ex/n/1.0>) } UNION { ?s <http://ex/edge> ?o FILTER(sameTerm(?o, <http://ex/n/1.00>)) } } } }", &maps, Dialect::Postgres).unwrap();
    assert!(
        plan.source_sized_states()
            .contains(&crate::resource_profile::SourceSizedState::ProjectedDistinct),
        "serving still rejects its existing source-sized profile"
    );
    let catalog = catalog(&LogicalSource::Table("items".into()));
    let (sql, params) = emit_subplan_sql(&plan, Dialect::Postgres, &catalog).unwrap();
    assert!(sql.contains(" UNION ALL "), "{sql}");
    assert!(
        sql.contains("__sf_numeric_raw.c0 AS TEXT) AS JSON"),
        "{sql}"
    );
    assert!(
        params.iter().any(|p| p.contains("1.0")) && params.iter().any(|p| p.contains("1.00")),
        "{params:?}"
    );
}

#[test]
fn numeric_d1_unknown_companion_cannot_borrow_unrelated_text_proof() {
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            vec![
                sf_sql::backend::ResultColumn {
                    natural_datatype: None,
                    name: "src".into(),
                    native_scalar: Some(NativeScalarKey::PostgresNumeric),
                    text_key: None,
                    sqlite_decode: None,
                },
                sf_sql::backend::ResultColumn {
                    natural_datatype: None,
                    name: "tenant".into(),
                    native_scalar: None,
                    text_key: Some(TextKey::Verbatim),
                    sqlite_decode: None,
                },
                sf_sql::backend::ResultColumn {
                    natural_datatype: None,
                    name: "mystery".into(),
                    native_scalar: None,
                    text_key: None,
                    sqlite_decode: None,
                },
            ],
        )
        .unwrap();
    let scan = Scan {
        alias: 0,
        source: ScanSource::Projection {
            input: Box::new(Scan {
                alias: 0,
                source: source.into(),
            }),
            columns: ["src", "mystery"]
                .into_iter()
                .map(|name| {
                    (
                        name.into(),
                        TermMap::Column(name.into(), TermSpec::plain_literal()),
                    )
                })
                .collect(),
            guards: vec![],
            distinct: true,
            native_keys: vec![],
            lexical_keys: vec![LexicalKey {
                column: "src".into(),
                mode: LexicalMode::Decoded,
            }],
        },
    };
    assert!(matches!(
        scan_ref(&scan, Dialect::Postgres, &catalog, &mut vec![], &mut 0),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn rendered_width_pooling_retains_decoder_guards_without_key_authority() {
    let source = LogicalSource::Table("items".into());
    let catalog = catalog(&source);
    let branches = ["http://ex/{src}", "http://ex/{src}/{tenant}"]
        .into_iter()
        .map(|recipe| {
            let mut b = Branch::single(Scan {
                alias: 0,
                source: source.clone().into(),
            });
            b.bindings.insert(
                "x".into(),
                TermDef::Derived {
                    alias: 0,
                    term_map: TermMap::Template(Template::parse(recipe).unwrap(), TermSpec::iri()),
                },
            );
            crate::iq::decode_valid::validate_term(
                &b.bindings["x"],
                Dialect::Postgres,
                &mut b.where_conds,
            );
            b
        })
        .collect();
    let pooled = crate::iq::lower::pool_rendered(branches, &["x".into()], Dialect::Postgres)
        .unwrap()
        .expect("same-alias decoder guards are retained, not rejected");
    for branch in pooled {
        let emitted = emit_branch_with(&branch, Dialect::Postgres, &catalog).unwrap();
        assert!(
            emitted.sql.contains("AS JSON) AS TEXT) IS NOT NULL"),
            "{}",
            emitted.sql
        );
        let ScanSource::Projection { guards, .. } = &branch.core[0].source else {
            panic!("typed projection")
        };
        assert!(guards
            .iter()
            .any(|g| matches!(g, SqlCond::DecodedIsNotNull(_))));
    }
}

#[test]
fn final_distinct_uses_only_current_output_consumers() {
    let source = LogicalSource::Table("items".into());
    let catalog = catalog(&source);
    let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
    let mut b = Branch::single(Scan {
        alias: 0,
        source: source.into(),
    });
    let iri = TermMap::Template(Template::parse("http://ex/{src}").unwrap(), TermSpec::iri());
    b.bindings.insert(
        "iri".into(),
        TermDef::Derived {
            alias: 0,
            term_map: iri.clone(),
        },
    );
    b.bindings.insert(
        "natural".into(),
        TermDef::Derived {
            alias: 0,
            term_map: TermMap::Column("src".into(), TermSpec::plain_literal()),
        },
    );
    b.distinct = true;
    assert_eq!(
        distinct_keys(&b, Dialect::Postgres, &actuals),
        [Some(NativeScalarKey::PostgresNumeric)]
    );
    let operand = crate::iq::iri_cmp::IriOperand::from_map(&iri, 0).unwrap();
    b.where_conds.push(SqlCond::IriCmp(Box::new(
        crate::iq::iri_cmp::IriComparison {
            left: operand.clone(),
            right: operand,
        },
    )));
    b.bindings.remove("iri");
    assert_eq!(
        distinct_keys(&b, Dialect::Postgres, &actuals),
        [None],
        "hidden IRI must not over-distinguish natural output"
    );
    assert!(
        !crate::cascade::distinct_scan::lexical_keys(&b, 0).is_empty(),
        "D1 keeps original hidden consumer"
    );
    assert_eq!(distinct_keys(&b, Dialect::MySql, &actuals), [None]);
}

#[test]
fn numeric_d1_retains_native_output_and_fences_policy_before_outer_validation() {
    let source = LogicalSource::Table("items".into());
    let catalog = catalog(&source);
    let policy = SqlCond::NativeCmp(ColRef::new(0, "tenant"), CmpOp::Eq, "allowed".into());
    let scan = Scan {
        alias: 0,
        source: ScanSource::Projection {
            input: Box::new(Scan {
                alias: 0,
                source: source.clone().into(),
            }),
            columns: vec![(
                "src".into(),
                TermMap::Column("src".into(), TermSpec::plain_literal()),
            )],
            guards: vec![
                policy.clone(),
                SqlCond::DecodedIsNotNull(ColRef::new(0, "src")),
            ],
            distinct: true,
            native_keys: vec![],
            lexical_keys: vec![LexicalKey {
                column: "src".into(),
                mode: LexicalMode::DecodedWithNatural,
            }],
        },
    };
    let mut b = Branch::single(scan);
    b.bindings.insert(
        "n".into(),
        TermDef::Derived {
            alias: 0,
            term_map: TermMap::Column("src".into(), TermSpec::plain_literal()),
        },
    );
    b.where_conds
        .push(SqlCond::DecodedIsNotNull(ColRef::new(0, "src")));
    let emitted = emit_branch_with(&b, Dialect::Postgres, &catalog).unwrap();
    assert!(
        emitted
            .sql
            .contains("SELECT t0.\"src\" AS \"src\", ROW_NUMBER()"),
        "{}",
        emitted.sql
    );
    assert!(
        emitted.sql.contains("AS JSON) AS TEXT) COLLATE \"C\""),
        "{}",
        emitted.sql
    );
    assert!(emitted.sql.contains("OFFSET 0"), "{}", emitted.sql);
    assert!(
        emitted.sql.contains("CASE WHEN (t0.\"tenant\" = $1) THEN"),
        "{}",
        emitted.sql
    );
    assert_eq!(emitted.params, ["allowed"]);
    assert_eq!(
        scan_actuals(&b.core[0], Dialect::Postgres, &catalog).scalar_columns["src"],
        NativeScalarKey::PostgresNumeric
    );
    b.core[0].source = source.into();
    b.where_conds.push(policy);
    let emitted = emit_branch_with(&b, Dialect::Postgres, &catalog).unwrap();
    assert!(
        emitted.sql.contains("CASE WHEN (t0.\"tenant\" = $1) THEN"),
        "{}",
        emitted.sql
    );
    assert_eq!(emitted.params, ["allowed"]);
}

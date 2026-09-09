use super::*;

fn fixture() -> (Vec<sf_core::ir::TriplesMap>, ColumnCatalog) {
    let maps = sf_mapping::parse_r2rml(
        r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#edge> rr:logicalTable [ rr:tableName "edge" ];
          rr:subjectMap [ rr:template "http://ex/n/{src}" ];
          rr:predicateObjectMap [ rr:predicate <http://ex/p>;
            rr:objectMap [ rr:template "http://ex/n/{dst}" ] ] .
    "#,
    )
    .unwrap();
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &maps[0].source,
            vec![
                sf_sql::backend::ResultColumn {
                    native_scalar: None,
                    sqlite_decode: None,
                    name: "src".into(),
                    text_key: None,
                },
                sf_sql::backend::ResultColumn {
                    native_scalar: None,
                    sqlite_decode: None,
                    name: "dst".into(),
                    text_key: Some(sf_sql::backend::TextKey::Verbatim),
                },
            ],
        )
        .unwrap();
    (maps, catalog)
}

#[test]
fn aggregate_metadata_uses_sql_projection_order() {
    let (maps, mut catalog) = fixture();
    let plan = crate::parse_and_translate(
        r#"
        SELECT ?a ?z WHERE {
          { SELECT ?a ?z (COUNT(*) AS ?zz) WHERE {
              ?a <http://ex/p> ?z . FILTER EXISTS { ?x <http://ex/p>+ ?y }
            } GROUP BY ?z ?a }
          { SELECT ?a ?z (COUNT(*) AS ?otherCount) WHERE {
              ?a <http://ex/p> ?z
            } GROUP BY ?z ?a }
        }
    "#,
        &maps,
        Dialect::Postgres,
    )
    .unwrap();
    let branches = plan.prepared_branches();
    // Runtime probes generated DISTINCT sources too; don't substitute the base
    // table's identity for those separately prepared statements in this test.
    for source in live_metadata_sources(&branches) {
        catalog
            .insert_live_result(
                source,
                vec![
                    sf_sql::backend::ResultColumn {
                        native_scalar: None,
                        sqlite_decode: None,
                        name: "src".into(),
                        text_key: None,
                    },
                    sf_sql::backend::ResultColumn {
                        native_scalar: None,
                        sqlite_decode: None,
                        name: "dst".into(),
                        text_key: Some(sf_sql::backend::TextKey::Verbatim),
                    },
                ],
            )
            .unwrap();
    }
    let mut checked = 0;
    for sp in &branches[0].subplan_joins {
        let emitted = emit_branch_with(&sp.plan.branches[0], Dialect::Postgres, &catalog).unwrap();
        assert_eq!(emitted.projection[0].column.as_ref(), "dst");
        assert_eq!(emitted.projection[1].column.as_ref(), "src");
        let actuals = subplan_actuals(&sp.plan, Dialect::Postgres, &catalog);
        assert_eq!(
            actuals.text_columns,
            HashMap::from([("c0".into(), sf_sql::backend::TextKey::Verbatim)])
        );
        checked += 1;
    }
    assert_eq!(checked, 2);
    let sql = emit_branch_with(&branches[0], Dialect::Postgres, &catalog)
        .unwrap()
        .sql;
    assert!(
        !sql.contains("\"c1\" COLLATE"),
        "integer aggregate key must remain native: {sql}"
    );
}

#[test]
fn singleton_metadata_uses_prepared_distinct_and_term_dedup_state() {
    let (maps, mut catalog) = fixture();
    catalog
        .insert_live_result(
            &maps[0].source,
            vec![
                sf_sql::backend::ResultColumn {
                    native_scalar: None,
                    sqlite_decode: None,
                    name: "src".into(),
                    text_key: None,
                },
                sf_sql::backend::ResultColumn {
                    native_scalar: None,
                    sqlite_decode: None,
                    name: "dst".into(),
                    text_key: Some(TextKey::SqliteCharacter(4)),
                },
            ],
        )
        .unwrap();
    for plan_distinct in [false, true] {
        for stored_distinct in [false, true] {
            for noninjective in [false, true] {
                let mut plan = crate::parse_and_translate(
                    "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }",
                    &maps,
                    Dialect::Sqlite,
                )
                .unwrap();
                let mut branch = Branch::single(crate::iq::Scan {
                    alias: 0,
                    source: maps[0].source.clone().into(),
                });
                branch.bindings.insert(
                    "v".into(),
                    TermDef::Derived {
                        alias: 0,
                        term_map: if noninjective {
                            TermMap::Template(
                                sf_core::ir::Template::parse("{src}{dst}").unwrap(),
                                sf_core::ir::TermSpec::plain_literal(),
                            )
                        } else {
                            TermMap::Column("dst".into(), sf_core::ir::TermSpec::plain_literal())
                        },
                    },
                );
                branch.distinct = stored_distinct;
                plan.branches = vec![branch];
                plan.distinct = plan_distinct;
                let expected = if plan_distinct && !noninjective {
                    TextKey::Verbatim
                } else {
                    TextKey::SqliteCharacter(4)
                };
                let actuals = subplan_actuals(&plan, Dialect::Sqlite, &catalog);
                assert_eq!(
                    actuals.text_columns.values().copied().collect::<Vec<_>>(),
                    vec![expected]
                );
                let emitted =
                    emit_branch_with(&plan.prepared_branches()[0], Dialect::Sqlite, &catalog)
                        .unwrap();
                assert_eq!(
                    emitted.sqlite_character_keys,
                    plan_distinct && !noninjective
                );
            }
        }
    }
}

#[test]
fn sqlite_avg_layout_includes_only_its_metadata_operand() {
    let (maps, catalog) = fixture();
    let mut plan = crate::parse_and_translate(
        "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }",
        &maps,
        Dialect::Sqlite,
    )
    .unwrap();
    let branch = &mut plan.branches[0];
    branch.core = vec![crate::iq::Scan {
        alias: 0,
        source: maps[0].source.clone().into(),
    }];
    branch.agg = Some(Aggregation {
        keys: vec![crate::iq::GroupKey {
            var: "s".into(),
            cols: vec![ColRef::new(0, "src")],
        }],
        aggs: vec![AggCol {
            var: "mean".into(),
            kind: AggKind::Avg,
            arg: Some(ColRef::new(0, "dst")),
            distinct: false,
            out: ColRef::new(1, "mean"),
            fixed_type: None,
        }],
    });
    for (dialect, width, text) in [
        (
            Dialect::Sqlite,
            3,
            HashMap::from([("c2".into(), sf_sql::backend::TextKey::Verbatim)]),
        ),
        (Dialect::Postgres, 2, HashMap::new()),
    ] {
        let emitted = emit_branch_with(&plan.branches[0], dialect, &catalog).unwrap();
        let actuals = subplan_actuals(&plan, dialect, &catalog);
        assert_eq!(emitted.projection.len(), width);
        assert_eq!(actuals.columns.len(), width);
        assert_eq!(actuals.text_columns, text);
    }
}

#[test]
fn nested_metadata_visits_each_plan_once_independent_of_width() {
    let (maps, catalog) = fixture();
    let mut plan = crate::parse_and_translate(
        "SELECT ?s ?o WHERE { ?s <http://ex/p>+ ?o }",
        &maps,
        Dialect::Postgres,
    )
    .unwrap();
    for _ in 0..20 {
        let mut wrapper = plan.clone();
        let mut branch = Branch::empty();
        for i in 0..2 {
            branch.bindings.insert(
                format!("v{i}"),
                TermDef::Derived {
                    term_map: TermMap::Column(
                        format!("c{i}").into(),
                        sf_core::ir::TermSpec::plain_literal(),
                    ),
                    alias: 0,
                },
            );
        }
        branch.subplan_joins.push(crate::iq::SubPlanJoin {
            alias: 0,
            plan: Box::new(plan),
            on: vec![],
            left: false,
        });
        wrapper.branches = vec![branch];
        plan = wrapper;
    }
    METADATA_VISITS.with(|visits| visits.set(0));
    let actuals = subplan_actuals(&plan, Dialect::Postgres, &catalog);
    assert_eq!(actuals.columns.len(), 2);
    assert_eq!(
        actuals.text_columns,
        HashMap::from([("c0".into(), sf_sql::backend::TextKey::Verbatim)])
    );
    METADATA_VISITS.with(|visits| assert_eq!(visits.get(), 21));
}

#[test]
fn standalone_path_retains_projected_text_facts() {
    let (maps, catalog) = fixture();
    let plan = crate::parse_and_translate(
        "SELECT ?s ?o WHERE { ?s <http://ex/p>+ ?o }",
        &maps,
        Dialect::Postgres,
    )
    .unwrap();
    assert!(plan.branches[0].path.is_some());
    let projection = emit_branch_with(&plan.branches[0], Dialect::Postgres, &catalog)
        .unwrap()
        .projection;
    let expected = projection
        .iter()
        .enumerate()
        .filter(|(_, c)| c.column.as_ref() == "sf_o")
        .map(|(i, _)| (format!("c{i}"), sf_sql::backend::TextKey::Verbatim))
        .collect::<HashMap<_, _>>();
    assert!(!expected.is_empty());
    assert_eq!(
        subplan_actuals(&plan, Dialect::Postgres, &catalog).text_columns,
        expected
    );
}

#[test]
fn character_decoder_emission_is_explicit_and_preserved_in_metadata() {
    for (dialect, key) in [
        (Dialect::Sqlite, TextKey::SqliteCharacter(4)),
        (Dialect::Postgres, TextKey::PostgresCharacter),
    ] {
        let (maps, mut catalog) = fixture();
        let plan = crate::parse_and_translate(
            "SELECT ?s ?o WHERE { ?s <http://ex/p>+ ?o }",
            &maps,
            dialect,
        )
        .unwrap();
        let branches = plan.prepared_branches();
        for source in live_metadata_sources(&branches) {
            catalog
                .insert_live_result(
                    source,
                    ["src", "dst"]
                        .map(|name| sf_sql::backend::ResultColumn {
                            native_scalar: None,
                            sqlite_decode: None,
                            name: name.into(),
                            text_key: Some(key),
                        })
                        .to_vec(),
                )
                .unwrap();
        }
        let emitted = emit_branch_with(&branches[0], dialect, &catalog).unwrap();
        assert_eq!(emitted.sqlite_character_keys, dialect == Dialect::Sqlite);
        if dialect == Dialect::Sqlite {
            assert!(emitted
                .metadata_sql
                .as_ref()
                .unwrap()
                .contains("__sf_character_key_v1"));
        } else {
            assert!(emitted.sql.contains("pg_catalog.bpcharsend"));
        }
        let ordinary = crate::parse_and_translate(
            "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }",
            &maps,
            dialect,
        )
        .unwrap();
        assert_eq!(
            emit_branch_with(&ordinary.prepared_branches()[0], dialect, &catalog)
                .unwrap()
                .sqlite_character_keys,
            dialect == Dialect::Sqlite,
        );
    }
}

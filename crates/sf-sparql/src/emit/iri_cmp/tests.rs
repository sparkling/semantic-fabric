use super::*;
use crate::iq::{scan::LexicalMode, LexicalKey, Scan, ScanSource};
use sf_core::ir::TermSpec;

#[test]
fn iri_dedup_never_falls_through_to_raw_distinct_for_an_unproven_sibling_key() {
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            ["u", "v"]
                .map(|name| sf_sql::backend::ResultColumn {
                    native_scalar: None,
                    name: name.into(),
                    text_key: None,
                    sqlite_decode: Some(SqliteDecode {
                        declared: None,
                        padding: None,
                    }),
                })
                .to_vec(),
        )
        .unwrap();
    let scan = Scan {
        alias: 0,
        source: ScanSource::Projection {
            input: Box::new(Scan {
                alias: 0,
                source: source.into(),
            }),
            columns: ["u", "v"]
                .map(|name| {
                    (
                        name.into(),
                        TermMap::Column(name.into(), TermSpec::plain_literal()),
                    )
                })
                .to_vec(),
            guards: vec![],
            distinct: true,
            native_keys: vec![],
            lexical_keys: vec![LexicalKey {
                column: "u".into(),
                mode: LexicalMode::Iri {
                    base: Some("http://ex/".into()),
                },
            }],
        },
    };
    let error = super::super::scan::scan_ref(&scan, Dialect::Sqlite, &catalog, &mut vec![], &mut 0)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("exact keys for every scan consumer"),
        "{error}"
    );
}

#[test]
fn missing_or_native_decoder_never_authorizes_raw_iri_equality() {
    let comparison = IriComparison {
        left: IriOperand::Column {
            column: ColRef::new(0, "u"),
            base: Some("http://ex/".into()),
        },
        right: IriOperand::Constant(sf_core::NamedNode::new_unchecked("http://ex/AB")),
    };
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        assert!(matches!(
            render(
                &comparison,
                dialect,
                &ColumnCatalog::default(),
                &ActualColumns::default(),
                &mut vec![],
                &mut 0
            ),
            Err(Error::Unsupported(_))
        ));
    }
}

#[test]
fn zero_slot_template_registers_its_finalizer_without_a_column_decoder() {
    let catalog = ColumnCatalog::default();
    let comparison = IriComparison {
        left: IriOperand::Template {
            parts: vec![IriPart::Literal("1:x".into())],
            base: Some("http://ex/".into()),
        },
        right: IriOperand::Constant(sf_core::NamedNode::new_unchecked("http://ex/1:x")),
    };
    let mut params = vec![];
    let sql = render(
        &comparison,
        Dialect::Sqlite,
        &catalog,
        &ActualColumns::default(),
        &mut params,
        &mut 0,
    )
    .unwrap();
    assert!(sql.contains("__sf_iri_key_v1"));
    assert!(catalog
        .lexical_keys
        .load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(params, ["http://ex/", "http://ex/1:x"]);
}

#[test]
fn native_scalar_proof_survives_raw_projection_but_not_names_only_refresh() {
    for (dialect, key, expression) in [
        (Dialect::Postgres, NativeScalarKey::Integer, "CAST("),
        (Dialect::MySql, NativeScalarKey::Integer, "DECIMAL(20, 0)"),
        (
            Dialect::Postgres,
            NativeScalarKey::PostgresBoolean,
            "CASE WHEN",
        ),
        (
            Dialect::Postgres,
            NativeScalarKey::PostgresBytea,
            "pg_catalog.encode(",
        ),
        (Dialect::MySql, NativeScalarKey::MysqlBinaryBytes, "HEX("),
        (Dialect::MySql, NativeScalarKey::MysqlBit, "HEX(CAST("),
        (Dialect::MySql, NativeScalarKey::MysqlTimestamp, "RPAD("),
        (Dialect::MySql, NativeScalarKey::MysqlTime, "'-00:00:00'"),
        (
            Dialect::MySql,
            NativeScalarKey::MysqlDecimal,
            "AS CHAR) USING utf8mb4",
        ),
    ] {
        let source = LogicalSource::Table("items".into());
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                &source,
                vec![sf_sql::backend::ResultColumn {
                    name: "id".into(),
                    native_scalar: Some(key),
                    text_key: None,
                    sqlite_decode: None,
                }],
            )
            .unwrap();
        let scan = Scan {
            alias: 7,
            source: ScanSource::Projection {
                input: Box::new(Scan {
                    alias: 3,
                    source: source.clone().into(),
                }),
                columns: vec![(
                    "renamed".into(),
                    TermMap::Column("ID".into(), TermSpec::plain_literal()),
                )],
                guards: vec![],
                distinct: true,
                native_keys: vec![],
                lexical_keys: vec![],
            },
        };
        let comparison = IriComparison {
            left: IriOperand::Template {
                parts: vec![
                    IriPart::Literal("http://ex/".into()),
                    IriPart::Column(ColRef::new(7, "renamed")),
                ],
                base: None,
            },
            right: IriOperand::Constant(sf_core::NamedNode::new_unchecked("http://ex/01")),
        };
        let actual = scan_actuals(&scan, dialect, &catalog);
        assert!(
            actual.text_columns.is_empty(),
            "scalar proof is not text authority"
        );
        let mut params = vec![];
        let sql = render(
            &comparison,
            dialect,
            &catalog,
            &HashMap::from([(7, actual)]),
            &mut params,
            &mut 0,
        )
        .unwrap();
        assert!(sql.contains(expression), "{sql}");
        assert!(
            !sql.contains("JSON_TABLE") && !sql.contains("unnest"),
            "proved scalar alphabets need no generic byte encoder"
        );
        assert_eq!(
            sql.contains("'%3A'"),
            matches!(
                key,
                NativeScalarKey::MysqlTimestamp | NativeScalarKey::MysqlTime
            )
        );
        dialect
            .emit_via_ast(&format!("SELECT {sql}"))
            .unwrap_or_else(|error| panic!("{dialect:?} {key:?}: {error}: {sql}"));
        assert!(!sql.contains("http://ex/01"), "query values stay bound");
        assert_eq!(params, ["http://ex/", "http://ex/01"]);
        catalog.insert(&source, vec!["id".into()]);
        let actuals = HashMap::from([(7, scan_actuals(&scan, dialect, &catalog))]);
        assert!(matches!(
            render(
                &comparison,
                dialect,
                &catalog,
                &actuals,
                &mut vec![],
                &mut 0
            ),
            Err(Error::Unsupported(_))
        ));
    }
}

#[test]
fn native_static_templates_require_live_decoder_facts_and_preserve_char_padding() {
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        for key in [
            None,
            Some(TextKey::Verbatim),
            Some(TextKey::PostgresCharacter),
        ] {
            if dialect == Dialect::MySql && key == Some(TextKey::PostgresCharacter) {
                continue;
            }
            let source = LogicalSource::Table("items".into());
            let mut catalog = ColumnCatalog::default();
            catalog
                .insert_live_result(
                    &source,
                    vec![sf_sql::backend::ResultColumn {
                        name: "id".into(),
                        native_scalar: None,
                        text_key: key,
                        sqlite_decode: None,
                    }],
                )
                .unwrap();
            let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
            let comparison = IriComparison {
                left: IriOperand::Template {
                    parts: vec![
                        IriPart::Literal("http://ex/".into()),
                        IriPart::Column(ColRef::new(0, "id")),
                    ],
                    base: None,
                },
                right: IriOperand::Constant(sf_core::NamedNode::new_unchecked("http://ex/a%20")),
            };
            let result = render(
                &comparison,
                dialect,
                &catalog,
                &actuals,
                &mut vec![],
                &mut 0,
            );
            if key.is_none() {
                assert!(
                    matches!(result, Err(Error::Unsupported(ref reason)) if reason.contains("proven live native decoder"))
                );
            } else {
                let sql = result.unwrap();
                assert_eq!(
                    sql.contains("bpcharsend"),
                    key == Some(TextKey::PostgresCharacter)
                );
                if dialect == Dialect::MySql {
                    assert!(sql.contains("CONCAT(") && sql.contains("utf8mb4_0900_bin"));
                }
            }
        }
    }
}

#[test]
fn scalar_recipes_do_not_cross_providers_or_coercing_union_outputs() {
    for (dialect, key) in [
        (Dialect::Postgres, NativeScalarKey::MysqlDecimal),
        (Dialect::MySql, NativeScalarKey::PostgresBoolean),
        (Dialect::Sqlite, NativeScalarKey::Integer),
    ] {
        assert!(matches!(
            scalar_lexical(key, "c", dialect),
            Err(Error::Unsupported(_))
        ));
    }
    let maps = sf_mapping::parse_r2rml(
        r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#m> rr:logicalTable [rr:tableName "items"];
          rr:subjectMap [rr:template "http://ex/{id}"];
          rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "id"]].
    "#,
    )
    .unwrap();
    for key in [
        NativeScalarKey::Integer,
        NativeScalarKey::MysqlDecimal,
        NativeScalarKey::MysqlDate,
        NativeScalarKey::MysqlDateTime,
        NativeScalarKey::MysqlBit,
        NativeScalarKey::MysqlTimestamp,
        NativeScalarKey::MysqlTime,
        NativeScalarKey::MysqlBinaryBytes,
    ] {
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                &maps[0].source,
                vec![sf_sql::backend::ResultColumn {
                    name: "id".into(),
                    native_scalar: Some(key),
                    text_key: None,
                    sqlite_decode: None,
                }],
            )
            .unwrap();
        let mut plan = crate::parse_and_translate(
            "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
            &maps,
            Dialect::MySql,
        )
        .unwrap();
        let mut branch = Branch::single(Scan {
            alias: 0,
            source: maps[0].source.clone().into(),
        });
        branch.bindings.insert(
            "s".into(),
            TermDef::Derived {
                alias: 0,
                term_map: TermMap::Column("id".into(), TermSpec::plain_literal()),
            },
        );
        plan.branches = vec![branch.clone()];
        let expected = if matches!(
            key,
            NativeScalarKey::MysqlDate | NativeScalarKey::MysqlDateTime
        ) {
            HashMap::new()
        } else {
            HashMap::from([("c0".into(), key)])
        };
        assert_eq!(
            subplan_actuals(&plan, Dialect::MySql, &catalog).scalar_columns,
            expected
        );
        plan.branches.push(branch.clone());
        let actuals = subplan_actuals(&plan, Dialect::MySql, &catalog);
        if matches!(
            key,
            NativeScalarKey::MysqlDecimal
                | NativeScalarKey::MysqlDate
                | NativeScalarKey::MysqlDateTime
                | NativeScalarKey::MysqlBit
                | NativeScalarKey::MysqlTimestamp
                | NativeScalarKey::MysqlTime
        ) {
            assert!(
                actuals.scalar_columns.is_empty(),
                "decimal, BIT and temporal UNION coercion has no identity proof"
            );
        } else {
            assert_eq!(actuals.scalar_columns, expected);
        }
        // Identical names with different native recipes must not confer authority.
        let other = LogicalSource::Table("other".into());
        catalog
            .insert_live_result(
                &other,
                vec![sf_sql::backend::ResultColumn {
                    name: "id".into(),
                    native_scalar: Some(NativeScalarKey::PostgresBytea),
                    text_key: None,
                    sqlite_decode: None,
                }],
            )
            .unwrap();
        branch.core[0].source = other.into();
        plan.branches[1] = branch;
        assert!(subplan_actuals(&plan, Dialect::MySql, &catalog)
            .scalar_columns
            .is_empty());
    }
}

#[test]
fn nested_emission_preserves_live_scalar_projection_recipes() {
    let maps = sf_mapping::parse_r2rml(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#m> rr:logicalTable [rr:tableName "items"];
        rr:subject <http://ex/s>;
        rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:template "http://ex/{id}"]]."#).unwrap();
    let plan = crate::parse_and_translate(
        "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
        &maps,
        Dialect::MySql,
    )
    .unwrap();
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &maps[0].source,
            vec![sf_sql::backend::ResultColumn {
                name: "id".into(),
                native_scalar: Some(NativeScalarKey::MysqlDate),
                text_key: None,
                sqlite_decode: None,
            }],
        )
        .unwrap();
    let top = emit_branch_with(&plan.prepared_branches()[0], Dialect::MySql, &catalog).unwrap();
    assert!(top.sql.contains("CAST(t0.`id` AS CHAR)"), "{}", top.sql);
    let (nested, _) = emit_subplan_sql(&plan, Dialect::MySql, &catalog).unwrap();
    assert!(nested.contains("CAST(t0.`id` AS CHAR)"), "{nested}");
}

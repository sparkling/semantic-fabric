use super::*;
use crate::iq::{CmpOp, LexicalKey, Scan, ScanSource};
use sf_core::ir::{Template, TermSpec};

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

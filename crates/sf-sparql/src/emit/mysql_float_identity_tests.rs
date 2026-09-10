use super::*;
use crate::iq::{scan::LexicalMode, LexicalKey};
use sf_core::{ir::TermSpec, Literal};

fn setup(key: NativeScalarKey) -> (LiteralComparison, ColumnCatalog, ActualColumns) {
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            vec![sf_sql::backend::ResultColumn {
                name: "v".into(),
                natural_datatype: Some(Double),
                native_scalar: Some(key),
                text_key: None,
                sqlite_decode: None,
            }],
        )
        .unwrap();
    let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
    (
        LiteralComparison {
            left: LiteralOperand::Column {
                column: ColRef::new(0, "v"),
                spec: TermSpec::plain_literal(),
            },
            right: LiteralOperand::Constant(Literal::new_typed_literal("0.0E0", Double.iri())),
            value_op: None,
        },
        catalog,
        actuals,
    )
}

#[test]
fn canonical_constants_bind_integer_coefficients_and_integer_zero_signs() {
    for key in [NativeScalarKey::MysqlFloat4, NativeScalarKey::MysqlFloat8] {
        let (mut cmp, _, actuals) = setup(key);
        for (lexical, binding, sign) in [
            ("0.0E0", "00e-1", "0"),
            ("-0.0E0", "-00e-1", "1"),
            ("1.1E0", "11e-1", "0"),
            ("5.0E-324", "50e-325", "0"),
        ] {
            for reverse in [false, true] {
                let constant =
                    LiteralOperand::Constant(Literal::new_typed_literal(lexical, Double.iri()));
                let column = LiteralOperand::Column {
                    column: ColRef::new(0, "v"),
                    spec: TermSpec::typed_literal(Double.iri().into_owned()),
                };
                (cmp.left, cmp.right) = if reverse {
                    (constant, column)
                } else {
                    (column, constant)
                };
                let mut params = vec![];
                let mut index = 0;
                let sql = comparison(&cmp, Dialect::MySql, &actuals, &mut params, &mut index)
                    .unwrap()
                    .unwrap();
                assert_eq!(params, [binding]);
                assert_eq!(index, 1);
                assert!(sql.contains(&format!("JSON_ARRAY(0,CAST(? AS DOUBLE),{sign})")));
                let raw = colref(&ColRef::new(0, "v"), Dialect::MySql, &actuals);
                assert!(sql.contains(&format!("CASE WHEN {raw} IS NULL THEN 2 ELSE 0 END")));
                assert!(sql.contains("JSON_UNQUOTE"));
                Dialect::MySql
                    .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                    .unwrap();
            }
        }
    }
}

#[test]
fn noncanonical_and_nonfinite_constants_cannot_gain_numeric_identity() {
    let (mut cmp, _, actuals) = setup(NativeScalarKey::MysqlFloat8);
    for lexical in [
        "0", "-0", "0e0", "1.10E0", "+1.1E0", "NaN", "INF", "-INF", "inf", "oops",
    ] {
        cmp.right = LiteralOperand::Constant(Literal::new_typed_literal(lexical, Double.iri()));
        let mut params = vec![];
        let sql = comparison(&cmp, Dialect::MySql, &actuals, &mut params, &mut 0)
            .unwrap()
            .unwrap();
        assert!(params.is_empty());
        assert!(sql.contains("JSON_ARRAY(1,CAST(0 AS DOUBLE),0)"));
        assert!(
            sql.contains("THEN NULL"),
            "missing columns must remain expression errors"
        );
    }
}

#[test]
fn same_decoder_identity_does_not_rebuild_shortest_lexicals() {
    for key in [NativeScalarKey::MysqlFloat4, NativeScalarKey::MysqlFloat8] {
        let (mut cmp, _, mut actuals) = setup(key);
        actuals.insert(1, actuals[&0].clone());
        cmp.right = LiteralOperand::Column {
            column: ColRef::new(1, "v"),
            spec: TermSpec::typed_literal(Double.iri().into_owned()),
        };
        let sql = comparison(&cmp, Dialect::MySql, &actuals, &mut vec![], &mut 0)
            .unwrap()
            .unwrap();
        assert!(
            !sql.contains("WITH"),
            "same-width identity is injective in raw fields"
        );
        assert!(sql.contains("THEN NULL"));
        assert!(
            sql.contains("JSON_UNQUOTE"),
            "preserve original negative zero"
        );
        actuals.get_mut(&1).unwrap().scalar_columns.insert(
            "v".into(),
            if key == NativeScalarKey::MysqlFloat4 {
                NativeScalarKey::MysqlFloat8
            } else {
                NativeScalarKey::MysqlFloat4
            },
        );
        assert!(
            comparison(&cmp, Dialect::MySql, &actuals, &mut vec![], &mut 0)
                .unwrap()
                .unwrap()
                .contains("WITH")
        );
        cmp.right = cmp.left.clone();
        let sql = comparison(&cmp, Dialect::MySql, &actuals, &mut vec![], &mut 0)
            .unwrap()
            .unwrap();
        assert!(sql.contains("IS NULL THEN NULL ELSE TRUE"));
        assert!(!sql.contains("JSON") && !sql.contains("WITH"));
    }
}

#[test]
fn identity_requires_retained_width_datatype_and_matching_constructor() {
    let (mut cmp, catalog, mut actuals) = setup(NativeScalarKey::MysqlFloat8);
    for datatype in [None, Some(sf_core::datatype::XsdTypeCode::String)] {
        actuals
            .get_mut(&0)
            .unwrap()
            .datatype_columns
            .insert("v".into(), datatype);
        assert!(comparison(&cmp, Dialect::MySql, &actuals, &mut vec![], &mut 0).is_err());
    }
    actuals
        .get_mut(&0)
        .unwrap()
        .datatype_columns
        .insert("v".into(), Some(Double));
    for key in [None, Some(NativeScalarKey::PostgresFloat8)] {
        actuals.get_mut(&0).unwrap().scalar_columns.clear();
        if let Some(key) = key {
            actuals
                .get_mut(&0)
                .unwrap()
                .scalar_columns
                .insert("v".into(), key);
        }
        assert!(comparison(&cmp, Dialect::MySql, &actuals, &mut vec![], &mut 0).is_err());
    }
    actuals
        .get_mut(&0)
        .unwrap()
        .scalar_columns
        .insert("v".into(), NativeScalarKey::MysqlFloat8);
    let LiteralOperand::Column { spec, .. } = &mut cmp.left else {
        unreachable!()
    };
    spec.datatype = Some(sf_core::NamedNode::new_unchecked(
        "http://www.w3.org/2001/XMLSchema#float",
    ));
    assert!(
        comparison(&cmp, Dialect::MySql, &actuals, &mut vec![], &mut 0)
            .unwrap()
            .is_none()
    );
    // A natural Double peer still intercepts mixed constructors and rejects
    // the unqualified override rather than inheriting its raw SQL equality.
    cmp.right = LiteralOperand::Column {
        column: ColRef::new(0, "v"),
        spec: TermSpec::plain_literal(),
    };
    assert!(comparison(&cmp, Dialect::MySql, &actuals, &mut vec![], &mut 0).is_err());
    assert!(
        comparison(&cmp, Dialect::Postgres, &actuals, &mut vec![], &mut 0)
            .unwrap()
            .is_none()
    );
    assert!(!native_literal_key::qualified_source(
        Double,
        Some(NativeScalarKey::MysqlFloat8),
        None
    ));
    assert!(iri_cmp::scalar_lexical(NativeScalarKey::MysqlFloat8, "v", Dialect::MySql).is_ok());
    assert!(!catalog.datatypes_by_source.is_empty());
}

#[test]
fn partition_keys_preserve_fields_without_granting_iri_or_pooled_authority() {
    for key in [NativeScalarKey::MysqlFloat4, NativeScalarKey::MysqlFloat8] {
        let (_, _, mut actuals) = setup(key);
        let c = ColRef::new(0, "v");
        for mode in [
            LexicalMode::Natural,
            LexicalMode::Decoded,
            LexicalMode::DecodedWithNatural,
            LexicalMode::TypedLiteral {
                datatype: Double.iri().into_owned(),
            },
        ] {
            let modes = [LexicalKey {
                column: "v".into(),
                mode,
            }];
            assert_eq!(decoder_key(&c, &actuals, &modes), Some(key));
            let partition = pg_numeric::key(&c, Dialect::MySql, &actuals, &modes).unwrap();
            assert!(
                partition.starts_with(&format!("{}, CASE", colref(&c, Dialect::MySql, &actuals)))
            );
        }
        assert_eq!(
            decoder_key(
                &c,
                &actuals,
                &[LexicalKey {
                    column: "v".into(),
                    mode: LexicalMode::Iri { base: None }
                }]
            ),
            None
        );
        actuals.get_mut(&0).unwrap().scalar_columns.clear();
        assert_eq!(
            decoder_key(
                &c,
                &actuals,
                &[LexicalKey {
                    column: "v".into(),
                    mode: LexicalMode::Natural
                }]
            ),
            None
        );
    }
}

#[test]
fn mysql_distinct_order_and_pooled_output_have_dialect_boundaries() {
    let maps = sf_mapping::parse_r2rml(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/item>;
      rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "v"]]."#,
    )
    .unwrap();
    let (_, catalog, _) = setup(NativeScalarKey::MysqlFloat8);
    for order in ["ASC", "DESC"] {
        let plan = crate::parse_and_translate(
            &format!("SELECT DISTINCT ?o WHERE {{ ?s <http://ex/p> ?o }} ORDER BY {order}(?o)"),
            &maps,
            Dialect::MySql,
        )
        .unwrap();
        let mut branches = plan.prepared_branches();
        branches[0].order.push(OrderKey {
            var: "o".into(),
            descending: order == "DESC",
            expr: None,
        });
        let sql = emit_branch_with(&branches[0], Dialect::MySql, &catalog)
            .unwrap()
            .sql;
        assert!(sql.contains("ROW_NUMBER()"));
        assert!(sql.contains("ORDER BY"));
        assert!(!sql.contains("NULLS FIRST") && !sql.contains("NULLS LAST"));
        branches.push(branches[0].clone());
        assert!(validate_union(&branches, Dialect::MySql, &catalog).is_err());
        assert!(validate_union(&branches, Dialect::Postgres, &catalog).is_ok());
        // An in-arm floating validation guard is not a consumed pooled field.
        // Constant-only output must not acquire a blanket native-float veto.
        for branch in &mut branches {
            branch.bindings.clear();
            branch
                .where_conds
                .push(SqlCond::DecodedIsNotNull(ColRef::new(0, "v")));
        }
        assert!(validate_union(&branches, Dialect::MySql, &catalog).is_ok());
    }
}

#[test]
fn mysql_typed_float_partition_retains_native_payload() {
    let maps = sf_mapping::parse_r2rml(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/item>;
      rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "v"; rr:datatype <http://www.w3.org/2001/XMLSchema#float>]]."#).unwrap();
    let (_, catalog, _) = setup(NativeScalarKey::MysqlFloat4);
    let plan = crate::parse_and_translate(
        "SELECT ?o WHERE { <http://ex/item> <http://ex/p> ?o }",
        &maps,
        Dialect::MySql,
    )
    .unwrap();
    let sql = emit_subplan_sql(&plan, Dialect::MySql, &catalog).unwrap().0;
    assert!(sql.contains("ROW_NUMBER()"));
}

#[test]
fn native_float_template_joins_keep_width_and_source_key_semantics() {
    for key in [NativeScalarKey::MysqlFloat4, NativeScalarKey::MysqlFloat8] {
        let (_, catalog, mut actuals) = setup(key);
        actuals.insert(1, actuals[&0].clone());
        let (a, b) = (ColRef::new(0, "v"), ColRef::new(1, "v"));
        for cond in [
            SqlCond::ColEq(a.clone(), b.clone()),
            SqlCond::NullSafeEq(a.clone(), b.clone()),
        ] {
            let sql = render_cond(
                &cond,
                Dialect::MySql,
                &catalog,
                &actuals,
                &mut vec![],
                &mut 0,
            )
            .unwrap();
            assert!(sql.contains("JSON_UNQUOTE"));
            assert!(!sql.contains("WITH"));
        }
        let native = render_cond(
            &SqlCond::NativeColEq(a.clone(), b.clone()),
            Dialect::MySql,
            &catalog,
            &actuals,
            &mut vec![],
            &mut 0,
        )
        .unwrap();
        assert!(
            !native.contains("JSON"),
            "foreign keys retain SQL value semantics"
        );
        for peer in [
            None,
            Some(NativeScalarKey::Integer),
            Some(NativeScalarKey::PostgresFloat4),
        ] {
            actuals.get_mut(&1).unwrap().scalar_columns.clear();
            if let Some(peer) = peer {
                actuals
                    .get_mut(&1)
                    .unwrap()
                    .scalar_columns
                    .insert("v".into(), peer);
            }
            assert!(key_equality(&a, &b, Dialect::MySql, &actuals).is_err());
        }
        let peer = if key == NativeScalarKey::MysqlFloat4 {
            NativeScalarKey::MysqlFloat8
        } else {
            NativeScalarKey::MysqlFloat4
        };
        actuals
            .get_mut(&1)
            .unwrap()
            .scalar_columns
            .insert("v".into(), peer);
        let sql = key_equality(&a, &b, Dialect::MySql, &actuals)
            .unwrap()
            .unwrap();
        assert!(sql.contains("JSON_UNQUOTE"));
        assert!(sql.contains("WITH"));
        assert!(sql.contains("utf8mb4_0900_bin"));
        Dialect::MySql
            .emit_via_ast(&format!("SELECT {sql} FROM items t0, items t1"))
            .unwrap();
        let native = render_cond(
            &SqlCond::NativeColEq(a.clone(), b.clone()),
            Dialect::MySql,
            &catalog,
            &actuals,
            &mut vec![],
            &mut 0,
        )
        .unwrap();
        assert!(
            !native.contains("JSON"),
            "mixed-width FK equality stays native"
        );
        actuals.get_mut(&1).unwrap().datatype_columns.clear();
        assert!(key_equality(&a, &b, Dialect::MySql, &actuals).is_err());
    }
}

#[test]
fn float_template_fallback_and_rendered_pool_cannot_bypass_authority() {
    use sf_core::ir::{Template, TermSpec};
    let template = Template::parse("http://ex/{v}").unwrap();
    for key in [NativeScalarKey::MysqlFloat4, NativeScalarKey::MysqlFloat8] {
        let (_, catalog, actuals) = setup(key);
        let cond = SqlCond::TemplateEq(
            template.segments().to_vec(),
            0,
            template.segments().to_vec(),
            0,
            true,
        );
        let comparison = render_cond(
            &cond,
            Dialect::MySql,
            &catalog,
            &actuals,
            &mut vec![],
            &mut 0,
        );
        let input = Scan {
            alias: 0,
            source: LogicalSource::Table("items".into()).into(),
        };
        let mut branch = Branch::single(Scan {
            alias: 1,
            source: ScanSource::Projection {
                input: Box::new(input),
                columns: vec![(
                    "rendered".into(),
                    TermMap::Template(template.clone(), TermSpec::iri()),
                )],
                guards: vec![],
                distinct: false,
                native_keys: vec![],
                lexical_keys: vec![],
            },
        });
        branch.bindings.insert(
            "o".into(),
            TermDef::Derived {
                term_map: TermMap::Column("rendered".into(), TermSpec::iri()),
                alias: 1,
            },
        );
        let rendered = scan_ref(
            &branch.core[0],
            Dialect::MySql,
            &catalog,
            &mut vec![],
            &mut 0,
        );
        for result in [comparison, rendered] {
            let sql = result.unwrap();
            assert!(sql.contains(if key == NativeScalarKey::MysqlFloat4 {
                "__sf_short_selected"
            } else {
                "__sf_double_corrected"
            }));
        }
        assert!(validate_union(&[branch.clone()], Dialect::MySql, &catalog).is_ok());
        assert!(
            validate_union(&[branch.clone(), branch.clone()], Dialect::MySql, &catalog).is_ok(),
            "decoder-qualified static IRIs are text before native pooling"
        );
        let mut unqualified = branch.clone();
        let ScanSource::Projection { columns, .. } = &mut unqualified.core[0].source else {
            unreachable!()
        };
        columns[0].1 = TermMap::Template(template.clone(), TermSpec::plain_literal());
        assert!(
            validate_union(&[branch, unqualified], Dialect::MySql, &catalog).is_err(),
            "unqualified rendering still cannot conceal floating lineage"
        );
    }
}

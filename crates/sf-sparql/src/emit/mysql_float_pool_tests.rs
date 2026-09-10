use super::*;
use crate::iq::{scan::LexicalMode, CmpOp, LexicalKey, Scan, ScanSource};
use sf_core::ir::{Template, TermSpec};
use sf_sql::backend::ResultColumn;
fn fixture() -> (LogicalSource, ColumnCatalog) {
    use sf_core::datatype::XsdTypeCode::{Double, Integer, String as Text};
    use NativeScalarKey::{MysqlFloat4, MysqlFloat8, MysqlTime, PostgresFloat4};
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    let descriptors = [
        ("id", Some(Integer), Some(NativeScalarKey::Integer), None),
        ("w", Some(Double), Some(MysqlFloat8), None),
        ("v", Some(Double), Some(MysqlFloat4), None),
        ("blank", Some(Text), None, Some(TextKey::Verbatim)),
        ("clock", None, Some(MysqlTime), None),
        ("foreign", None, Some(PostgresFloat4), None),
    ];
    catalog
        .insert_live_result(
            &source,
            descriptors
                .into_iter()
                .map(
                    |(name, natural_datatype, native_scalar, text_key)| ResultColumn {
                        name: name.into(),
                        natural_datatype,
                        native_scalar,
                        text_key,
                        sqlite_decode: None,
                    },
                )
                .collect(),
        )
        .unwrap();
    (source, catalog)
}
fn projection(source: &LogicalSource, term_map: TermMap) -> Scan {
    Scan {
        alias: 1,
        source: ScanSource::Projection {
            input: Box::new(Scan {
                alias: 0,
                source: source.clone().into(),
            }),
            columns: vec![("rendered".into(), term_map)],
            guards: vec![],
            distinct: false,
            native_keys: vec![],
            lexical_keys: vec![],
        },
    }
}

#[test]
fn mysql_lexical_only_floats_cross_policy_barriers_as_text_not_native_values() {
    let (source, catalog) = fixture();
    for raw in ["v", "w"] {
        let mut scan = projection(
            &source,
            TermMap::Column(raw.into(), TermSpec::plain_literal()),
        );
        let ScanSource::Projection {
            lexical_keys,
            guards,
            ..
        } = &mut scan.source
        else {
            unreachable!()
        };
        lexical_keys.push(LexicalKey {
            column: raw.into(),
            mode: LexicalMode::Decoded,
        });
        guards.push(SqlCond::NativeCmp(
            ColRef::new(0, "blank"),
            CmpOp::Eq,
            "allowed".into(),
        ));
        let actuals = scan_actuals(&scan, Dialect::MySql, &catalog);
        assert_eq!(
            actuals.text_columns.get("rendered"),
            Some(&TextKey::Verbatim)
        );
        assert_eq!(
            actuals.datatype_columns.get("rendered"),
            Some(&Some(sf_core::datatype::XsdTypeCode::String))
        );
        assert!(!actuals.scalar_columns.contains_key("rendered"));
        assert!(!actuals.natural_columns.contains_key("rendered"));
        let columns = HashMap::from([(1, actuals.clone())]);
        let rendered = ColRef::new(1, "rendered");
        assert!(iri_cmp::unreserved_column(&rendered, &columns));
        let encoded =
            iri_cmp::encode_text_column(&rendered, Dialect::MySql, &catalog, &columns).unwrap();
        assert!(
            !encoded.contains("JSON_TABLE"),
            "bounded numeric text has no escaping work"
        );
        let mut params = vec![];
        let sql = scan_ref(&scan, Dialect::MySql, &catalog, &mut params, &mut 0).unwrap();
        let renderer = if raw == "v" {
            "__sf_short_selected"
        } else {
            "__sf_double_corrected"
        };
        assert!(sql.contains(renderer));
        assert!(
            sql.contains("JSON_ARRAY(v)"),
            "zero sign must be captured before projection"
        );
        let output = sql.rfind("AS `rendered`").unwrap();
        assert!(sql[..output].contains(renderer));
        assert!(
            sql[..output].contains("USING utf8mb4) COLLATE utf8mb4_0900_bin"),
            "binary shortest digits must be projected as text, not driver-decoded hex"
        );
        assert!(sql[output..].contains("LIMIT 18446744073709551615"));
        assert_eq!(params, ["allowed"]);
        let copied = ref_atom::actuals(
            &Branch::single(scan.clone()),
            &[ColRef::new(1, "rendered")],
            Dialect::MySql,
            &catalog,
        );
        assert_eq!(copied.text_columns.get("c0"), Some(&TextKey::Verbatim));
        assert!(!copied.scalar_columns.contains_key("c0"));
        assert!(copied.iri_unreserved_columns.contains("c0"));

        for mode in [
            LexicalMode::Natural,
            LexicalMode::DecodedWithNatural,
            LexicalMode::Iri { base: None },
            LexicalMode::TypedLiteral {
                datatype: sf_core::datatype::XsdTypeCode::Double.iri().into_owned(),
            },
        ] {
            for column in [raw.to_owned(), raw.to_uppercase()] {
                let mut mixed = scan.clone();
                let ScanSource::Projection { lexical_keys, .. } = &mut mixed.source else {
                    unreachable!()
                };
                lexical_keys.push(LexicalKey {
                    column: column.into(),
                    mode: mode.clone(),
                });
                let actuals = scan_actuals(&mixed, Dialect::MySql, &catalog);
                assert!(!actuals.text_columns.contains_key("rendered"));
                assert!(actuals.scalar_columns.contains_key("rendered"));
            }
        }
        for both in [false, true] {
            let mut native = scan.clone();
            let ScanSource::Projection { native_keys, .. } = &mut native.source else {
                unreachable!()
            };
            native_keys.push(("rendered".into(), both));
            let actuals = scan_actuals(&native, Dialect::MySql, &catalog);
            assert!(!actuals.text_columns.contains_key("rendered"));
            assert!(actuals.scalar_columns.contains_key("rendered"));
        }
    }
    for raw in ["missing", "foreign"] {
        let mut scan = projection(
            &source,
            TermMap::Column(raw.into(), TermSpec::plain_literal()),
        );
        let ScanSource::Projection { lexical_keys, .. } = &mut scan.source else {
            unreachable!()
        };
        lexical_keys.push(LexicalKey {
            column: raw.into(),
            mode: LexicalMode::Decoded,
        });
        assert!(!scan_actuals(&scan, Dialect::MySql, &catalog)
            .text_columns
            .contains_key("rendered"));
    }
    let raw = HashMap::from([(0, source_actuals(&source, &catalog))]);
    assert!(!iri_cmp::unreserved_column(&ColRef::new(0, "blank"), &raw));
    assert!(
        iri_cmp::encode_text_column(&ColRef::new(0, "blank"), Dialect::MySql, &catalog, &raw)
            .unwrap()
            .contains("JSON_TABLE")
    );
}

#[test]
fn mysql_rendered_pool_public_filter_retains_iri_authority() {
    let (_, catalog) = fixture();
    let maps = sf_mapping::parse_r2rml(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#m> rr:logicalTable [rr:tableName "items"];
        rr:subjectMap [rr:template "http://ex/s/{id}"];
        rr:predicateObjectMap [rr:predicate <http://ex/p4>; rr:objectMap [rr:template "http://ex/{v}/"]];
        rr:predicateObjectMap [rr:predicate <http://ex/p8>; rr:objectMap [rr:template "http://ex/{w}/{blank}"]]."#).unwrap();
    let mut optional = crate::parse_and_translate(
        "SELECT ?s ?o WHERE { VALUES ?s { <http://ex/s/0> <http://ex/s/1> } OPTIONAL { ?s <http://ex/p4> ?o FILTER(?o = <http://ex/1.1/>) } }", &maps, Dialect::MySql).unwrap();
    assert_eq!(optional.branches.len(), 2);
    for branch in &mut optional.branches {
        let join = &mut branch.opts[0];
        let ScanSource::Projection { guards, .. } = &mut join.scan.source else {
            unreachable!()
        };
        assert!(guards.is_empty());
        guards.push(SqlCond::NativeCmp(
            ColRef::new(0, "blank"),
            CmpOp::Eq,
            "allowed".into(),
        ));
        let restricted = iri_cmp::restrict_optional(join, Dialect::MySql, &catalog);
        let ScanSource::Projection { guards, .. } = &restricted.source else {
            unreachable!()
        };
        assert_eq!(
            guards.len(),
            2,
            "only policy plus total integer identity, never floating filter"
        );
        assert!(
            matches!(guards[0], SqlCond::NativeCmp(..)) && matches!(guards[1], SqlCond::IriCmp(_))
        );
        assert!(matches!(
            iri_cmp::restrict_optional(join, Dialect::Postgres, &catalog),
            std::borrow::Cow::Borrowed(_)
        ));
        assert!(join
            .extra
            .iter()
            .any(|guard| matches!(guard, SqlCond::IriCmp(_))));
    }
    for filter in ["?o = <http://ex/1.1/>", "sameTerm(?o, <http://ex/1.1/>)"] {
        let query = format!(
            "SELECT ?s WHERE {{ {{ SELECT ?s ?o WHERE {{ {{ ?s <http://ex/p4> ?o }} UNION {{ ?s <http://ex/p8> ?o }} }} }} FILTER({filter}) }}"
        );
        let plan = crate::parse_and_translate(&query, &maps, Dialect::MySql).unwrap();
        let (sql, _) = emit_subplan_sql(&plan, Dialect::MySql, &catalog).unwrap();
        assert!(sql.contains("UNION ALL"));
    }
    let plan = crate::parse_and_translate(
        "SELECT ?s WHERE { { SELECT ?s ?o WHERE { { ?s <http://ex/p4> ?o } UNION { ?s <http://ex/p8> ?o } } OFFSET 0 } FILTER(?o = <http://ex/1.1/>) }",
        &maps, Dialect::MySql,
    ).unwrap();
    assert!(
        plan.branches
            .iter()
            .any(|b| b.subplan_joins.iter().any(|s| s.plan.branches.len() > 1)),
        "public regression must consume a pooled SubPlan"
    );
    assert_eq!(plan.branches.len(), 1);
    let outer = &plan.branches[0];
    assert_eq!(outer.subplan_joins.len(), 1);
    let nested = &outer.subplan_joins[0];
    assert_eq!(nested.plan.branches.len(), 2);
    let nested_actuals = subplan_actuals(&nested.plan, Dialect::MySql, &catalog);
    assert_eq!(nested_actuals.static_iri_columns.len(), 2);
    assert_eq!(
        branch_actuals(outer, Dialect::MySql, &catalog)[&nested.alias].static_iri_columns,
        nested_actuals.static_iri_columns
    );
    assert!(plan.source_sized_states().is_empty());
    assert!(emit_subplan_sql(&plan, Dialect::MySql, &catalog)
        .unwrap()
        .0
        .contains("UNION ALL"));
    let distinct = crate::parse_and_translate(
        "SELECT DISTINCT ?o WHERE { { SELECT ?s ?o WHERE { { ?s <http://ex/p4> ?o } UNION { ?s <http://ex/p8> ?o } } } }",
        &maps, Dialect::MySql,
    ).unwrap();
    assert!(
        distinct
            .source_sized_states()
            .contains(&crate::resource_profile::SourceSizedState::ProjectedDistinct),
        "existing source-sized DISTINCT admission remains unchanged"
    );
}

#[test]
fn mysql_static_template_authority_requires_every_slot_and_encodes_once() {
    let (source, catalog) = fixture();
    let live = source_actuals(&source, &catalog);
    let qualified = Template::parse("http://ex/{v}/{blank}").unwrap();
    assert!(iri_cmp::qualified_static_template(
        &qualified,
        &TermSpec::iri(),
        Dialect::MySql,
        &live
    ));
    for (template, spec) in [
        (qualified.clone(), TermSpec::iri().with_base("http://base/")),
        (qualified.clone(), TermSpec::plain_literal()),
        (
            Template::parse("http://ex/{v}/{missing}").unwrap(),
            TermSpec::iri(),
        ),
        (
            Template::parse("http://ex/{v}/{foreign}").unwrap(),
            TermSpec::iri(),
        ),
    ] {
        assert!(!iri_cmp::qualified_static_template(
            &template,
            &spec,
            Dialect::MySql,
            &live
        ));
        let unqualified = Branch::single(projection(&source, TermMap::Template(template, spec)));
        assert!(
            identity::validate_union(
                &[unqualified.clone(), unqualified],
                Dialect::MySql,
                &catalog,
            )
            .is_err(),
            "a valid float slot cannot license other slots or late bases"
        );
    }

    let scan = projection(&source, TermMap::Template(qualified, TermSpec::iri()));
    let output = scan_actuals(&scan, Dialect::MySql, &catalog);
    assert_eq!(
        output.static_iri_columns,
        HashSet::from(["rendered".into()])
    );
    let sql = scan_ref(&scan, Dialect::MySql, &catalog, &mut vec![], &mut 0).unwrap();
    assert!(sql.contains("__sf_short_selected"), "{sql}");
    assert!(sql.contains("utf8mb4_0900_bin"), "{sql}");

    let temporal = projection(
        &source,
        TermMap::Template(
            Template::parse("http://ex/{clock}").unwrap(),
            TermSpec::iri(),
        ),
    );
    let sql = scan_ref(&temporal, Dialect::MySql, &catalog, &mut vec![], &mut 0).unwrap();
    assert!(sql.contains("'%3A'"), "{sql}");
    assert!(
        !sql.contains("'%253A'"),
        "temporal slot encoded twice: {sql}"
    );
    assert!(
        !sql.contains("JSON_TABLE"),
        "scalar alphabet needs no byte walk"
    );

    let literal = projection(
        &source,
        TermMap::Template(
            Template::parse("{clock}").unwrap(),
            TermSpec::plain_literal(),
        ),
    );
    let sql = scan_ref(&literal, Dialect::MySql, &catalog, &mut vec![], &mut 0).unwrap();
    assert!(
        !sql.contains("SUBSTRING_INDEX"),
        "literal spelling changed: {sql}"
    );

    let raw = projection(&source, TermMap::Column("blank".into(), TermSpec::iri()));
    assert!(scan_actuals(&raw, Dialect::MySql, &catalog)
        .static_iri_columns
        .is_empty());
    let raw_actuals = HashMap::from([(1, scan_actuals(&raw, Dialect::MySql, &catalog))]);
    assert!(iri_cmp::column(
        &ColRef::new(1, "rendered"),
        None,
        Dialect::MySql,
        &catalog,
        &raw_actuals,
        &mut vec![],
        &mut 0,
    )
    .is_err());

    let mut renamed = projection(&source, TermMap::Column("rendered".into(), TermSpec::iri()));
    renamed.alias = 2;
    let ScanSource::Projection { input, .. } = &mut renamed.source else {
        unreachable!()
    };
    **input = scan;
    assert!(iri_cmp::static_iri_name(
        "rendered",
        &scan_actuals(&renamed, Dialect::MySql, &catalog)
    ));
    assert_eq!(
        ref_atom::actuals(
            &Branch::single(renamed),
            &[ColRef::new(2, "rendered")],
            Dialect::MySql,
            &catalog,
        )
        .static_iri_columns,
        HashSet::from(["c0".into()])
    );
}

#[test]
fn mysql_static_iri_lineage_intersects_across_union_and_licenses_column_identity() {
    let (source, catalog) = fixture();
    let template = Template::parse("http://ex/{v}/{blank}").unwrap();
    let scan = projection(
        &source,
        TermMap::Template(template.clone(), TermSpec::iri()),
    );
    let mut branch = Branch::single(scan.clone());
    branch.bindings.insert(
        "o".into(),
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Column("rendered".into(), TermSpec::iri()),
        },
    );
    let maps = sf_mapping::parse_r2rml(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
        rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "v"]]."#,
    )
    .unwrap();
    let mut plan = crate::parse_and_translate(
        "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
        &maps,
        Dialect::MySql,
    )
    .unwrap();
    plan.branches = vec![branch.clone(), branch.clone()];
    assert!(identity::validate_union(&plan.branches, Dialect::MySql, &catalog).is_ok());
    let (sql, _) = emit_subplan_sql(&plan, Dialect::MySql, &catalog).unwrap();
    assert!(sql.contains("UNION ALL"));
    assert!(sql.matches("utf8mb4_0900_bin").count() >= 2);
    let pooled = subplan_actuals(&plan, Dialect::MySql, &catalog);
    assert_eq!(pooled.static_iri_columns, HashSet::from(["c0".into()]));
    assert!(pooled.iri_unreserved_columns.is_empty());
    let actuals = HashMap::from([(9, pooled)]);
    let mut params = vec![];
    let sql = iri_cmp::column(
        &ColRef::new(9, "c0"),
        None,
        Dialect::MySql,
        &catalog,
        &actuals,
        &mut params,
        &mut 0,
    )
    .unwrap();
    assert!(sql.contains("utf8mb4_0900_bin"), "{sql}");
    assert!(params.is_empty());

    let through_ref = |plan: &crate::Plan| {
        let mut branch = Branch::empty();
        branch.subplan_joins.push(crate::iq::SubPlanJoin {
            alias: 9,
            plan: Box::new(plan.clone()),
            on: vec![],
            left: false,
        });
        ref_atom::actuals(&branch, &[ColRef::new(9, "c0")], Dialect::MySql, &catalog)
    };
    assert_eq!(
        through_ref(&plan).static_iri_columns,
        HashSet::from(["c0".into()])
    );

    let late = sf_mapping::parse_r2rml(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#m> rr:logicalTable [rr:tableName "items"];
        rr:subjectMap [rr:template "http://{v}/{blank}"] ."#,
    )
    .unwrap();
    assert!(matches!(&late[0].subject.term, TermMap::Template(_, spec) if spec.base.is_some()));
    for term in [
        TermMap::Template(template, TermSpec::plain_literal()),
        TermMap::Template(
            Template::parse("http://ex/{v}/{missing}").unwrap(),
            TermSpec::iri(),
        ),
        TermMap::Template(
            Template::parse("http://ex/{v}/{foreign}").unwrap(),
            TermSpec::iri(),
        ),
        late[0].subject.term.clone(),
    ] {
        let ScanSource::Projection { columns, .. } = &mut branch.core[0].source else {
            unreachable!()
        };
        columns[0].1 = term;
        plan.branches[1] = branch.clone();
        assert!(identity::validate_union(&plan.branches, Dialect::MySql, &catalog).is_err());
        assert!(subplan_actuals(&plan, Dialect::MySql, &catalog)
            .static_iri_columns
            .is_empty());
        assert!(
            through_ref(&plan).static_iri_columns.is_empty(),
            "RefAtom cannot revive a lost pool proof"
        );
    }
}

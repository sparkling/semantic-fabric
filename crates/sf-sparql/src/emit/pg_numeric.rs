//! Native NUMERIC payloads and decoded RDF lexical keys are separate authorities.
use super::*;
use crate::iq::scan::LexicalMode;

pub(super) fn lexical_mode(mode: &LexicalMode) -> bool {
    matches!(mode, LexicalMode::Decoded | LexicalMode::DecodedWithNatural)
}

pub(super) fn native_companion(
    column: &ColRef,
    dialect: Dialect,
    actuals: &ActualColumns,
    modes: &[crate::iq::LexicalKey],
) -> bool {
    if dialect != Dialect::Postgres {
        return false;
    }
    match iri_cmp::scalar_column(column, actuals) {
        Some(
            NativeScalarKey::Integer
            | NativeScalarKey::PostgresBoolean
            | NativeScalarKey::PostgresBytea,
        ) => modes.iter().any(|key| {
            key.column == column.column
                && (lexical_mode(&key.mode) || key.mode == LexicalMode::Natural)
        }),
        Some(NativeScalarKey::PostgresNumeric) => modes
            .iter()
            .any(|key| key.column == column.column && key.mode == LexicalMode::Natural),
        _ => false,
    }
}

pub(super) fn is_numeric(column: &ColRef, dialect: Dialect, actuals: &ActualColumns) -> bool {
    dialect == Dialect::Postgres
        && iri_cmp::scalar_column(column, actuals) == Some(NativeScalarKey::PostgresNumeric)
}

pub(super) fn lexical(raw: &str) -> String {
    // PostgreSQL JSON, unlike JSONB, preserves its validated input text.
    format!("CAST(CAST(CAST({raw} AS TEXT) AS JSON) AS TEXT)")
}

pub(super) fn validates(condition: &SqlCond, actuals: &ActualColumns) -> bool {
    match condition {
        SqlCond::DecodedIsNotNull(column) => is_numeric(column, Dialect::Postgres, actuals),
        SqlCond::IriCmp(cmp) => cmp
            .columns()
            .any(|c| is_numeric(c, Dialect::Postgres, actuals)),
        SqlCond::Not(inner) => validates(inner, actuals),
        SqlCond::And(cs) | SqlCond::Or(cs) => cs.iter().any(|c| validates(c, actuals)),
        _ => false,
    }
}

pub(super) fn key(
    column: &ColRef,
    dialect: Dialect,
    actuals: &ActualColumns,
    modes: &[crate::iq::LexicalKey],
) -> Option<String> {
    (is_numeric(column, dialect, actuals)
        && modes
            .iter()
            .any(|key| key.column == column.column && lexical_mode(&key.mode)))
    .then(|| path_comparison::exact_text(lexical(&colref(column, dialect, actuals)), dialect))
}

pub(super) fn distinct_keys(b: &Branch, dialect: Dialect, actuals: &ActualColumns) -> Vec<bool> {
    if !actuals.values().any(|a| {
        a.scalar_columns
            .values()
            .any(|k| *k == NativeScalarKey::PostgresNumeric)
    }) {
        return vec![false; b.projection().len()];
    }
    let modes: HashMap<_, _> = actuals
        .keys()
        .map(|alias| {
            (
                *alias,
                crate::cascade::distinct_scan::binding_lexical_keys(b, *alias),
            )
        })
        .collect();
    b.projection()
        .iter()
        .map(|column| {
            is_numeric(column, dialect, actuals)
                && modes.get(&column.alias).is_some_and(|keys| {
                    keys.iter()
                        .any(|key| key.column == column.column && lexical_mode(&key.mode))
                })
        })
        .collect()
}

/// Already-compatible pooled arms still need decoded numeric keys across arms.
/// No arm may lend a pre-coercion key recipe to a different native result type.
pub(super) fn union_keys(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<Option<Vec<bool>>> {
    if dialect != Dialect::Postgres {
        return Ok(None);
    }
    let keys: Vec<_> = branches
        .iter()
        .map(|b| distinct_keys(b, dialect, &branch_actuals(b, dialect, catalog)))
        .collect();
    if !keys.iter().flatten().any(|key| *key) {
        return Ok(None);
    }
    if branches.iter().any(|b| b.agg.is_some() || b.path.is_some())
        || keys.iter().any(|key| key != &keys[0])
    {
        return Err(Error::Unsupported(
            "numeric UNION requires agreeing raw decoder and output consumer roles".into(),
        ));
    }
    Ok(keys.into_iter().next())
}

pub(super) fn distinct_sql(raw: String, keys: &[bool]) -> String {
    let columns = (0..keys.len())
        .map(|i| format!("__sf_numeric_raw.c{i}"))
        .collect::<Vec<_>>();
    let partition = columns
        .iter()
        .zip(keys)
        .map(|(column, decoded)| {
            if *decoded {
                path_comparison::exact_text(lexical(column), Dialect::Postgres)
            } else {
                column.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let output = (0..keys.len())
        .map(|i| format!("__sf_numeric_distinct.c{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT {output} FROM (SELECT {}, ROW_NUMBER() OVER (PARTITION BY {partition}) AS __sf_rank FROM ({raw}) __sf_numeric_raw) __sf_numeric_distinct WHERE __sf_numeric_distinct.__sf_rank = 1", columns.join(", "))
}

pub(super) fn order(b: &Branch, projection: &[ColRef]) -> Result<Option<String>> {
    let mut keys = Vec::new();
    for key in &b.order {
        let column = b
            .bindings
            .get(&key.var)
            .and_then(order_column)
            .ok_or_else(|| {
                Error::Unsupported(format!(
                    "ORDER BY ?{} is not a bound rr:column term",
                    key.var
                ))
            })?;
        let index = projection
            .iter()
            .position(|c| c == &column)
            .ok_or_else(|| {
                Error::Unsupported("ORDER BY column missing from DISTINCT output".into())
            })?;
        keys.push(format!(
            "__sf_numeric_distinct.c{index} {}",
            if key.descending {
                "DESC NULLS LAST"
            } else {
                "ASC NULLS FIRST"
            }
        ));
    }
    Ok((!keys.is_empty()).then(|| format!(" ORDER BY {}", keys.join(", "))))
}

#[cfg(test)]
mod tests {
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
                        name: "src".into(),
                        native_scalar: Some(NativeScalarKey::PostgresNumeric),
                        text_key: None,
                        sqlite_decode: None,
                    },
                    sf_sql::backend::ResultColumn {
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
                        name: "src".into(),
                        native_scalar: Some(NativeScalarKey::PostgresNumeric),
                        text_key: None,
                        sqlite_decode: None,
                    },
                    sf_sql::backend::ResultColumn {
                        name: "tenant".into(),
                        native_scalar: None,
                        text_key: Some(TextKey::Verbatim),
                        sqlite_decode: None,
                    },
                    sf_sql::backend::ResultColumn {
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
                        term_map: TermMap::Template(
                            Template::parse(recipe).unwrap(),
                            TermSpec::iri(),
                        ),
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
        assert_eq!(distinct_keys(&b, Dialect::Postgres, &actuals), [true]);
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
            [false],
            "hidden IRI must not over-distinguish natural output"
        );
        assert!(
            !crate::cascade::distinct_scan::lexical_keys(&b, 0).is_empty(),
            "D1 keeps original hidden consumer"
        );
        assert_eq!(distinct_keys(&b, Dialect::MySql, &actuals), [false]);
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
}

use super::*;
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn reference_shape_matches_raw_recipe_and_sticky_boundaries() {
    use crate::iq::{ColRef, TermDef};
    use sf_core::ir::{TermMap, TermSpec};
    let mut branch = Branch::empty();
    branch.core = vec![scan("a"), scan("b")];
    branch.core[1].alias = 1;
    let column = ColRef::new(0, Box::<str>::from("key"));
    branch.where_conds.push(SqlCond::NativeColEq(
        column.clone(),
        ColRef::new(1, Box::<str>::from("key")),
    ));
    let mut def = TermDef::Derived {
        alias: 0,
        term_map: TermMap::Column("key".into(), TermSpec::iri()),
    };
    for _ in 0..64 {
        def = TermDef::Concat(vec![def]);
    }
    branch.bindings.insert("x".into(), def.clone());
    branch.bindings.insert("y".into(), def);
    let columns = vec![column];
    crate::iq::scan::ref_atom::validate_shape(&branch, &columns).unwrap();
    assert_source_boundaries(
        |control| {
            crate::emit::ref_atom::validate_shape_controlled(
                &branch,
                &columns,
                SourceWork::new(Some(control)),
            )
            .map_err(|error| match error {
                crate::Error::QueryControl(cause) => sf_sql::Error::QueryControl(cause),
                other => sf_sql::Error::Emit(other.to_string()),
            })
        },
        (),
    );
}

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
}

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
    fn consume(&self, kind: QueryCharge, units: u64) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(kind, units)?;
        if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

fn scan(name: &str) -> Scan {
    Scan {
        alias: 0,
        source: LogicalSource::Table(name.into()).into(),
    }
}

fn identities(sources: Vec<&LogicalSource>) -> Vec<*const LogicalSource> {
    sources.into_iter().map(std::ptr::from_ref).collect()
}

#[test]
fn source_inventory_preserves_duplicate_order_and_exact_sticky_boundaries() {
    let mut branch = Branch::empty();
    branch.core = vec![scan("first"), scan("repeated")];
    branch.where_conds.push(SqlCond::And(vec![
        SqlCond::Exists {
            scans: vec![scan("repeated"), scan("last")],
            conds: vec![],
        },
        SqlCond::Not(Box::new(SqlCond::NotExists {
            scans: vec![scan("first")],
            conds: vec![],
        })),
    ]));
    let branches = vec![branch];
    let expected = identities(crate::emit::live_metadata_sources(&branches));
    let control = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    assert_eq!(
        identities(live_metadata_sources_controlled(&branches, &control).unwrap()),
        expected
    );
    let total = control.budget.consumed(QueryCharge::SourceWork);
    assert_eq!(control.budget.consumed(QueryCharge::CompilerWork), 0);
    assert_eq!(
        identities(live_metadata_sources_controlled(&branches, &budget(total)).unwrap()),
        expected
    );
    assert!(matches!(
        live_metadata_sources_controlled(&branches, &budget(total - 1)),
        Err(sf_sql::Error::QueryControl(
            QueryControlError::SourceWorkExceeded
        ))
    ));
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=control.calls.load(Ordering::Relaxed) {
            let stopped = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                at,
                cause,
            };
            assert!(
                matches!(live_metadata_sources_controlled(&branches, &stopped),
                Err(sf_sql::Error::QueryControl(actual)) if actual == cause),
                "charge {at}"
            );
        }
    }
}

#[test]
fn nested_condition_inventory_uses_a_paid_iterative_stack() {
    let mut condition = SqlCond::Exists {
        scans: vec![scan("deep")],
        conds: vec![],
    };
    for _ in 0..512 {
        condition = SqlCond::Not(Box::new(condition));
    }
    let mut branch = Branch::empty();
    branch.where_conds.push(condition);
    let branches = vec![branch];
    assert_eq!(
        identities(live_metadata_sources_controlled(&branches, &budget(u64::MAX)).unwrap()),
        identities(crate::emit::live_metadata_sources(&branches))
    );
    assert!(live_metadata_sources_controlled(&branches, &budget(0)).is_err());
}

fn assert_source_boundaries<T: std::fmt::Debug + PartialEq>(
    run: impl Fn(&dyn QueryControl) -> sf_sql::Result<T>,
    expected: T,
) {
    let counted = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    assert_eq!(run(&counted).unwrap(), expected);
    let total = counted.budget.consumed(QueryCharge::SourceWork);
    assert_eq!(counted.budget.consumed(QueryCharge::CompilerWork), 0);
    assert!(total > 0);
    assert_eq!(run(&budget(total)).unwrap(), expected);
    assert!(matches!(
        run(&budget(total - 1)),
        Err(sf_sql::Error::QueryControl(
            QueryControlError::SourceWorkExceeded
        ))
    ));
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=counted.calls.load(Ordering::Relaxed) {
            let stopped = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                at,
                cause,
            };
            assert!(
                matches!(run(&stopped),
                Err(sf_sql::Error::QueryControl(actual)) if actual == cause),
                "charge {at}"
            );
        }
    }
}

#[test]
fn source_membership_preserves_variant_identity_and_first_encounter_order() {
    let sources = [
        LogicalSource::Table("z".into()),
        LogicalSource::Query("z".into()),
        LogicalSource::Table("a".into()),
        LogicalSource::Table("z".into()),
        LogicalSource::Query("z".into()),
        LogicalSource::Query("α".into()),
        LogicalSource::Table("α".into()),
        LogicalSource::Table("".into()),
    ];
    assert_source_boundaries(
        |control| {
            let mut seen = SourceSet::default();
            let mut first = Vec::new();
            for (index, source) in sources.iter().enumerate() {
                if seen.insert(source, control)? {
                    first.push(index);
                }
            }
            Ok(first)
        },
        vec![0, 1, 2, 5, 6, 7],
    );
}

#[test]
fn controlled_probes_preserve_identifier_escaping_and_query_bytes() {
    for dialect in [
        sf_sql::Dialect::Sqlite,
        sf_sql::Dialect::Postgres,
        sf_sql::Dialect::MySql,
        sf_sql::Dialect::BigQuery,
    ] {
        for name in [
            "",
            "plain",
            "α\"quoted\"`table",
            "x\"; DROP TABLE y; --",
            "a\0b",
        ] {
            let source = LogicalSource::Table(name.into());
            assert_source_boundaries(
                |control| source_probe_controlled(&source, dialect, control),
                dialect.probe_sql(&source),
            );
        }
        let query = LogicalSource::Query("SELECT 'α', '\0' /* unchanged */".into());
        assert_source_boundaries(
            |control| source_probe_controlled(&query, dialect, control),
            dialect.probe_sql(&query),
        );
    }
}

#[test]
fn live_catalog_preserves_descriptors_and_cumulative_source_boundaries() {
    use sf_core::datatype::XsdTypeCode;
    use sf_sql::backend::{NativeScalarKey, ResultColumn, SqliteDecode, TextKey};
    let columns = vec![
        ResultColumn {
            name: "α".into(),
            natural_datatype: Some(XsdTypeCode::String),
            native_scalar: None,
            text_key: Some(TextKey::SqliteCharacter(4)),
            sqlite_decode: Some(SqliteDecode {
                declared: Some(XsdTypeCode::String),
                padding: Some(4),
            }),
        },
        ResultColumn {
            name: "number".into(),
            natural_datatype: None,
            native_scalar: Some(NativeScalarKey::Integer),
            text_key: None,
            sqlite_decode: None,
        },
        ResultColumn {
            name: "unknown".into(),
            natural_datatype: None,
            native_scalar: None,
            text_key: None,
            sqlite_decode: None,
        },
    ];
    let sources = [
        LogicalSource::Table("same".into()),
        LogicalSource::Query("same".into()),
    ];
    let snapshot = |catalog: &crate::emit::ColumnCatalog| {
        (
            catalog.by_source.clone(),
            catalog.text_by_source.clone(),
            catalog.sqlite_by_source.clone(),
            catalog.scalars_by_source.clone(),
            catalog.datatypes_by_source.clone(),
        )
    };
    let mut raw = crate::emit::ColumnCatalog::default();
    for source in &sources {
        raw.insert_live_result(source, columns.clone()).unwrap();
    }
    assert_source_boundaries(
        |control| {
            let mut controlled = crate::emit::ColumnCatalog::default();
            for source in &sources {
                controlled.insert_live_result_controlled(source, columns.clone(), control)?;
            }
            Ok(snapshot(&controlled))
        },
        snapshot(&raw),
    );
    let mut duplicate = columns.clone();
    duplicate.push(columns[0].clone());
    assert!(crate::emit::ColumnCatalog::default()
        .insert_live_result_controlled(&sources[0], duplicate, &budget(u64::MAX))
        .is_err());
}

#[test]
fn paid_column_resolution_preserves_exact_folded_missing_and_rowid_rules() {
    let source = LogicalSource::Table("t".into());
    let mut catalog = crate::emit::ColumnCatalog::default();
    catalog.insert(
        &source,
        vec!["Exact".into(), "ambiguous".into(), "AMBIGUOUS".into()],
    );
    for dialect in [
        sf_sql::Dialect::Sqlite,
        sf_sql::Dialect::Postgres,
        sf_sql::Dialect::MySql,
    ] {
        for column in [
            "Exact",
            "exact",
            "ambiguous",
            "Ambiguous",
            "missing",
            "rowid",
        ] {
            let expected = catalog.validate_live_column(&source, column, dialect);
            let measured = catalog.validate_live_column_controlled(
                &source,
                column,
                dialect,
                SourceWork::new(Some(&budget(u64::MAX))),
            );
            assert_eq!(
                measured.as_ref().err().map(ToString::to_string),
                expected.as_ref().err().map(ToString::to_string)
            );
            if expected.is_ok() {
                assert_source_boundaries(
                    |control| {
                        catalog
                            .validate_live_column_controlled(
                                &source,
                                column,
                                dialect,
                                SourceWork::new(Some(control)),
                            )
                            .map_err(|error| match error {
                                crate::Error::QueryControl(cause) => {
                                    sf_sql::Error::QueryControl(cause)
                                }
                                other => sf_sql::Error::Emit(other.to_string()),
                            })
                    },
                    (),
                );
            }
        }
    }
}

#[test]
fn controlled_validation_keeps_nested_alias_shadowing_local() {
    use crate::iq::ColRef;
    let outer = LogicalSource::Table("outer".into());
    let inner = LogicalSource::Table("inner".into());
    let mut catalog = crate::emit::ColumnCatalog::default();
    catalog.insert(&outer, vec!["outer_only".into()]);
    catalog.insert(&inner, vec!["inner_only".into()]);
    let mut branch = Branch::empty();
    branch.core.push(scan("outer"));
    branch.where_conds.push(SqlCond::Exists {
        scans: vec![scan("inner")],
        conds: vec![SqlCond::IsNotNull(ColRef::new(
            0,
            Box::<str>::from("inner_only"),
        ))],
    });
    branch.where_conds.push(SqlCond::IsNotNull(ColRef::new(
        0,
        Box::<str>::from("outer_only"),
    )));
    let branches = vec![branch];
    assert_source_boundaries(
        |control| {
            crate::emit::validate_live_columns_controlled(
                &branches,
                sf_sql::Dialect::Sqlite,
                &catalog,
                SourceWork::new(Some(control)),
            )
            .map_err(|error| match error {
                crate::Error::QueryControl(cause) => sf_sql::Error::QueryControl(cause),
                other => sf_sql::Error::Emit(other.to_string()),
            })
        },
        (),
    );
}

#[test]
fn borrowed_definition_walk_preserves_columns_without_recursive_materialization() {
    use crate::iq::{AggKind, ColRef, TermDef};
    let mut definition = TermDef::Concat(vec![
        TermDef::Agg {
            col: ColRef::new(1, Box::<str>::from("a")),
            kind: AggKind::Count,
            operand: None,
            fixed_type: None,
        },
        TermDef::Agg {
            col: ColRef::new(2, Box::<str>::from("b")),
            kind: AggKind::Count,
            operand: None,
            fixed_type: None,
        },
    ]);
    for _ in 0..256 {
        definition = TermDef::Concat(vec![definition]);
    }
    let expected: Vec<_> = definition
        .columns()
        .into_iter()
        .map(|col| (col.alias, col.column.to_string()))
        .collect();
    assert_source_boundaries(
        |control| {
            let mut actual = Vec::new();
            validate_definition_columns(
                &definition,
                SourceWork::new(Some(control)),
                |alias, name| {
                    actual.push((alias, name.to_owned())); // test-only observation
                    Ok(())
                },
            )
            .map_err(|error| match error {
                crate::Error::QueryControl(cause) => sf_sql::Error::QueryControl(cause),
                other => sf_sql::Error::Emit(other.to_string()),
            })?;
            Ok(actual)
        },
        expected,
    );
}

#[test]
fn reference_output_validation_parses_canonical_index_without_width_sized_work() {
    for width in [0, 1, 10, 32] {
        for name in [
            "", "c", "c0", "c00", "c01", "c9", "c31", "c32", "c+1", "c-1", "c１", "C1", "c1 ",
        ] {
            let expected = (0..width).any(|index| name == format!("c{index}"));
            assert_eq!(
                crate::emit::ref_atom::validate_output(width, name).is_ok(),
                expected
            );
        }
    }
    for name in ["c0".to_owned(), format!("c{}", usize::MAX - 1)] {
        assert_source_boundaries(
            |control| {
                crate::emit::ref_atom::validate_output_controlled(
                    usize::MAX,
                    &name,
                    SourceWork::new(Some(control)),
                )
                .map_err(|error| match error {
                    crate::Error::QueryControl(cause) => sf_sql::Error::QueryControl(cause),
                    other => sf_sql::Error::Emit(other.to_string()),
                })
            },
            (),
        );
    }
    assert!(
        crate::emit::ref_atom::validate_output(usize::MAX, &format!("c{}", usize::MAX)).is_err()
    );
    assert!(
        crate::emit::ref_atom::validate_output(usize::MAX, &format!("c{}0", usize::MAX)).is_err()
    );
}

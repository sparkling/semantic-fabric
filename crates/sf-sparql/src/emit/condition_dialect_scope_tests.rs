use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
use sf_core::{datatype::XsdTypeCode, ir::TermSpec, Literal};
use sf_sql::backend::ResultColumn;

const DIALECTS: [Dialect; 2] = [Dialect::Postgres, Dialect::MySql];

struct Fixture {
    dialect: Dialect,
    catalog: ColumnCatalog,
    outer: ActualColumns,
    cond: SqlCond,
}

fn result_column(
    name: &str,
    natural: Option<XsdTypeCode>,
    scalar: Option<NativeScalarKey>,
    text: Option<TextKey>,
) -> ResultColumn {
    ResultColumn {
        name: name.into(),
        natural_datatype: natural,
        text_key: text,
        native_scalar: scalar,
        sqlite_decode: None,
    }
}

fn numeric_leaf(dialect: Dialect, alias: usize) -> SqlCond {
    if dialect == Dialect::MySql {
        let spec = TermSpec::typed_literal(XsdTypeCode::Integer.iri().into_owned());
        SqlCond::LiteralCmp(Box::new(LiteralComparison {
            left: LiteralOperand::Column {
                column: ColRef::new(alias, "num0"),
                spec,
            },
            right: LiteralOperand::Constant(Literal::new_typed_literal(
                "1.00",
                XsdTypeCode::Decimal.iri(),
            )),
            value_op: Some(CmpOp::Lt),
        }))
    } else {
        SqlCond::DecodedIsNotNull(ColRef::new(alias, "num0"))
    }
}

fn tail(dialect: Dialect) -> SqlCond {
    let any = vec![numeric_leaf(dialect, 0), SqlCond::ExpressionError];
    SqlCond::Not(Box::new(SqlCond::Or(any)))
}

fn fixture(dialect: Dialect) -> Fixture {
    let mysql = dialect == Dialect::MySql;
    let items = LogicalSource::Table("items".into());
    let shadow = LogicalSource::Table("shadow".into());
    let (code, scalar) = if mysql {
        (XsdTypeCode::Integer, NativeScalarKey::Integer)
    } else {
        (XsdTypeCode::Decimal, NativeScalarKey::PostgresNumeric)
    };
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &items,
            vec![
                result_column("num0", Some(code), Some(scalar), None),
                result_column("key0", None, None, Some(TextKey::Verbatim)),
            ],
        )
        .unwrap();
    catalog
        .insert_live_result(
            &shadow,
            vec![
                result_column("NUM0", None, Some(NativeScalarKey::Integer), None),
                result_column("KEY0", None, None, Some(TextKey::Verbatim)),
            ],
        )
        .unwrap();
    let mut actual = source_actuals(&items, &catalog);
    let text = &mut actual.text_columns;
    text.insert("key0".to_owned(), TextKey::Verbatim);
    let outer = HashMap::from([(0, actual)]);

    let shadow_scope = SqlCond::Exists {
        scans: vec![scan(0, "shadow")],
        conds: vec![
            native(0, "key0", "in"),
            SqlCond::DecodedIsNotNull(ColRef::new(0, "num0")),
        ],
    };
    let mut conds = vec![shadow_scope, native(0, "key0", "out")];
    if !mysql {
        conds.push(SqlCond::NotExists {
            scans: vec![scan(2, "items")],
            conds: vec![
                native(2, "key0", "inner"),
                SqlCond::DecodedIsNotNull(ColRef::new(2, "num0")),
            ],
        });
    }
    conds.push(tail(dialect));
    Fixture {
        dialect,
        catalog,
        outer,
        cond: SqlCond::And(conds),
    }
}

fn run_with(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    work: SourceWork<'_>,
) -> Result<(String, Vec<String>)> {
    let mut params = vec![];
    let mut index = 0;
    let sql = condition(
        cond,
        dialect,
        catalog,
        actuals,
        &mut params,
        &mut index,
        work,
    )?;
    assert_eq!(index, params.len());
    Ok((sql, params))
}

fn run(f: &Fixture, work: SourceWork<'_>) -> Result<(String, Vec<String>)> {
    run_with(&f.cond, f.dialect, &f.catalog, &f.outer, work)
}

fn admit(f: &Fixture, conds: &[&SqlCond]) -> (Option<String>, Vec<String>) {
    let mut params = vec![];
    let mut index = 0;
    let sql = authorized(
        conds,
        f.dialect,
        &f.catalog,
        &f.outer,
        &mut params,
        &mut index,
        SourceWork::new(None),
    );
    (sql.unwrap(), params)
}

#[test]
fn postgres_conjunction_reaches_case_in_outer_and_nested_scopes() {
    let f = fixture(Dialect::Postgres);
    let expected = r#"(CASE WHEN (t0."key0" = $1) THEN (EXISTS (SELECT 1 FROM "shadow" t0 WHERE t0."KEY0" = $2 AND t0."NUM0" IS NOT NULL) AND NOT EXISTS (SELECT 1 FROM "items" t2 WHERE CASE WHEN (t2."key0" = $3) THEN (CAST(CAST(CAST(t2."num0" AS TEXT) AS JSON) AS TEXT) IS NOT NULL) ELSE FALSE END) AND (NOT (CAST(CAST(CAST(t0."num0" AS TEXT) AS JSON) AS TEXT) IS NOT NULL OR (NULL = 1)))) ELSE FALSE END)"#;
    let control = budget(u64::MAX);
    let controlled = run(&f, SourceWork::new(Some(&control))).unwrap();
    assert_eq!(controlled.0, expected);
    assert_eq!(controlled.1, ["out", "in", "inner"]);
    assert_eq!(controlled, run(&f, SourceWork::new(None)).unwrap());
}

#[test]
fn mysql_conjunction_reaches_case_and_restores_outer_scope() {
    let f = fixture(Dialect::MySql);
    let prefix = "(CASE WHEN (t0.`key0` = ?) THEN (EXISTS (SELECT 1 FROM `shadow` t0 WHERE t0.`KEY0` = ? AND t0.`NUM0` IS NOT NULL) AND (NOT (";
    let suffix = " OR (NULL = 1)))) ELSE FALSE END)";
    let control = budget(u64::MAX);
    let (sql, params) = run(&f, SourceWork::new(Some(&control))).unwrap();
    assert_eq!(params, ["out", "in", "1.00"]);
    assert!(sql.starts_with(prefix), "{sql}");
    assert!(sql.ends_with(suffix), "{sql}");
    assert!(sql.len() > prefix.len() + suffix.len(), "{sql}");
    let leaf = &sql[prefix.len()..sql.len() - suffix.len()];
    assert!(leaf.contains("JSON_EXTRACT"), "{leaf}");
    assert!(!leaf.contains("NUM0"), "{leaf}");
}

#[test]
fn populated_conjunctions_admit_exactly_and_refuse_one_unit_less() {
    for dialect in DIALECTS {
        let f = fixture(dialect);
        let meter = budget(u64::MAX);
        let expected = run(&f, SourceWork::new(Some(&meter))).unwrap();
        let units = meter.consumed(QueryCharge::SourceWork);
        assert!(units > 0);
        let exact = budget(units);
        let admitted = run(&f, SourceWork::new(Some(&exact))).unwrap();
        assert_eq!(admitted, expected);
        assert_eq!(exact.consumed(QueryCharge::SourceWork), units);
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), 0);
        let short = budget(units - 1);
        let refused = run(&f, SourceWork::new(Some(&short)));
        assert!(matches!(
            refused,
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
    }
}

#[test]
fn populated_conjunctions_keep_first_terminal_cause_at_every_charge() {
    for dialect in DIALECTS {
        let f = fixture(dialect);
        let measured = Trace::new(usize::MAX, QueryControlError::Cancelled);
        run(&f, SourceWork::new(Some(&measured))).unwrap();
        let charges = measured.seen().len();
        assert!(charges > 20);
        for reason in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=charges {
                let control = Trace::new(at, reason);
                let result = run(&f, SourceWork::new(Some(&control)));
                assert!(
                    matches!(result, Err(Error::QueryControl(found)) if found == reason),
                    "{reason:?} at charge {at}"
                );
                assert_eq!(control.checkpoint(), Err(reason));
            }
        }
    }
}

#[test]
fn populated_metadata_copies_are_charged_and_outer_copy_is_restored() {
    for dialect in DIALECTS {
        let f = fixture(dialect);
        let actual = &f.outer[&0usize];
        assert!(!actual.scalar_columns.is_empty());
        assert!(!actual.text_columns.is_empty());
        let trace = Trace::new(usize::MAX, QueryControlError::Cancelled);
        run(&f, SourceWork::new(Some(&trace))).unwrap();
        let observed = trace.seen();
        let (header, blocks) = copy_recipe(&f.outer);
        assert!(find_copy(&observed, 0, header + 1, &blocks).is_none());
        let end = find_copy(&observed, 0, header, &blocks).expect("outer copy charged");
        if dialect == Dialect::Postgres {
            let again = find_copy(&observed, end, header, &blocks);
            assert!(again.is_some(), "outer copy after restore charged");
        }
    }
}

#[test]
fn authorized_gate_needs_policy_and_a_validating_leaf() {
    for dialect in DIALECTS {
        let f = fixture(dialect);
        let mysql = dialect == Dialect::MySql;
        let policy = native(0, "key0", "out");
        let validating = tail(dialect);
        let plain = SqlCond::IsNull(ColRef::new(0, "num0"));
        let quote = if mysql { '`' } else { '"' };
        let placeholder = if mysql { "?" } else { "$1" };
        let head = format!("CASE WHEN (t0.{quote}key0{quote} = {placeholder}) THEN (");
        let (sql, params) = admit(&f, &[&policy, &validating]);
        let sql = sql.expect("authorized conjunction");
        assert!(sql.starts_with(&head), "{sql}");
        assert!(sql.ends_with(") ELSE FALSE END"), "{sql}");
        assert_eq!(params.first().map(String::as_str), Some("out"));
        assert_eq!(admit(&f, &[&policy]).0, None);
        assert_eq!(admit(&f, &[&validating]).0, None);
        assert_eq!(admit(&f, &[&policy, &plain]).0, None);
    }
}

fn leaked_actuals(f: &Fixture) -> ActualColumns {
    let shadow = metadata::scan_actuals_controlled(
        &scan(0, "shadow"),
        f.dialect,
        &f.catalog,
        SourceWork::new(None),
    )
    .unwrap();
    HashMap::from([(0, shadow)])
}

#[test]
fn postgres_missing_restore_would_change_the_authorized_tail() {
    let f = fixture(Dialect::Postgres);
    let leaked = leaked_actuals(&f);
    let cond = SqlCond::And(vec![native(0, "key0", "out"), tail(f.dialect)]);
    let restored_sql = r#"(CASE WHEN (t0."key0" = $1) THEN ((NOT (CAST(CAST(CAST(t0."num0" AS TEXT) AS JSON) AS TEXT) IS NOT NULL OR (NULL = 1)))) ELSE FALSE END)"#;
    let leaked_sql = r#"(t0."KEY0" = $1 AND (NOT (t0."NUM0" IS NOT NULL OR (NULL = 1))))"#;
    let work = SourceWork::new(None);
    let restored = run_with(&cond, f.dialect, &f.catalog, &f.outer, work).unwrap();
    let shadowed = run_with(&cond, f.dialect, &f.catalog, &leaked, work).unwrap();
    let out = vec!["out".to_owned()];
    assert_eq!(restored, (restored_sql.to_owned(), out.clone()));
    assert_eq!(shadowed, (leaked_sql.to_owned(), out));
}

#[test]
fn mysql_missing_restore_would_lose_the_authorized_case() {
    let f = fixture(Dialect::MySql);
    let leaked = leaked_actuals(&f);
    let cond = SqlCond::And(vec![native(0, "key0", "out"), tail(f.dialect)]);
    let head = "(CASE WHEN (t0.`key0` = ?) THEN ((NOT (";
    let work = SourceWork::new(None);
    let (sql, params) = run_with(&cond, f.dialect, &f.catalog, &f.outer, work).unwrap();
    assert!(sql.starts_with(head), "{sql}");
    assert!(sql.contains("JSON_EXTRACT"), "{sql}");
    assert_eq!(params, ["out", "1.00"]);
    let shadowed = run_with(&cond, f.dialect, &f.catalog, &leaked, work);
    if let Ok((sql, _)) = shadowed {
        assert!(!sql.starts_with(head), "{sql}");
    }
}

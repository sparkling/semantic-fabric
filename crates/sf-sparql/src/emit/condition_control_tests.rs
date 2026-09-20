use super::*;
use crate::iq::{CmpOp, Scan};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
}
fn native(alias: usize, name: &str, value: &str) -> SqlCond {
    SqlCond::NativeCmp(ColRef::new(alias, name), CmpOp::Eq, value.into())
}
fn render(cond: &SqlCond, dialect: Dialect, work: SourceWork<'_>) -> Result<(String, Vec<String>)> {
    let mut params = vec![];
    let mut index = 0;
    let sql = render_cond_controlled(
        cond,
        dialect,
        &ColumnCatalog::default(),
        &HashMap::new(),
        &mut params,
        &mut index,
        work,
    )?;
    assert_eq!(index, params.len());
    Ok((sql, params))
}

#[test]
fn shallow_boolean_and_exists_sql_and_parameter_order_are_unchanged() {
    let cond = SqlCond::And(vec![
        SqlCond::Not(Box::new(native(0, "a", "first"))),
        SqlCond::Or(vec![SqlCond::ExpressionError, native(0, "b", "second")]),
        SqlCond::Exists {
            scans: vec![Scan {
                alias: 2,
                source: LogicalSource::Table("items".into()).into(),
            }],
            conds: vec![native(2, "c", "third")],
        },
        SqlCond::NotExists {
            scans: vec![],
            conds: vec![],
        },
        SqlCond::And(vec![]),
        SqlCond::Or(vec![]),
    ]);
    for (dialect, q, placeholders) in [
        (Dialect::Sqlite, '"', ["?", "?", "?"]),
        (Dialect::Postgres, '"', ["$1", "$2", "$3"]),
        (Dialect::MySql, '`', ["?", "?", "?"]),
    ] {
        let (sql, params) = render(&cond, dialect, SourceWork::new(None)).unwrap();
        let [a, b, c] = placeholders;
        assert_eq!(sql, format!("((NOT t0.{q}a{q} = {a}) AND ((NULL = 1) OR t0.{q}b{q} = {b}) AND EXISTS (SELECT 1 FROM {q}items{q} t2 WHERE t2.{q}c{q} = {c}) AND NOT EXISTS (SELECT 1 WHERE 1 = 1) AND (1 = 1) AND 1 = 0)"));
        assert_eq!(params, ["first", "second", "third"]);
    }
}

fn dismantle(condition: SqlCond) {
    let mut pending = vec![condition];
    while let Some(condition) = pending.pop() {
        match condition {
            SqlCond::Not(inner) => pending.push(*inner),
            SqlCond::And(inner)
            | SqlCond::Or(inner)
            | SqlCond::Exists { conds: inner, .. }
            | SqlCond::NotExists { conds: inner, .. }
            | SqlCond::PathExists { conds: inner, .. } => pending.extend(inner),
            _ => {}
        }
    }
}

#[test]
fn deep_renderer_and_public_branch_refusal_do_not_recurse() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            for kind in 0..6 {
                let mut cond = SqlCond::ExpressionError;
                for _ in 0..4096 {
                    cond = match kind {
                        0 => SqlCond::Not(Box::new(cond)),
                        1 => SqlCond::And(vec![cond]),
                        2 => SqlCond::Or(vec![cond]),
                        3 => SqlCond::Exists {
                            scans: vec![],
                            conds: vec![cond],
                        },
                        4 => SqlCond::NotExists {
                            scans: vec![],
                            conds: vec![cond],
                        },
                        _ => SqlCond::PathExists {
                            pc: path(),
                            conds: vec![cond],
                            negated: false,
                        },
                    };
                }
                let mut branch = Branch::empty();
                branch.where_conds.push(cond);
                let renderer = budget(u64::MAX);
                let rendered = render(
                    &branch.where_conds[0],
                    Dialect::Sqlite,
                    SourceWork::new(Some(&renderer)),
                )
                .unwrap();
                assert!(rendered.0.contains("(NULL = 1)"));
                // Public admission must propagate its control into the renderer.
                // Full SQL parser qualification is a separate G1 boundary.
                let control = budget(100_000);
                let emitted = emit_branch_controlled(
                    &branch,
                    Dialect::Sqlite,
                    &ColumnCatalog::default(),
                    BranchModifiers::stored(&branch),
                    SourceWork::new(Some(&control)),
                );
                for cond in branch.where_conds.drain(..) {
                    dismantle(cond);
                }
                assert!(matches!(
                    emitted,
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
                assert!(renderer.consumed(QueryCharge::SourceWork) > 4096);
                assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

struct Stop {
    budget: QueryBudget,
    calls: AtomicUsize,
    at: usize,
    reason: QueryControlError,
}
impl QueryControl for Stop {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
    fn consume(&self, kind: QueryCharge, units: u64) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(kind, units)?;
        if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
            self.budget.terminate(self.reason);
        }
        self.budget.checkpoint()
    }
}

#[test]
fn exact_boundary_and_every_charge_preserve_terminal_cause() {
    let cond = SqlCond::And(vec![
        native(0, "tenant", "allowed"),
        SqlCond::Not(Box::new(SqlCond::Or(vec![SqlCond::ExpressionError]))),
        SqlCond::Exists {
            scans: vec![],
            conds: vec![native(1, "other", "value")],
        },
    ]);
    let measured = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at: usize::MAX,
        reason: QueryControlError::Cancelled,
    };
    let expected = render(&cond, Dialect::Postgres, SourceWork::new(Some(&measured))).unwrap();
    let units = measured.budget.consumed(QueryCharge::SourceWork);
    assert_eq!(
        expected,
        render(
            &cond,
            Dialect::Postgres,
            SourceWork::new(Some(&budget(units)))
        )
        .unwrap()
    );
    assert!(matches!(
        render(
            &cond,
            Dialect::Postgres,
            SourceWork::new(Some(&budget(units - 1)))
        ),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.calls.load(Ordering::Relaxed) {
            let control = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                at,
                reason,
            };
            assert!(
                matches!(render(&cond, Dialect::Postgres, SourceWork::new(Some(&control))), Err(Error::QueryControl(found)) if found == reason)
            );
            assert_eq!(control.checkpoint(), Err(reason));
        }
    }
}

fn natural_fixture(dialect: Dialect) -> (ColumnCatalog, ActualColumns, SqlCond) {
    use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
    use sf_core::{datatype::XsdTypeCode, ir::TermSpec, Literal};
    let mysql = dialect == Dialect::MySql;
    let source = LogicalSource::Table("items".into());
    let mut catalog = ColumnCatalog::default();
    catalog
        .insert_live_result(
            &source,
            vec![sf_sql::backend::ResultColumn {
                name: "src".into(),
                natural_datatype: Some(if mysql {
                    XsdTypeCode::Integer
                } else {
                    XsdTypeCode::Decimal
                }),
                text_key: None,
                native_scalar: Some(if mysql {
                    NativeScalarKey::Integer
                } else {
                    NativeScalarKey::PostgresNumeric
                }),
                sqlite_decode: None,
            }],
        )
        .unwrap();
    let actuals = HashMap::from([(0, source_actuals(&source, &catalog))]);
    let leaf = if mysql {
        SqlCond::LiteralCmp(Box::new(LiteralComparison {
            left: LiteralOperand::Column {
                column: ColRef::new(0, "src"),
                spec: TermSpec::typed_literal(XsdTypeCode::Integer.iri().into_owned()),
            },
            right: LiteralOperand::Constant(Literal::new_typed_literal(
                "1.00",
                XsdTypeCode::Decimal.iri(),
            )),
            value_op: Some(CmpOp::Lt),
        }))
    } else {
        SqlCond::DecodedIsNotNull(ColRef::new(0, "src"))
    };
    (catalog, actuals, leaf)
}

#[test]
fn deep_policy_validation_preserves_parameter_order_on_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            for dialect in [Dialect::MySql, Dialect::Postgres] {
                let (catalog, actuals, mut cond) = natural_fixture(dialect);
                for index in 0..4096 {
                    cond = match index % 3 {
                        0 => SqlCond::Not(Box::new(cond)),
                        1 => SqlCond::And(vec![cond]),
                        _ => SqlCond::Or(vec![cond]),
                    };
                }
                let policy = native(0, "tenant", "allowed");
                let invoke = |control: &dyn QueryControl| {
                    let mut params = vec![];
                    let sql = natural_literal::authorized_conjunction_controlled(
                        &[&cond, &policy],
                        dialect,
                        &catalog,
                        &actuals,
                        &mut params,
                        &mut 0,
                        SourceWork::new(Some(control)),
                    )?;
                    Ok::<_, Error>((sql.unwrap(), params))
                };
                let meter = budget(u64::MAX);
                let expected = invoke(&meter).unwrap();
                let units = meter.consumed(QueryCharge::SourceWork);
                let exact = invoke(&budget(units)).unwrap();
                let short = invoke(&budget(units - 1));
                dismantle(cond);
                assert_eq!(expected, exact);
                assert!(matches!(
                    short,
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
                if dialect == Dialect::MySql {
                    assert!(expected.0.starts_with("CASE WHEN (t0.`tenant` = ?) THEN"));
                    assert!(expected.0.contains("JSON_EXTRACT"));
                    assert_eq!(expected.1, ["allowed", "1.00"]);
                } else {
                    assert!(expected
                        .0
                        .starts_with("CASE WHEN (t0.\"tenant\" = $1) THEN"));
                    assert!(expected
                        .0
                        .contains("CAST(CAST(CAST(t0.\"src\" AS TEXT) AS JSON) AS TEXT)"));
                    assert_eq!(expected.1, ["allowed"]);
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

fn path() -> PathClosure {
    PathClosure {
        alias: 0,
        kind: PathKind::One,
        hop: HopExpr::Pred(crate::iq::HopRelation {
            source: LogicalSource::Table("edges".into()),
            subj_col: "s".into(),
            obj_col: "o".into(),
        }),
    }
}

#[test]
fn exists_and_path_exists_restore_outer_decoder_scope() {
    for path_scope in [false, true] {
        for dialect in [Dialect::MySql, Dialect::Postgres] {
            let (mut catalog, actuals, leaf) = natural_fixture(dialect);
            catalog.insert(&LogicalSource::Table("child".into()), vec!["src".into()]);
            let nested = if path_scope {
                SqlCond::PathExists {
                    pc: path(),
                    conds: vec![],
                    negated: true,
                }
            } else {
                SqlCond::Exists {
                    scans: vec![Scan {
                        alias: 0,
                        source: LogicalSource::Table("child".into()).into(),
                    }],
                    conds: vec![],
                }
            };
            let policy = native(0, "tenant", "allowed");
            let mut params = vec![];
            let sql = conjunction(
                &[&nested, &leaf, &policy],
                dialect,
                &catalog,
                &actuals,
                &mut params,
                &mut 0,
                SourceWork::new(Some(&budget(u64::MAX))),
            )
            .unwrap();
            assert!(sql.starts_with("CASE WHEN ("));
            assert!(sql.contains("EXISTS ("));
            if dialect == Dialect::MySql {
                assert!(sql.contains("JSON_EXTRACT"));
                assert_eq!(params, ["allowed", "1.00"]);
            } else {
                assert!(sql
                    .contains("CAST(CAST(CAST(t0.\"src\" AS TEXT) AS JSON) AS TEXT) IS NOT NULL"));
                assert_eq!(params, ["allowed"]);
            }
        }
    }
}

#[test]
fn path_exists_preserves_prelude_and_nested_parameter_order() {
    let pc = path();
    let prelude = path_sql::prelude(
        &pc,
        0,
        Dialect::Postgres,
        &ColumnCatalog::default(),
        SourceWork::new(None),
    )
    .unwrap();
    let condition = SqlCond::PathExists {
        pc,
        conds: vec![native(0, "sf_o", "object")],
        negated: false,
    };
    let (sql, params) = render(
        &condition,
        Dialect::Postgres,
        SourceWork::new(Some(&budget(u64::MAX))),
    )
    .unwrap();
    assert_eq!(
        sql,
        format!("EXISTS ({prelude} SELECT 1 FROM t0 WHERE t0.\"sf_o\" = $1)")
    );
    assert_eq!(params, ["object"]);
}

#[test]
fn public_branch_fixed_exact_work_proves_control_threading() {
    let mut branch = Branch::empty();
    branch
        .where_conds
        .push(SqlCond::Not(Box::new(SqlCond::ExpressionError)));
    let invoke = |control: &dyn QueryControl| {
        emit_branch_controlled(
            &branch,
            Dialect::Postgres,
            &ColumnCatalog::default(),
            BranchModifiers::stored(&branch),
            SourceWork::new(Some(control)),
        )
    };
    let meter = budget(u64::MAX);
    let expected = invoke(&meter).unwrap();
    let units = meter.consumed(QueryCharge::SourceWork);
    assert_eq!(units, 1876, "fixed production condition work");
    assert_eq!(expected.sql, invoke(&budget(units)).unwrap().sql);
    assert!(matches!(
        invoke(&budget(units - 1)),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
}

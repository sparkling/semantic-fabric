use super::*;
use crate::compiler_control::CompileContext;
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

#[test]
fn schema_comparison_and_relocation_observe_every_cancel_and_deadline() {
    use sf_core::query_control::QueryControl;
    struct Stop {
        budget: QueryBudget,
        at: u64,
        cause: QueryControlError,
    }
    impl QueryControl for Stop {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            kind: QueryCharge,
            units: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.budget.consume(kind, units)?;
            if self.budget.consumed(QueryCharge::CompilerWork) >= self.at {
                return Err(self.budget.terminate(self.cause));
            }
            Ok(())
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.budget.terminate(cause)
        }
    }
    let tables: Vec<_> = ["z", "middle", "a"]
        .into_iter()
        .map(TableSchema::new)
        .collect();
    let budget = || QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let measured = budget();
    build(
        &tables,
        BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(
            &measured,
        ))),
    )
    .unwrap();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.consumed(QueryCharge::CompilerWork) {
            let stopped = Stop {
                budget: budget(),
                at,
                cause,
            };
            assert!(
                matches!(build(&tables, BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(&stopped)))), Err(crate::Error::QueryControl(actual)) if actual == cause)
            );
            assert_eq!(stopped.checkpoint(), Err(cause));
        }
    }
}

#[test]
fn d1_and_d2_share_controlled_schema_without_losing_source_type_authority() {
    for right_type in ["text", "float8"] {
        let mut left = TableSchema::new("a");
        left.columns = vec![sf_sql::Column::new("value", "text", true)];
        let mut right = TableSchema::new("longer_second_table");
        right.columns = vec![sf_sql::Column::new("value", right_type, true)];
        let tables = [right, left];
        let branches: Vec<_> = ["a", "longer_second_table"]
            .into_iter()
            .enumerate()
            .map(|(alias, table)| {
                let mut branch = Branch::single(Scan {
                    alias,
                    source: LogicalSource::Table(table.into()).into(),
                });
                branch.bindings.insert(
                    "value".into(),
                    TermDef::Derived {
                        alias,
                        term_map: TermMap::Column(
                            "value".into(),
                            sf_core::ir::TermSpec::plain_literal(),
                        ),
                    },
                );
                branch
            })
            .collect();
        let run = |units| {
            let budget = QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
            let work = BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(
                &budget,
            )));
            let result = (|| -> Result<PoolTypeSafety> {
                let schema = build(&tables, work)?;
                let mut branches = branches.clone();
                let authorities = branches
                    .iter()
                    .map(|b| PoolSourceAuthority::capture_with_work(b, work))
                    .collect::<Result<Vec<_>>>()?;
                force_distinct_with_schema(
                    &mut branches,
                    &schema,
                    sf_sql::Dialect::Postgres,
                    work,
                )?;
                super::super::group_pool_type_safety_with_schema(
                    &branches.iter().collect::<Vec<_>>(),
                    Some(&authorities.iter().collect::<Vec<_>>()),
                    &schema,
                    sf_sql::Dialect::Postgres,
                    crate::compiler_schema::ColumnTypeUse::CallerAuthorizedFrozen,
                    work,
                )
            })();
            (result, budget.consumed(QueryCharge::CompilerWork))
        };
        let expected = if right_type == "text" {
            PoolTypeSafety::ProvenSafe
        } else {
            PoolTypeSafety::Unproven
        };
        let (actual, used) = run(u64::MAX);
        assert_eq!(actual.unwrap(), expected);
        assert_eq!(run(used).0.unwrap(), expected);
        assert!(matches!(
            run(used - 1).0,
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
    }
}

#[test]
fn sorted_unique_schema_matches_raw_and_exact_budget() {
    let schema: Vec<_> = ["z", "m", "a", "b"]
        .into_iter()
        .map(TableSchema::new)
        .collect();
    let expected = super::super::build_schema_map(&schema);
    let run = |units| {
        let budget = QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
        let result = build(
            &schema,
            BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(
                &budget,
            ))),
        );
        (result, budget.consumed(QueryCharge::CompilerWork))
    };
    let (actual, used) = run(u64::MAX);
    assert_eq!(
        actual.unwrap().iter().map(|v| v.0).collect::<Vec<_>>(),
        expected.iter().map(|v| v.0).collect::<Vec<_>>()
    );
    assert!(run(used).0.is_ok());
    assert!(matches!(
        run(used - 1).0,
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
}

#[test]
fn ambiguous_schema_never_mutates_or_grants_constraint_authority() {
    for conflicting in [false, true] {
        let first = TableSchema::new("duplicate");
        let mut second = first.clone();
        if conflicting {
            second.primary_key = vec!["id".into()];
        }
        let schema = [first, second];
        let branch = Branch::single(Scan {
            alias: 0,
            source: LogicalSource::Table("duplicate".into()).into(),
        });
        let budget = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
        for mode in [
            crate::CompilerWorkMode::Uncontrolled,
            crate::CompilerWorkMode::Metered(CompileContext::new(&budget)),
        ] {
            let mut branches = vec![branch.clone()];
            assert!(
                matches!(force_distinct_with_work(&mut branches, &schema, sf_sql::Dialect::Sqlite, BuildWork::new(mode)), Err(crate::Error::Unsupported(message)) if message == "ambiguous compiler schema: duplicate table name")
            );
            assert_eq!(format!("{:?}", branches[0]), format!("{branch:?}"));
        }
    }
}

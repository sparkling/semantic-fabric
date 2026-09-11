use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sql::{Column, Dialect, TableSchema};

use super::{
    force_distinct_for_dup_safety, group_pool_type_safety,
    group_pool_type_safety_with_source_authority, PoolSourceAuthority, PoolTypeSafety,
};
use crate::compiler_schema::ColumnTypeUse;
use crate::iq::{Branch, OptJoin, Scan, SqlCond, TermDef};

fn bind_column(branch: &mut Branch, variable: &str, alias: usize, column: &str) {
    branch.bindings.insert(
        variable.to_owned(),
        TermDef::Derived {
            term_map: TermMap::Column(column.into(), TermSpec::plain_literal()),
            alias,
        },
    );
}

#[test]
fn controlled_capture_preserves_first_source_order_and_exact_budget() {
    use crate::build::control::BuildWork;
    use crate::compiler_control::CompileContext;
    use crate::CompilerWorkMode;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    let mut branch = core_branch(7, LogicalSource::Table("first".into()));
    branch
        .core
        .extend(core_branch(7, LogicalSource::Query("ignored".into())).core);
    branch.opts = optional_branch(8, LogicalSource::Table("optional".into())).opts;
    branch.where_conds = vec![SqlCond::And(vec![SqlCond::Exists {
        scans: vec![
            Scan {
                alias: 7,
                source: LogicalSource::Table("duplicate".into()).into(),
            },
            Scan {
                alias: 9,
                source: LogicalSource::Query("nested".into()).into(),
            },
        ],
        conds: vec![SqlCond::Not(Box::new(SqlCond::Exists {
            scans: vec![Scan {
                alias: 10,
                source: LogicalSource::Table("deep".into()).into(),
            }],
            conds: vec![],
        }))],
    }])];
    let snapshot = |authority: PoolSourceAuthority| {
        authority
            .sources
            .into_iter()
            .map(|source| (source.alias, format!("{:?}", source.source), source.is_core))
            .collect::<Vec<_>>()
    };
    let expected = snapshot(PoolSourceAuthority::capture(&branch));
    assert_eq!(
        expected
            .iter()
            .map(|(alias, _, core)| (*alias, *core))
            .collect::<Vec<_>>(),
        vec![(7, true), (8, false), (9, false), (10, false)]
    );
    let budget = |limit| QueryBudget::new(QueryLimits::new(limit, u64::MAX, u64::MAX, u64::MAX));
    let paid = budget(u64::MAX);
    let capture = |control: &QueryBudget| {
        PoolSourceAuthority::capture_with_work(
            &branch,
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
        )
    };
    assert_eq!(snapshot(capture(&paid).unwrap()), expected);
    let total = paid.consumed(QueryCharge::CompilerWork);
    assert!(total > 0);
    assert_eq!(snapshot(capture(&budget(total)).unwrap()), expected);
    assert!(matches!(
        capture(&budget(total - 1)),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
}

fn core_branch(alias: usize, source: LogicalSource) -> Branch {
    let mut branch = Branch::single(Scan {
        alias,
        source: source.into(),
    });
    bind_column(&mut branch, "value", alias, "value");
    branch
}

fn optional_branch(alias: usize, source: LogicalSource) -> Branch {
    let mut branch = Branch::empty();
    branch.opts.push(OptJoin {
        scan: Scan {
            alias,
            source: source.into(),
        },
        on: Vec::new(),
        extra: Vec::new(),
    });
    bind_column(&mut branch, "value", alias, "value");
    branch
}

fn condition_branch(alias: usize, source: LogicalSource) -> Branch {
    let mut branch = Branch::empty();
    branch.where_conds.push(SqlCond::Exists {
        scans: vec![Scan {
            alias,
            source: source.into(),
        }],
        conds: Vec::new(),
    });
    bind_column(&mut branch, "value", alias, "value");
    branch
}

fn captured_safety(
    branches: &[Branch],
    authorities: &[PoolSourceAuthority],
    schema: &[TableSchema],
    type_use: ColumnTypeUse,
) -> PoolTypeSafety {
    let members = branches.iter().collect::<Vec<_>>();
    let sources = authorities.iter().collect::<Vec<_>>();
    group_pool_type_safety_with_source_authority(
        &members,
        &sources,
        schema,
        Dialect::Postgres,
        type_use,
    )
}

#[test]
fn captured_sources_preserve_table_and_query_identity() {
    for (left, right, expected) in [
        (
            LogicalSource::Table("shared".to_owned()),
            LogicalSource::Table("shared".to_owned()),
            PoolTypeSafety::ProvenSafe,
        ),
        (
            LogicalSource::Table("left".to_owned()),
            LogicalSource::Table("right".to_owned()),
            PoolTypeSafety::Unproven,
        ),
        (
            LogicalSource::Query("SELECT value FROM shared".to_owned()),
            LogicalSource::Query("SELECT value FROM shared".to_owned()),
            PoolTypeSafety::ProvenSafe,
        ),
        (
            LogicalSource::Query("SELECT value FROM left".to_owned()),
            LogicalSource::Query("SELECT value FROM right".to_owned()),
            PoolTypeSafety::Unproven,
        ),
        (
            LogicalSource::Table("shared".to_owned()),
            LogicalSource::Query("shared".to_owned()),
            PoolTypeSafety::Unproven,
        ),
    ] {
        let branches = vec![core_branch(0, left), core_branch(1, right)];
        let authorities = branches
            .iter()
            .map(PoolSourceAuthority::capture)
            .collect::<Vec<_>>();
        assert_eq!(
            captured_safety(&branches, &authorities, &[], ColumnTypeUse::Unverified),
            expected
        );
    }
}

#[test]
fn captured_sources_keep_alias_first_match_and_non_core_lookup_semantics() {
    let mut prioritized = core_branch(0, LogicalSource::Table("first".to_owned()));
    prioritized.opts.push(OptJoin {
        scan: Scan {
            alias: 0,
            source: (LogicalSource::Table("later-optional".to_owned())).into(),
        },
        on: Vec::new(),
        extra: Vec::new(),
    });
    prioritized.where_conds.push(SqlCond::NotExists {
        scans: vec![Scan {
            alias: 0,
            source: (LogicalSource::Query("later-condition".to_owned())).into(),
        }],
        conds: Vec::new(),
    });
    let branches = vec![
        prioritized,
        core_branch(1, LogicalSource::Table("first".to_owned())),
    ];
    let authorities = branches
        .iter()
        .map(PoolSourceAuthority::capture)
        .collect::<Vec<_>>();
    assert_eq!(
        captured_safety(&branches, &authorities, &[], ColumnTypeUse::Unverified),
        PoolTypeSafety::ProvenSafe,
        "the first core source must remain the alias authority"
    );

    for branches in [
        vec![
            optional_branch(0, LogicalSource::Table("shared".to_owned())),
            optional_branch(1, LogicalSource::Table("shared".to_owned())),
        ],
        vec![
            condition_branch(0, LogicalSource::Table("shared".to_owned())),
            condition_branch(1, LogicalSource::Table("shared".to_owned())),
        ],
    ] {
        let authorities = branches
            .iter()
            .map(PoolSourceAuthority::capture)
            .collect::<Vec<_>>();
        assert_eq!(
            captured_safety(&branches, &authorities, &[], ColumnTypeUse::Unverified),
            PoolTypeSafety::ProvenSafe,
            "OPTIONAL and condition scans remain physical-source identity"
        );
    }

    for branches in [
        vec![
            optional_branch(0, LogicalSource::Table("left".to_owned())),
            optional_branch(1, LogicalSource::Table("right".to_owned())),
        ],
        vec![
            condition_branch(0, LogicalSource::Table("left".to_owned())),
            condition_branch(1, LogicalSource::Table("right".to_owned())),
        ],
    ] {
        let authorities = branches
            .iter()
            .map(PoolSourceAuthority::capture)
            .collect::<Vec<_>>();
        let schema = vec![
            TableSchema {
                columns: vec![Column::new("value", "text", false)],
                ..TableSchema::new("left")
            },
            TableSchema {
                columns: vec![Column::new("value", "text", false)],
                ..TableSchema::new("right")
            },
        ];
        assert_eq!(
            captured_safety(
                &branches,
                &authorities,
                &schema,
                ColumnTypeUse::CallerAuthorizedFrozen,
            ),
            PoolTypeSafety::Unproven,
            "only a core scan may supply catalog column-type authority"
        );
    }
}

#[test]
fn pre_d1_authority_ignores_generated_wrapper_text_and_reads_current_bindings() {
    let mut branches = vec![
        core_branch(4, LogicalSource::Table("shared".to_owned())),
        core_branch(9, LogicalSource::Table("shared".to_owned())),
    ];
    let authorities = branches
        .iter()
        .map(PoolSourceAuthority::capture)
        .collect::<Vec<_>>();

    force_distinct_for_dup_safety(&mut branches, &[], Dialect::Postgres);
    let wrappers = branches
        .iter()
        .map(|branch| {
            assert!(matches!(
                branch.core[0].source,
                crate::iq::ScanSource::Projection { .. }
            ));
            crate::emit::emit_branch(branch, Dialect::Postgres)
                .unwrap()
                .sql
        })
        .collect::<Vec<_>>();
    assert_ne!(
        wrappers[0], wrappers[1],
        "aliases make the wrapper text differ"
    );

    {
        let members = branches.iter().collect::<Vec<_>>();
        assert_eq!(
            group_pool_type_safety(&members, &[], Dialect::Postgres, ColumnTypeUse::Unverified,),
            PoolTypeSafety::Unproven,
            "post-D1 Query text is not physical-source identity"
        );
    }
    assert_eq!(
        captured_safety(&branches, &authorities, &[], ColumnTypeUse::Unverified),
        PoolTypeSafety::ProvenSafe,
        "the pre-D1 Table identity remains authoritative"
    );

    bind_column(&mut branches[1], "value", 9, "other");
    assert_eq!(
        captured_safety(&branches, &authorities, &[], ColumnTypeUse::Unverified),
        PoolTypeSafety::Unproven,
        "pooling must compare the branch's current bindings"
    );
    assert_eq!(
        group_pool_type_safety_with_source_authority(
            &branches.iter().collect::<Vec<_>>(),
            &[&authorities[0]],
            &[],
            Dialect::Postgres,
            ColumnTypeUse::Unverified,
        ),
        PoolTypeSafety::Unproven,
        "a broken branch-to-authority alignment must fail closed"
    );
}

#[test]
fn controlled_reference_origin_preserves_canonical_names_and_budget_boundary() {
    use crate::build::control::BuildWork;
    use crate::compiler_control::CompileContext;
    use crate::CompilerWorkMode;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    let source = crate::iq::ScanSource::RefAtom {
        input: Box::new(Branch::single(Scan {
            alias: 7,
            source: LogicalSource::Table("items".into()).into(),
        })),
        columns: vec![crate::iq::ColRef::new(7, "value")],
    };
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    for name in [
        "c0",
        "c00",
        "c+0",
        "c1",
        "C0",
        "",
        "c",
        "c١",
        "c184467440737095516160",
    ] {
        let snapshot = |origin: Option<(&LogicalSource, &str)>| {
            origin.map(|(source, name)| (format!("{source:?}"), name.to_owned()))
        };
        let expected = snapshot(source.raw_column_origin(name));
        let run = |control: &QueryBudget| {
            super::pool_raw_column_origin(
                &source,
                name,
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
            )
        };
        let measured = budget(u64::MAX);
        assert_eq!(snapshot(run(&measured).unwrap()), expected, "{name:?}");
        let units = measured.consumed(QueryCharge::CompilerWork);
        assert_eq!(snapshot(run(&budget(units)).unwrap()), expected);
        assert!(matches!(
            run(&budget(units - 1)),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
        let stopped = budget(u64::MAX);
        stopped.terminate(QueryControlError::Cancelled);
        assert!(run(&stopped).is_err());
    }
}

#[test]
fn controlled_physical_fallback_preserves_first_match_with_or_without_capture() {
    use crate::build::control::BuildWork;
    use crate::compiler_control::CompileContext;
    use crate::CompilerWorkMode;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    let mut branch = core_branch(7, LogicalSource::Table("first".into()));
    branch
        .core
        .extend(core_branch(7, LogicalSource::Table("duplicate".into())).core);
    branch.opts = optional_branch(9, LogicalSource::Table("optional".into())).opts;
    branch.where_conds = vec![SqlCond::Exists {
        scans: core_branch(11, LogicalSource::Query("nested".into())).core,
        conds: vec![],
    }];
    let captured = PoolSourceAuthority::capture(&branch);
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    for authority in [None, Some(&captured)] {
        for alias in [7, 9, 11, 999] {
            let expected = format!(
                "{:?}",
                super::pool_physical_source(&branch, authority, alias)
            );
            let run = |control: &QueryBudget| {
                super::pool_physical_source_with_work(
                    &branch,
                    authority,
                    alias,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
                )
            };
            let measured = budget(u64::MAX);
            assert_eq!(
                format!("{:?}", run(&measured).unwrap().as_deref()),
                expected
            );
            let units = measured.consumed(QueryCharge::CompilerWork);
            assert_eq!(
                format!("{:?}", run(&budget(units)).unwrap().as_deref()),
                expected
            );
            assert!(matches!(
                run(&budget(units - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerWorkExceeded
                ))
            ));
        }
    }
}

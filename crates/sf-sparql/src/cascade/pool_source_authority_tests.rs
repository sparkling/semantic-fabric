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
        .map(|branch| match branch.core[0].source.logical() {
            Some(LogicalSource::Query(query)) => query,
            other => panic!("D1 did not wrap {other:?}"),
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

use super::*;
use crate::iq::{OptJoin, ScanSource};
use sf_core::ir::Template;

fn projected(source: LogicalSource) -> Scan {
    Scan {
        alias: 7,
        source: ScanSource::Projection {
            input: Box::new(Scan {
                alias: 6,
                source: ScanSource::Projection {
                    input: Box::new(Scan {
                        alias: 5,
                        source: source.into(),
                    }),
                    columns: vec![(
                        "KEY".into(),
                        TermMap::Column("KEY".into(), TermSpec::plain_literal()),
                    )],
                    guards: vec![],
                    distinct: true,
                    native_keys: vec![],
                },
            }),
            columns: vec![(
                "value".into(),
                TermMap::Template(Template::parse("prefix/{KEY}").unwrap(), TermSpec::iri()),
            )],
            guards: vec![SqlCond::IsNotNull(ColRef::new(6, "KEY"))],
            distinct: false,
            native_keys: vec![],
        },
    }
}

fn plan(scan: Scan, position: usize) -> Plan {
    let mut branch = Branch::empty();
    let condition = SqlCond::IsNotNull(ColRef::new(7, "value"));
    match position {
        0 => branch.core.push(scan),
        1 => branch.opts.push(OptJoin {
            scan,
            on: vec![condition],
            extra: vec![],
        }),
        2 => branch.where_conds.push(SqlCond::Exists {
            scans: vec![scan],
            conds: vec![condition],
        }),
        3 => branch.where_conds.push(SqlCond::NotExists {
            scans: vec![scan],
            conds: vec![condition],
        }),
        _ => unreachable!(),
    }
    let mut plan = select_plan(vec![branch]);
    plan.dialect = Dialect::Postgres;
    plan
}

#[test]
fn nested_projection_positions_probe_only_original_sources_and_fold_live_columns() {
    for source in [
        LogicalSource::Table("items".into()),
        LogicalSource::Query("SELECT actual AS key FROM items /* AS KEY is not metadata */".into()),
    ] {
        for position in 0..4 {
            let mut backend = backend_with(vec![Ok(vec!["key".into()])]);
            run_select(&plan(projected(source.clone()), position), &mut backend).unwrap();
            assert_eq!(backend.probes, vec![Dialect::Postgres.probe_sql(&source)]);
            assert_eq!(backend.opens, 1);
            assert!(
                backend.sql[0].contains("t5.\"key\" AS \"KEY\""),
                "{}",
                backend.sql[0]
            );
            assert!(
                backend.sql[0].contains("t6.\"KEY\" IS NOT NULL"),
                "{}",
                backend.sql[0]
            );
        }
    }
}

#[test]
fn missing_or_ambiguous_leaf_names_reject_before_any_cursor() {
    for names in [vec!["wrong".into()], vec!["key".into(), "Key".into()]] {
        for position in 0..4 {
            let mut backend = backend_with(vec![Ok(names.clone())]);
            assert!(run_select(
                &plan(projected(LogicalSource::Table("items".into())), position),
                &mut backend
            )
            .is_err());
            assert_eq!(backend.opens, 0);
        }
    }
}

#[test]
fn wrapper_recipes_are_charged_by_plan_measurement() {
    use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};
    let small = projected(LogicalSource::Table("items".into()));
    let mut large = small.clone();
    let ScanSource::Projection { columns, .. } = &mut large.source else {
        unreachable!()
    };
    columns[0].0 = "x".repeat(4096).into();
    let a = measure_compiler_clone_root_v1(CompilerCloneRootV1::Scan(&small)).unwrap();
    let b = measure_compiler_clone_root_v1(CompilerCloneRootV1::Scan(&large)).unwrap();
    assert!(b.deep_clone_work > a.deep_clone_work);
}

#[test]
fn parameter_values_stay_outside_projection_recipes() {
    let mut plan = plan(projected(LogicalSource::Table("items".into())), 0);
    plan.branches[0].where_conds.push(SqlCond::Cmp(
        ColRef::new(7, "value"),
        crate::iq::CmpOp::Eq,
        "user-supplied-'value".into(),
    ));
    let emitted = crate::emit::emit_branch(&plan.branches[0], Dialect::Postgres).unwrap();
    assert_eq!(emitted.params, vec!["user-supplied-'value"]);
    assert!(!emitted.sql.contains("user-supplied"));
    assert!(emitted.sql.contains("$1"));
}

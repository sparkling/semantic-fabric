use super::*;
use crate::iq::{OptJoin, ScanSource};

fn scan() -> Scan {
    Scan {
        alias: 7,
        source: ScanSource::Path {
            closure: Box::new(PathClosure {
                alias: 7,
                kind: PathKind::OneOrMore,
                hop: HopExpr::Pred(HopRelation {
                    source: LogicalSource::Table("edges".into()),
                    subj_col: "PARENT".into(),
                    obj_col: "CHILD".into(),
                }),
            }),
            cte_alias: 8,
        },
    }
}

fn plan(position: usize) -> Plan {
    let path_scan = scan();
    let mut branch = Branch::empty();
    let condition = SqlCond::IsNotNull(ColRef::new(7, "sf_s"));
    match position {
        0 => branch.core.push(path_scan),
        1 => branch.opts.push(OptJoin {
            scan: path_scan,
            on: vec![condition],
            extra: vec![],
        }),
        2 => branch.where_conds.push(SqlCond::Exists {
            scans: vec![path_scan],
            conds: vec![condition],
        }),
        3 => branch.where_conds.push(SqlCond::NotExists {
            scans: vec![path_scan],
            conds: vec![condition],
        }),
        _ => unreachable!(),
    }
    let mut plan = select_plan(vec![branch]);
    plan.dialect = Dialect::Postgres;
    plan
}

#[test]
fn all_scan_positions_probe_physical_leaves_and_emit_with_live_names() {
    for position in 0..4 {
        let mut backend = backend_with(vec![Ok(vec!["parent".into(), "child".into()])]);
        run_select(&plan(position), &mut backend).unwrap();
        assert_eq!(
            backend.probes,
            vec![Dialect::Postgres.probe_sql(&LogicalSource::Table("edges".into()))]
        );
        assert_eq!(backend.opens, 1);
        assert!(
            backend.sql[0].contains("h0.\"parent\""),
            "{}",
            backend.sql[0]
        );
        assert!(!backend.sql[0].contains("\"PARENT\""));
        assert!(
            backend.sql[0].contains("t8r"),
            "internal CTE alias stays separate"
        );
    }
}

#[test]
fn missing_path_leaf_column_fails_before_every_branch_cursor() {
    for position in 0..4 {
        let mut backend = backend_with(vec![Ok(vec!["parent".into(), "wrong".into()])]);
        assert!(run_select(&plan(position), &mut backend).is_err());
        assert_eq!(backend.opens, 0);
    }
}

#[test]
fn a_deferred_path_still_requires_source_sized_order_admission() {
    let mut plan = plan(0);
    plan.order.push(crate::iq::OrderKey {
        var: "v".into(),
        descending: false,
        expr: None,
    });
    assert!(plan
        .source_sized_states()
        .contains(&crate::resource_profile::SourceSizedState::GlobalOrder));
}

#[test]
fn invalid_derived_output_rejects_before_opening_a_cursor() {
    let mut plan = plan(0);
    plan.branches[0].bindings.insert(
        "v".into(),
        TermDef::Derived {
            alias: 7,
            term_map: TermMap::Column("not_a_path_output".into(), TermSpec::plain_literal()),
        },
    );
    let mut backend = backend_with(vec![Ok(vec!["parent".into(), "child".into()])]);
    assert!(run_select(&plan, &mut backend).is_err());
    assert_eq!(backend.opens, 0);
}

#[test]
fn path_recipe_payload_is_charged_by_the_existing_clone_measure() {
    use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};
    let small = scan();
    let mut large = small.clone();
    let ScanSource::Path { closure, .. } = &mut large.source else {
        unreachable!()
    };
    let HopExpr::Pred(hop) = &mut closure.hop else {
        unreachable!()
    };
    hop.subj_col = "P".repeat(4096).into();
    let a = measure_compiler_clone_root_v1(CompilerCloneRootV1::Scan(&small)).unwrap();
    let b = measure_compiler_clone_root_v1(CompilerCloneRootV1::Scan(&large)).unwrap();
    assert!(b.deep_clone_work > a.deep_clone_work);
}

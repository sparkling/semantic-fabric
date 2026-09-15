use super::*;
use crate::iq::Branch;

#[test]
fn recipe_equality_covers_template_spec_and_literal_fields() {
    use crate::iq::TermDef;
    use sf_core::ir::{Template, TermMap, TermSpec};
    use sf_core::{BlankNode, Literal, NamedNode};
    use sf_sql::source_work::SourceWork;
    let mut values = Vec::new();
    let mut specs = vec![TermSpec::iri(), TermSpec::blank_node()];
    for field in 0..3 {
        let mut spec = TermSpec::iri();
        match field {
            0 => spec.base = Some("urn:base:".into()),
            1 => spec.language = Some("en".into()),
            _ => spec.datatype = Some(NamedNode::new("urn:type").unwrap()),
        }
        specs.push(spec);
    }
    for spec in specs {
        for template in ["urn:{x}", "urn:{y}", "other:{x}", "literal"] {
            values.push(TermDef::Derived {
                alias: 1,
                term_map: TermMap::Template(Template::parse(template).unwrap(), spec.clone()),
            });
        }
    }
    for literal in [
        Literal::new_simple_literal("x"),
        Literal::new_simple_literal("y"),
        Literal::new_language_tagged_literal("x", "en").unwrap(),
        Literal::new_language_tagged_literal("x", "fr").unwrap(),
        Literal::new_typed_literal("x", NamedNode::new("urn:type").unwrap()),
    ] {
        values.push(TermDef::Const(literal.clone().into()));
        values.push(TermDef::Derived {
            alias: 0,
            term_map: TermMap::Constant(literal.into()),
        });
    }
    for name in ["a", "b"] {
        values.push(TermDef::Const(BlankNode::new(name).unwrap().into()));
    }
    for a in &values {
        for b in &values {
            assert_eq!(
                recipes_same(a, b, SourceWork::new(None)).unwrap(),
                format!("{a:?}") == format!("{b:?}")
            );
        }
    }
}

#[test]
fn source_validation_does_not_fund_later_root_siblings_before_first_error() {
    use crate::iq::{Scan, ScanSource};
    use sf_core::query_control::{QueryBudget, QueryLimits};
    for later in [0, 10_000] {
        let malformed = Branch::single(Scan {
            alias: 0,
            source: ScanSource::RefAtom {
                input: Box::new(Branch::empty()),
                columns: vec![],
            },
        });
        let mut branches = vec![malformed];
        branches.extend((0..later).map(|_| Branch::empty()));
        let control = QueryBudget::new(QueryLimits::new(u64::MAX, 8192, u64::MAX, u64::MAX));
        assert!(matches!(crate::emit::validate_live_columns_controlled(
            &branches, sf_sql::Dialect::Sqlite, &crate::emit::ColumnCatalog::default(),
            sf_sql::source_work::SourceWork::new(Some(&control))),
            Err(crate::Error::Unsupported(message)) if message == "invalid reference atom relation"));
    }
}

#[test]
fn mixed_source_validation_checks_nested_scan_before_caller_conditions() {
    use crate::iq::{ColRef, Scan, ScanSource, SqlCond};
    use sf_core::ir::{LogicalSource, TermMap, TermSpec};
    let source = LogicalSource::Table("t".into());
    let mut relation = Branch::empty();
    relation.core = (0..2)
        .map(|alias| Scan {
            alias,
            source: source.clone().into(),
        })
        .collect();
    relation.where_conds.push(SqlCond::NativeColEq(
        ColRef::new(0, "key"),
        ColRef::new(1, "key"),
    ));
    let projection = Scan {
        alias: 3,
        source: ScanSource::Projection {
            input: Box::new(Scan {
                alias: 2,
                source: ScanSource::RefAtom {
                    input: Box::new(relation),
                    columns: vec![],
                },
            }),
            columns: vec![("x".into(), TermMap::Column("c0".into(), TermSpec::iri()))],
            guards: vec![],
            distinct: false,
            native_keys: vec![],
            lexical_keys: vec![],
        },
    };
    let mut root = Branch::single(Scan {
        alias: 0,
        source: source.clone().into(),
    });
    root.where_conds.push(SqlCond::Exists {
        scans: vec![projection],
        conds: vec![SqlCond::IsNull(ColRef::new(0, "missing"))],
    });
    let mut catalog = crate::emit::ColumnCatalog::default();
    catalog.insert(&source, vec!["key".into()]);
    let run = |root: &Branch| {
        crate::emit::validate_live_columns_controlled(
            std::slice::from_ref(root),
            sf_sql::Dialect::Sqlite,
            &catalog,
            sf_sql::source_work::SourceWork::new(None),
        )
    };
    assert!(run(&root)
        .unwrap_err()
        .to_string()
        .contains("missing reference atom output"));
    let SqlCond::Exists { scans, .. } = &mut root.where_conds[0] else {
        unreachable!()
    };
    let ScanSource::Projection { columns, .. } = &mut scans[0].source else {
        unreachable!()
    };
    columns.clear();
    assert!(run(&root)
        .unwrap_err()
        .to_string()
        .contains("missing a required result column"));
}

#[test]
fn mixed_source_validation_has_no_recursive_branch_scan_cycle() {
    use crate::iq::{ColRef, Scan, ScanSource, SqlCond};
    use sf_core::ir::LogicalSource;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let source = LogicalSource::Table("t".into());
            let base = || {
                let mut branch = Branch::empty();
                branch.core = (0..2)
                    .map(|alias| Scan {
                        alias,
                        source: source.clone().into(),
                    })
                    .collect();
                branch.where_conds.push(SqlCond::NativeColEq(
                    ColRef::new(0, "key"),
                    ColRef::new(1, "key"),
                ));
                branch
            };
            let mut branch = base();
            for _ in 0..4096 {
                let projection = Scan {
                    alias: 3,
                    source: ScanSource::Projection {
                        input: Box::new(Scan {
                            alias: 2,
                            source: ScanSource::RefAtom {
                                input: Box::new(branch),
                                columns: vec![],
                            },
                        }),
                        columns: vec![],
                        guards: vec![],
                        distinct: false,
                        native_keys: vec![],
                        lexical_keys: vec![],
                    },
                };
                branch = base();
                branch.where_conds.push(SqlCond::Exists {
                    scans: vec![projection],
                    conds: vec![],
                });
            }
            let mut catalog = crate::emit::ColumnCatalog::default();
            catalog.insert(&source, vec!["key".into()]);
            let budget =
                |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
            let run = |control: &QueryBudget| {
                crate::emit::validate_live_columns_controlled(
                    std::slice::from_ref(&branch),
                    sf_sql::Dialect::Sqlite,
                    &catalog,
                    SourceWork::new(Some(control)),
                )
            };
            let measured = budget(u64::MAX);
            run(&measured).unwrap();
            let total = measured.consumed(QueryCharge::SourceWork);
            run(&budget(total)).unwrap();
            assert!(matches!(
                run(&budget(total - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::SourceWorkExceeded
                ))
            ));
            // Disassemble the test fixture, not the separate recursive IQ Drop.
            while let Some(SqlCond::Exists { mut scans, .. }) = branch.where_conds.pop() {
                let ScanSource::Projection { input, .. } = scans.pop().unwrap().source else {
                    unreachable!()
                };
                let ScanSource::RefAtom { input, .. } = input.source else {
                    unreachable!()
                };
                branch = *input;
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn source_validation_restores_parent_aliases_between_subplans() {
    use crate::iq::{ColRef, Scan, SqlCond, SubPlanJoin};
    use sf_core::ir::LogicalSource;
    let outer = LogicalSource::Table("outer".into());
    let inner = LogicalSource::Table("inner".into());
    let condition = |name: &str| SqlCond::IsNotNull(ColRef::new(0, name));
    let mut root = Branch::single(Scan {
        alias: 0,
        source: outer.clone().into(),
    });
    let mut child = Branch::single(Scan {
        alias: 0,
        source: inner.clone().into(),
    });
    child.where_conds.push(condition("inner_only"));
    let plan = |branches| crate::Plan {
        branches,
        form: crate::PlanForm::Select { vars: vec![] },
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        rust_group: None,
        dialect: sf_sql::Dialect::Sqlite,
        dedup_scopes: vec![],
        construct_drops_some_branch_var: false,
    };
    root.subplan_joins = vec![
        SubPlanJoin {
            alias: 1,
            plan: Box::new(plan(vec![child])),
            on: vec![condition("outer_only")],
            left: false,
        },
        SubPlanJoin {
            alias: 2,
            plan: Box::new(plan(vec![])),
            on: vec![condition("outer_only")],
            left: false,
        },
    ];
    let mut catalog = crate::emit::ColumnCatalog::default();
    catalog.insert(
        &outer,
        vec!["outer_only".into(), "Key".into(), "KEY".into()],
    );
    catalog.insert(&inner, vec!["inner_only".into()]);
    let run = |root: &Branch| {
        crate::emit::validate_live_columns_controlled(
            std::slice::from_ref(root),
            sf_sql::Dialect::Sqlite,
            &catalog,
            sf_sql::source_work::SourceWork::new(None),
        )
    };
    run(&root).unwrap();
    // First child's failure precedes the next sibling's ON failure.
    root.subplan_joins[0].plan.branches[0].where_conds = vec![condition("missing")];
    root.subplan_joins[1].on = vec![condition("key")];
    assert!(run(&root)
        .unwrap_err()
        .to_string()
        .contains("missing a required"));
    root.subplan_joins[0].plan.branches[0].where_conds.clear();
    assert!(run(&root)
        .unwrap_err()
        .to_string()
        .contains("ambiguous result-column"));
}

#[test]
fn source_validation_subplans_use_an_owned_stack_and_cumulative_allowance() {
    use crate::iq::SubPlanJoin;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut branch = Branch::empty();
            for _ in 0..4096 {
                let plan = crate::Plan {
                    branches: vec![branch, Branch::empty()],
                    form: crate::PlanForm::Select { vars: vec![] },
                    distinct: false,
                    limit: None,
                    offset: 0,
                    order: vec![],
                    rust_group: None,
                    dialect: sf_sql::Dialect::Sqlite,
                    dedup_scopes: vec![],
                    construct_drops_some_branch_var: false,
                };
                branch = Branch::empty();
                branch.subplan_joins.push(SubPlanJoin {
                    alias: 0,
                    plan: Box::new(plan),
                    on: vec![],
                    left: false,
                });
            }
            let catalog = crate::emit::ColumnCatalog::default();
            let budget =
                |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
            let run = |control: &QueryBudget| {
                crate::emit::validate_live_columns_controlled(
                    std::slice::from_ref(&branch),
                    sf_sql::Dialect::Sqlite,
                    &catalog,
                    SourceWork::new(Some(control)),
                )
            };
            let measured = budget(u64::MAX);
            run(&measured).unwrap();
            let total = measured.consumed(QueryCharge::SourceWork);
            run(&budget(total)).unwrap();
            assert!(matches!(
                run(&budget(total - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::SourceWorkExceeded
                ))
            ));
            // Disassemble only the fixture's ownership spine; recursive IQ Drop
            // is a separate lifetime boundary, not what this validator tests.
            while let Some(mut join) = branch.subplan_joins.pop() {
                branch = join.plan.branches.remove(0);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn source_validation_uses_owned_stacks_for_deep_conditions_and_hops() {
    use crate::iq::{HopExpr, PathClosure, PathKind, SqlCond};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut condition = SqlCond::ExpressionError;
            let mut hop = HopExpr::Alt(vec![]);
            for _ in 0..4096 {
                condition = SqlCond::Exists {
                    scans: vec![],
                    conds: vec![SqlCond::Not(Box::new(condition))],
                };
                hop = HopExpr::Inverse(Box::new(hop));
            }
            let mut branch = Branch::empty();
            branch.where_conds.push(condition);
            branch.path = Some(PathClosure {
                alias: 0,
                kind: PathKind::One,
                hop,
            });
            let catalog = crate::emit::ColumnCatalog::default();
            let budget =
                |units| QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX));
            let run = |control: &QueryBudget| {
                crate::emit::validate_live_columns_controlled(
                    std::slice::from_ref(&branch),
                    sf_sql::Dialect::Sqlite,
                    &catalog,
                    SourceWork::new(Some(control)),
                )
            };
            let measured = budget(u64::MAX);
            run(&measured).unwrap();
            let total = measured.consumed(QueryCharge::SourceWork);
            run(&budget(total)).unwrap();
            assert!(matches!(
                run(&budget(total - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::SourceWorkExceeded
                ))
            ));
            // This test exercises validator stack ownership, not recursive IQ Drop.
            let mut condition = branch.where_conds.pop().unwrap();
            loop {
                condition = match condition {
                    SqlCond::Exists { mut conds, .. } => conds.pop().unwrap(),
                    SqlCond::Not(inner) => *inner,
                    SqlCond::ExpressionError => break,
                    _ => unreachable!(),
                };
            }
            let mut hop = branch.path.take().unwrap().hop;
            while let HopExpr::Inverse(inner) = hop {
                hop = *inner;
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn borrowed_preparation_matches_raw_modifier_law_without_mutating_input() {
    for count in [0, 1, 2] {
        for distinct in [false, true] {
            for unordered in [false, true] {
                for limit in [None, Some(0), Some(3)] {
                    for offset in [0, 2] {
                        let mut branches = vec![Branch::empty(); count];
                        for branch in &mut branches {
                            branch.distinct = true;
                            branch.limit = Some(7);
                            branch.offset = 1;
                        }
                        let original = format!("{branches:?}");
                        let mut expected = branches.clone();
                        if let [branch] = expected.as_mut_slice() {
                            branch.distinct = distinct;
                            if unordered {
                                branch.limit = limit;
                                branch.offset = offset;
                            }
                        }
                        for (branch, expected) in branches.iter().zip(&expected) {
                            assert_eq!(
                                crate::emit::BranchModifiers::prepared(
                                    branch,
                                    count == 1,
                                    distinct,
                                    unordered,
                                    limit,
                                    offset,
                                ),
                                crate::emit::BranchModifiers::stored(expected)
                            );
                        }
                        assert_eq!(format!("{branches:?}"), original);
                    }
                }
            }
        }
    }
}

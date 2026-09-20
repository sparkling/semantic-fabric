use super::*;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
}
fn fixture() -> PathClosure {
    PathClosure {
        alias: 3,
        kind: PathKind::OneOrMore,
        hop: HopExpr::Pred(crate::iq::HopRelation {
            source: LogicalSource::Table("edges".into()),
            subj_col: "s".into(),
            obj_col: "o".into(),
        }),
    }
}

fn run_edge(edge: usize, work: SourceWork<'_>) -> Result<String> {
    let pc = fixture();
    let catalog = ColumnCatalog::default();
    let mut params = Vec::new();
    let mut index = 0;
    let sql = match edge {
        0 => {
            let mut branch = Branch::empty();
            branch.bindings.insert(
                "s".into(),
                TermDef::Derived {
                    alias: 3,
                    term_map: TermMap::Column("sf_s".into(), sf_core::ir::TermSpec::iri()),
                },
            );
            emit_path_branch(
                &branch,
                &pc,
                Dialect::Sqlite,
                &catalog,
                BranchModifiers::stored(&branch),
                work,
            )?
            .sql
        }
        1 => render_cond_controlled(
            &SqlCond::PathExists {
                pc,
                conds: vec![],
                negated: false,
            },
            Dialect::Sqlite,
            &catalog,
            &HashMap::new(),
            &mut params,
            &mut index,
            work,
        )?,
        2 => scan::scan_ref_controlled(
            &crate::iq::Scan {
                alias: 3,
                source: crate::iq::ScanSource::Path {
                    closure: Box::new(pc),
                    cte_alias: 7,
                },
            },
            Dialect::Sqlite,
            &catalog,
            &mut params,
            &mut index,
            work,
        )?,
        _ => unreachable!(),
    };
    assert!(params.is_empty());
    assert_eq!(index, 0);
    Ok(sql)
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
fn production_edges_have_fixed_accounting_and_sticky_boundaries() {
    for (edge, fixed_units) in [3958, 2078, 2953].into_iter().enumerate() {
        let meter = budget(u64::MAX);
        let expected = run_edge(edge, SourceWork::new(Some(&meter))).unwrap();
        let units = meter.consumed(QueryCharge::SourceWork);
        assert_eq!(units, fixed_units, "production edge {edge}");
        assert_eq!(meter.consumed(QueryCharge::CompilerWork), 0);
        assert_eq!(expected, run_edge(edge, SourceWork::new(None)).unwrap());
        assert_eq!(
            expected,
            run_edge(edge, SourceWork::new(Some(&budget(units)))).unwrap()
        );
        let short = budget(units - 1);
        assert!(matches!(
            run_edge(edge, SourceWork::new(Some(&short))),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
        assert_eq!(
            short.checkpoint(),
            Err(QueryControlError::SourceWorkExceeded)
        );
    }
}

#[test]
fn every_charge_can_interrupt_without_resetting_terminal_cause() {
    let pc = fixture();
    let catalog = ColumnCatalog::default();
    let measured = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at: usize::MAX,
        reason: QueryControlError::Cancelled,
    };
    prelude(
        &pc,
        pc.alias,
        Dialect::Sqlite,
        &catalog,
        SourceWork::new(Some(&measured)),
    )
    .unwrap();
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
                matches!(prelude(&pc,pc.alias,Dialect::Sqlite,&catalog,SourceWork::new(Some(&control))),Err(Error::QueryControl(found)) if found==reason)
            );
            assert_eq!(control.checkpoint(), Err(reason));
        }
    }
}

#[test]
fn rejected_format_charge_precedes_owned_output_copy() {
    let short = budget(3);
    format::COPIES.with(|count| count.set(0));
    assert!(format::render(SourceWork::new(Some(&short)), format_args!("long output")).is_err());
    assert_eq!(format::COPIES.with(|count| count.get()), 0);
}

#[test]
fn derived_alias_borrows_original_closure() {
    let pc = fixture();
    let sql = derived(
        &pc,
        77,
        Dialect::Postgres,
        &ColumnCatalog::default(),
        SourceWork::new(None),
    )
    .unwrap();
    assert_eq!(pc.alias, 3);
    assert!(sql.starts_with("WITH RECURSIVE t77r"));
    assert!(sql.ends_with("FROM t77"));
}

#[test]
fn nested_and_wide_hops_pay_more_and_stop_at_the_same_boundary() {
    let leaf = fixture().hop;
    let mut deep = leaf.clone();
    for _ in 0..64 {
        deep = HopExpr::Inverse(Box::new(deep));
    }
    let mut previous = 0;
    for hop in [leaf.clone(), HopExpr::Alt(vec![leaf; 16]), deep] {
        let pc = PathClosure {
            alias: 3,
            kind: PathKind::One,
            hop,
        };
        let meter = budget(u64::MAX);
        let expected = prelude(
            &pc,
            3,
            Dialect::Sqlite,
            &ColumnCatalog::default(),
            SourceWork::new(Some(&meter)),
        )
        .unwrap();
        let units = meter.consumed(QueryCharge::SourceWork);
        assert!(units > previous);
        previous = units;
        assert_eq!(
            expected,
            prelude(
                &pc,
                3,
                Dialect::Sqlite,
                &ColumnCatalog::default(),
                SourceWork::new(Some(&budget(units)))
            )
            .unwrap()
        );
        assert!(matches!(
            prelude(
                &pc,
                3,
                Dialect::Sqlite,
                &ColumnCatalog::default(),
                SourceWork::new(Some(&budget(units - 1)))
            ),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
    }
}

#[test]
fn metadata_quoting_authored_sources_and_rowid_preserve_sql() {
    let mut pc = fixture();
    let HopExpr::Pred(ref mut rel) = pc.hop else {
        unreachable!()
    };
    rel.source = LogicalSource::Query("SELECT \"S\", \"o\" FROM \"odd\"\"table\"".into());
    let mut catalog = ColumnCatalog::default();
    catalog.insert(&rel.source, vec!["S".into(), "o".into()]);
    let sql = prelude(&pc, 3, Dialect::Postgres, &catalog, SourceWork::new(None)).unwrap();
    assert!(sql.contains("h0.\"S\""));
    assert!(sql.contains("FROM (SELECT \"S\", \"o\" FROM \"odd\"\"table\") h0"));
    let HopExpr::Pred(ref mut rel) = pc.hop else {
        unreachable!()
    };
    rel.source = LogicalSource::Table("odd\"table".into());
    rel.subj_col = "rowid".into();
    let sql = prelude(&pc, 3, Dialect::Postgres, &catalog, SourceWork::new(None)).unwrap();
    assert!(sql.contains("(h0.ctid)::text"));
    assert!(sql.contains("FROM \"odd\"\"table\" h0"));
    assert_eq!(
        leaf::quote("a`b", Dialect::MySql, SourceWork::new(None)).unwrap(),
        "`a``b`"
    );
}

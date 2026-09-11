use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::iq::node::{BindDef, ColOrConst, IqCond, IqNode};
use crate::iq::{Scan, SqlCond, TermDef};
use crate::{Error, Result};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

pub(super) fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}
pub(super) fn run(node: IqNode, control: &dyn QueryControl) -> Result<IqNode> {
    super::normalize_with_work_control(node, control)
}
pub(super) fn data(alias: usize, vars: &[&str]) -> IqNode {
    IqNode::Extensional {
        scan: Scan {
            alias,
            source: sf_core::ir::LogicalSource::Table("items".into()).into(),
        },
        bind: vars
            .iter()
            .map(|v| ((*v).into(), ColOrConst::Col((*v).into())))
            .collect(),
    }
}
pub(super) fn value(value: &str) -> BindDef {
    BindDef::Resolved(TermDef::Const(sf_core::Literal::from(value).into()))
}
pub(super) fn construction(child: IqNode, bindings: &[(&str, BindDef)]) -> IqNode {
    IqNode::Construction {
        child: Box::new(child),
        subst: bindings
            .iter()
            .map(|(v, d)| ((*v).into(), d.clone()))
            .collect(),
        project: bindings.iter().map(|(v, _)| (*v).into()).collect(),
    }
}
pub(super) fn filter(child: IqNode, cond: Vec<IqCond>) -> IqNode {
    IqNode::Filter {
        child: Box::new(child),
        cond,
    }
}
pub(super) fn union() -> IqNode {
    IqNode::Union {
        children: vec![data(3, &["y"]), data(1, &["x"]), data(3, &["y"])],
        project: vec!["y".into(), "x".into()],
    }
}
pub(super) fn truth() -> IqCond {
    IqCond::Sql(SqlCond::And(vec![]))
}

/// Test-setup-only raw BUILD/RESOLVE. No source connection is created.
pub(super) fn resolved(query: &str) -> IqNode {
    let maps = sf_mapping::parse_r2rml(
        r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <http://ex/map> a rr:TriplesMap;
          rr:logicalTable [rr:tableName "items"];
          rr:subjectMap [rr:template "http://ex/{id}"];
          rr:predicateObjectMap [rr:predicate <http://ex/p>;
            rr:objectMap [rr:template "http://ex/{parent}"]].
    "#,
    )
    .unwrap();
    let spargebra::Query::Select { pattern, .. } = crate::parse_query(query).unwrap() else {
        panic!("SELECT fixture");
    };
    let tree = crate::build::build_tree(&pattern, None).unwrap();
    let tbox = crate::Tbox::default();
    let mut cx = crate::iq::resolve::ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite, &[]);
    crate::iq::resolve::resolve(tree, &mut cx).unwrap()
}

fn same_outcome(raw: &Result<IqNode>, metered: &Result<IqNode>) {
    match (raw, metered) {
        (Ok(a), Ok(b)) => assert_eq!(format!("{a:?}"), format!("{b:?}")),
        // Controlled symbolic-error text is bounded/redacted, but remains 501.
        (Err(Error::Unsupported(_)), Err(Error::Unsupported(_))) => (),
        _ => panic!("normalization outcome changed: {raw:?} != {metered:?}"),
    }
}

pub(super) fn exact_and_short(tree: IqNode) {
    let raw = super::normalize(tree.clone());
    let observed = budget(u64::MAX);
    same_outcome(&raw, &run(tree.clone(), &observed));
    let work = observed.consumed(QueryCharge::CompilerWork);
    assert!(work > 0);
    let exact = budget(work);
    same_outcome(&raw, &run(tree.clone(), &exact));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), work);
    let short = budget(work - 1);
    assert!(matches!(
        run(tree, &short),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(
        short.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
    assert_eq!(observed.consumed(QueryCharge::SourceWork), 0);
}

struct StopAtCharge {
    budget: QueryBudget,
    calls: AtomicUsize,
    stop: usize,
    cause: QueryControlError,
}
impl QueryControl for StopAtCharge {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn consume(
        &self,
        charge: QueryCharge,
        amount: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(charge, amount)?;
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.stop {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}
pub(super) fn every_stop(tree: IqNode) {
    let observed = StopAtCharge {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        stop: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    same_outcome(
        &super::normalize(tree.clone()),
        &run(tree.clone(), &observed),
    );
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for stop in 1..=observed.calls.load(Ordering::SeqCst) {
            let c = StopAtCharge {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                stop,
                cause,
            };
            assert!(
                matches!(run(tree.clone(), &c), Err(Error::QueryControl(e)) if e == cause),
                "stop {stop}: {tree:?}"
            );
            assert_eq!(c.calls.load(Ordering::SeqCst), stop);
            assert_eq!(c.terminate(QueryControlError::CompilerWorkExceeded), cause);
        }
    }
}

pub(super) fn empty_projection(child: IqNode) -> IqNode {
    IqNode::Construction {
        child: Box::new(child),
        subst: BTreeMap::new(),
        project: vec![],
    }
}

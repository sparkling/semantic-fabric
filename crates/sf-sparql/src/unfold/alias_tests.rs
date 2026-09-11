use super::*;
use crate::compiler_control::CompileContext;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use crate::CompilerWorkMode;
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

#[test]
fn alias_work_shared_resolver_counter_is_checked_and_failure_atomic() {
    let tbox = Tbox::default();
    for metered in [false, true] {
        let c = budget(1);
        let mut uf = Unfolder::new(&[], &tbox, sf_sql::Dialect::Sqlite, &[]);
        if metered {
            uf = uf.with_work_mode(CompilerWorkMode::Metered(CompileContext::new(&c)));
        }
        uf.next_alias = usize::MAX - 1;
        assert_eq!(uf.alias().unwrap(), usize::MAX - 1);
        assert_eq!(uf.next_alias, usize::MAX);
        assert!(matches!(
            uf.alias(),
            Err(Error::QueryControl(QueryControlError::AccountingOverflow))
        ));
        assert_eq!(uf.next_alias, usize::MAX);
        if metered {
            assert_eq!(c.consumed(QueryCharge::CompilerWork), 1);
            assert_eq!(c.checkpoint(), Err(QueryControlError::AccountingOverflow));
        }
    }
    every_stop(|c| {
        let mut uf = Unfolder::new(&[], &tbox, sf_sql::Dialect::Sqlite, &[])
            .with_work_mode(CompilerWorkMode::Metered(CompileContext::new(c)));
        let result = uf.alias();
        if result.is_err() {
            assert_eq!(uf.next_alias, 0);
        }
        result
    });
}

#[test]
fn alias_work_flat_pool_counter_commits_only_after_success() {
    let arms = || {
        (1..=2)
            .map(|alias| {
                let mut b = Branch::single(Scan {
                    alias,
                    source: sf_core::ir::LogicalSource::Table("items".into()).into(),
                });
                b.bindings.insert(
                    "value".into(),
                    TermDef::Derived {
                        alias,
                        term_map: TermMap::Column(
                            "value".into(),
                            sf_core::ir::TermSpec::plain_literal(),
                        ),
                    },
                );
                b
            })
            .collect()
    };
    let vars = ["value".into()];
    let expected = pool_group(
        arms(),
        &vars,
        sf_sql::Dialect::Sqlite,
        &mut 7,
        CompilerWorkMode::Uncontrolled,
    )
    .unwrap();
    exact_and_short(expected, |c| {
        let mut next = 7;
        let result = pool_group(
            arms(),
            &vars,
            sf_sql::Dialect::Sqlite,
            &mut next,
            CompilerWorkMode::Metered(CompileContext::new(c)),
        );
        assert_eq!(next, if result.is_ok() { 8 } else { 7 });
        result
    });
    every_stop(|c| {
        let mut next = 7;
        let result = pool_group(
            arms(),
            &vars,
            sf_sql::Dialect::Sqlite,
            &mut next,
            CompilerWorkMode::Metered(CompileContext::new(c)),
        );
        assert_eq!(next, if result.is_ok() { 8 } else { 7 });
        result
    });
    let c = budget(1);
    let mut next = usize::MAX;
    assert!(matches!(
        pool_group(
            arms(),
            &vars,
            sf_sql::Dialect::Sqlite,
            &mut next,
            CompilerWorkMode::Metered(CompileContext::new(&c))
        ),
        Err(Error::QueryControl(QueryControlError::AccountingOverflow))
    ));
    assert_eq!(next, usize::MAX);
    assert_eq!(c.checkpoint(), Err(QueryControlError::AccountingOverflow));
}

use super::*;
use crate::compiler_control::CompileContext;
use sf_core::ir::{Template, TermSpec};
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

#[test]
fn rendered_projection_matches_raw_and_refusal_preserves_branch() {
    let mut source = Branch::single(Scan {
        alias: 0,
        source: LogicalSource::Table("source".into()).into(),
    });
    source.distinct = true;
    source.bindings.insert(
        "subject".into(),
        TermDef::Derived {
            alias: 0,
            term_map: TermMap::Template(
                Template::parse("{a}{b}").unwrap(),
                TermSpec::iri().with_base("http://example/"),
            ),
        },
    );
    source
        .where_conds
        .push(SqlCond::IsNotNull(ColRef::new(0, "a")));
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    let run = |branch: &mut Branch, control: &QueryBudget| {
        wrap_with_work(
            branch,
            sf_sql::Dialect::Sqlite,
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
        )
    };
    let mut expected = source.clone();
    assert!(super::super::wrap_raw(
        &mut expected,
        sf_sql::Dialect::Sqlite
    ));
    let measured = budget(u64::MAX);
    let mut actual = source.clone();
    assert!(run(&mut actual, &measured).unwrap());
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    let units = measured.consumed(QueryCharge::CompilerWork);
    let mut exact = source.clone();
    assert!(run(&mut exact, &budget(units)).unwrap());
    for allowed in 0..units {
        let mut refused = source.clone();
        assert!(matches!(
            run(&mut refused, &budget(allowed)),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
        assert_eq!(format!("{refused:?}"), format!("{source:?}"));
    }
    for condition in [
        SqlCond::IsNull(ColRef::new(1, "foreign")),
        SqlCond::Not(Box::new(SqlCond::IsNotNull(ColRef::new(0, "a")))),
    ] {
        let mut branch = source.clone();
        branch.where_conds.push(condition);
        let mut expected = branch.clone();
        let accepted = super::super::wrap_raw(&mut expected, sf_sql::Dialect::Sqlite);
        assert_eq!(run(&mut branch, &budget(u64::MAX)).unwrap(), accepted);
        assert_eq!(format!("{branch:?}"), format!("{expected:?}"));
    }
}

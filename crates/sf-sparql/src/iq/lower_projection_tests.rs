use super::projection::*;
use super::*;
use crate::compiler_control::CompileContext;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use sf_core::query_control::{QueryCharge, QueryControl};

fn work(control: &dyn QueryControl) -> BuildWork<'_> {
    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)))
}
fn bindings() -> BTreeMap<String, TermDef> {
    ["drop", "keep", "東京", "é"]
        .into_iter()
        .map(|k| {
            (
                k.into(),
                TermDef::Const(sf_core::Literal::from("payload".repeat(200)).into()),
            )
        })
        .collect()
}
fn pointer(term: &TermDef) -> *const u8 {
    let TermDef::Const(sf_core::Term::Literal(literal)) = term else {
        panic!()
    };
    literal.value().as_ptr()
}

#[test]
fn projection_work_exact_decisions_preserve_payload_and_failure_is_atomic() {
    let project: Vec<Var> = ["xxxx", "keep", "keep", "é"].map(Into::into).into();
    let extra = HashSet::from(["東京".into(), "other".into()]);
    // Entry1 + bool slots/payload8 + key visits4 + project work36 + retain4;
    // two extra scans each visit2, one compares6 bytes, and both pay capacity.
    let exact = 63 + 2 * extra.capacity() as u64;
    let mut source = bindings();
    let kept_pointer = pointer(&source["keep"]);
    let control = budget(exact);
    retain_bindings(&mut source, &project, &extra, work(&control)).unwrap();
    assert_eq!(control.consumed(QueryCharge::CompilerWork), exact);
    assert_eq!(
        source.keys().map(String::as_str).collect::<Vec<_>>(),
        ["keep", "é", "東京"]
    );
    assert_eq!(pointer(&source["keep"]), kept_pointer);
    let run = |control: &dyn QueryControl| {
        let mut source = bindings();
        let before = format!("{source:?}");
        let result = retain_bindings(&mut source, &project, &extra, work(control));
        if result.is_err() {
            assert_eq!(format!("{source:?}"), before);
        }
        result.map(|()| source)
    };
    exact_and_short(source, run);
    every_stop(run);
}

#[test]
fn projection_work_project_hit_skips_extra_capacity_and_random_order_is_invariant() {
    let source = bindings();
    let all: Vec<Var> = source.keys().map(|s| s.as_str().into()).collect();
    let mut extra = HashSet::with_capacity(4096);
    extra.extend(["keep".into(), "東京".into()]);
    let mut costs = Vec::new();
    for extra in [&HashSet::new(), &extra] {
        let mut b = source.clone();
        let control = budget(u64::MAX);
        retain_bindings(&mut b, &all, extra, work(&control)).unwrap();
        assert_eq!(format!("{b:?}"), format!("{source:?}"));
        costs.push(control.consumed(QueryCharge::CompilerWork));
    }
    assert_eq!(costs[0], costs[1]);
    let mut baseline = None;
    for names in [["keep", "東京", "other"], ["other", "東京", "keep"]] {
        let extra: HashSet<String> = names.map(Into::into).into();
        let mut b = source.clone();
        let control = budget(u64::MAX);
        retain_bindings(&mut b, &[], &extra, work(&control)).unwrap();
        let result = (
            control.consumed(QueryCharge::CompilerWork),
            format!("{b:?}"),
        );
        if let Some(expected) = &baseline {
            assert_eq!(&result, expected);
        }
        baseline = Some(result);
    }
    let mut roomy = HashSet::with_capacity(100);
    roomy.insert("keep".into());
    let compact = HashSet::from(["keep".into()]);
    let mut costs = Vec::new();
    for extra in [&compact, &roomy] {
        let mut b = source.clone();
        let control = budget(u64::MAX);
        retain_bindings(&mut b, &[], extra, work(&control)).unwrap();
        assert_eq!(b.keys().map(String::as_str).collect::<Vec<_>>(), ["keep"]);
        costs.push(control.consumed(QueryCharge::CompilerWork));
    }
    assert_eq!(
        costs[1] - costs[0],
        4 * (roomy.capacity() - compact.capacity()) as u64
    );
}

#[test]
fn projection_work_empty_all_removed_and_extra_only_component_match_raw() {
    for source in [BTreeMap::new(), bindings()] {
        for project in [vec![], vec!["東京".into(), "東京".into()]] {
            for extra in [HashSet::new(), HashSet::from(["keep".into()])] {
                let mut raw = source.clone();
                retain_bindings(
                    &mut raw,
                    &project,
                    &extra,
                    BuildWork::new(CompilerWorkMode::Uncontrolled),
                )
                .unwrap();
                let run = |control: &dyn QueryControl| {
                    let mut b = source.clone();
                    retain_bindings(&mut b, &project, &extra, work(control))?;
                    Ok(b)
                };
                exact_and_short(raw, run);
                every_stop(run);
            }
        }
    }
}

//! Exact Query b676cf4f compiler compatibility, not issued serving admission.
//! Unverified mapping/SQLite fixtures exercise full queries, never golden data.

mod generated_successor_support;
use generated_successor_support::*;
use sf_core::query_control::UncontrolledQueryControl;
use sf_core::Term;
use sf_sparql::cache::generated::{ConstantCoverageError, ConstantOccurrence};
use sf_sparql::{exec, CompilerBinding, Plan};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const FREE: &UncontrolledQueryControl = &UncontrolledQueryControl;

fn compile(binding: &CompilerBinding, query: &str, cached: bool) -> Arc<Plan> {
    let check = |_: ConstantOccurrence<'_>| Ok::<_, ConstantCoverageError>(());
    if cached {
        binding
            .compile_shared_with_generated_admission(query, FREE, check)
            .unwrap()
    } else {
        binding
            .compile_uncached_shared_with_generated_admission(query, FREE, check)
            .unwrap()
    }
}

fn rows(conn: &rusqlite::Connection, plan: &Plan) -> Vec<Vec<Option<Term>>> {
    exec::select(plan, conn).unwrap().rows
}

#[test]
fn exact_template_bytes_are_the_consumer_contract() {
    assert_eq!(
        format!("{:x}", Sha256::digest(ENUMERATE.as_bytes())),
        "3459cb4a268ffc242de3c3afcc032f8001792958537b1303901c76a75ae2e2ec"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(ROOT.as_bytes())),
        "7fb994ca6015966d8be3eb823e61df6080926b0cbcd991b118e86f6f7a74b2a3"
    );
}

#[test]
fn full_enumeration_preserves_count_order_cursor_and_65_row_lookahead() {
    let (binding, conn) = fixture();
    for (cursor, range) in [("", 1..=65), ("STYLE-064", 65..=70), ("STYLE-070", 71..=70)] {
        let query = render(ENUMERATE, "{{AFTER_STYLE_NUMBER}}", cursor, true);
        let expected: Vec<_> = range
            .map(|n| {
                vec![
                    Some(string(&format!("STYLE-{n:03}"))),
                    Some(typed("70", "integer")),
                ]
            })
            .collect();
        let cold = compile(&binding, &query, true);
        let warm = compile(&binding, &query, true);
        let uncached = compile(&binding, &query, false);
        assert!(Arc::ptr_eq(&cold, &warm));
        for plan in [cold, warm, uncached] {
            assert_eq!(rows(&conn, &plan), expected, "cursor {cursor}");
        }
    }
}

#[test]
fn full_root_template_returns_all_ten_classes_without_other_style_leakage() {
    let (binding, conn) = fixture();
    for style in ["STYLE-001", "STYLE-002", "STYLE-099"] {
        let query = render(ROOT, "{{STYLE_NUMBER}}", style, false);
        let mut expected = expected_root(style);
        expected.sort_by_key(|row| row[0].as_ref().unwrap().to_string());
        for cached in [true, true, false] {
            let plan = compile(&binding, &query, cached);
            let sf_sparql::PlanForm::Select { vars } = &plan.form else {
                panic!("root contract must select");
            };
            assert_eq!(
                vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(),
                ROOT_VARS
            );
            let mut actual = rows(&conn, &plan);
            actual.sort_by_key(|row| row[0].as_ref().unwrap().to_string());
            assert_eq!(
                actual, expected,
                "exact RDF terms and unbound positions for {style}"
            );
        }
    }
}

#[test]
fn changed_constant_verdict_refuses_before_cold_or_warm_execution() {
    use sf_sparql::cache::generated::{GeneratedCompileError, GeneratedQueryRefusal};
    for template in [ENUMERATE, ROOT] {
        let query = if template == ROOT {
            render(template, "{{STYLE_NUMBER}}", "STYLE-001", false)
        } else {
            render(template, "{{AFTER_STYLE_NUMBER}}", "", true)
        };
        let (binding, _conn) = fixture();
        for warm in [false, true] {
            let before = warm.then(|| compile(&binding, &query, true));
            let result = binding.compile_shared_with_generated_admission(&query, FREE, |_| {
                Err(ConstantCoverageError::Uncovered)
            });
            assert!(matches!(
                result,
                Err(GeneratedCompileError::Refused(
                    GeneratedQueryRefusal::CoverageRefused
                ))
            ));
            if let Some(before) = before {
                assert!(Arc::ptr_eq(&before, &compile(&binding, &query, true)));
            }
        }
    }
}

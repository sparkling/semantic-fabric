//! Ordinary startup of two isolated file-backed SQLite sources with reload
//! disabled: sealed generations, request-scoped 503 refusal, unchanged
//! readiness and explicit-rebuild recovery for every WAL/DELETE pair.
//!
//! Answers are asked only of admitted federated shapes: two source-affine
//! UNION arms over distinct mapped predicates, or the bounded two-pattern
//! join. Single patterns and arms owned by zero, both or the same source stay
//! `501`. Positive qualification runs at the finite product defaults. The
//! ladder test varies only the retained-byte ceiling to prove that one limit
//! caused the failures seen at the first fixture's 4096.

use axum::http::StatusCode;
use sf_core::SourceId;

use super::ordinary_federation_fixture::*;
use crate::generation::GenerationRequirement;

type Case = (&'static str, String, &'static [&'static str], Vec<String>);

/// The query that answered `501` in the first candidate.
const ORIGINAL: &str = "SELECT ?s ?o WHERE { ?s <http://example.test/label> ?o }";

const UNSUPPORTED: (StatusCode, &str) = (StatusCode::NOT_IMPLEMENTED, "unsupported-query");
const UNAVAILABLE: (StatusCode, &str) = (StatusCode::SERVICE_UNAVAILABLE, "source-unavailable");

const LABEL_VARS: &[&str] = &["s", "o"];
const BAG_VARS: &[&str] = &["o"];
const AFFINE_VARS: &[&str] = &["s", "l", "r"];
const JOIN_VARS: &[&str] = &["left", "right"];

/// Shapes outside both admitted profiles: typed `501`, never a bogus `200`.
fn unsupported_shapes() -> Vec<(&'static str, String)> {
    let one = |predicate: &str| select("?s ?o", &triple("?s", predicate, "?o"));
    vec![
        ("original single shared pattern", ORIGINAL.to_owned()),
        ("single left key", one("left")),
        ("single right key", one("right")),
        ("single left label", one("leftLabel")),
        ("single right label", one("rightLabel")),
        (
            "both arms on the left source",
            same_var("leftLabel", "left"),
        ),
        ("arm owned by both sources", same_var("label", "rightLabel")),
        ("arm owned by no source", same_var("absent", "rightLabel")),
    ]
}

/// Every admitted federated shape with its exact expected bag.
fn cases(left: &[Row], right: &[Row]) -> Vec<Case> {
    let keys = affine_expected(left, right, false);
    let names = affine_expected(left, right, true);
    vec![
        (
            "UNION",
            labels("?s ?o"),
            LABEL_VARS,
            union_expected(left, right),
        ),
        (
            "projected bag",
            labels("?o"),
            BAG_VARS,
            labels_expected(left, right),
        ),
        ("key affinity", affine("left", "right"), AFFINE_VARS, keys),
        (
            "label affinity",
            affine("leftLabel", "rightLabel"),
            AFFINE_VARS,
            names,
        ),
        (
            "bounded join",
            join_query(),
            JOIN_VARS,
            join_expected(left, right),
        ),
    ]
}

fn admitted() -> Vec<(&'static str, String)> {
    let all = cases(&[], &[]);
    all.into_iter()
        .map(|(label, query, _, _)| (label, query))
        .collect()
}

fn ids() -> [SourceId; 2] {
    [SourceId::new(LEFT).unwrap(), SourceId::new(RIGHT).unwrap()]
}

/// Mismatches of one phase, reported together with any diagnostic notes.
struct Report {
    phase: String,
    notes: Vec<String>,
    failures: Vec<String>,
}

impl Report {
    fn new(phase: String) -> Self {
        Self {
            phase,
            notes: Vec::new(),
            failures: Vec::new(),
        }
    }

    fn check(&mut self, ok: bool, failure: String) {
        if !ok {
            self.failures.push(failure);
        }
    }

    fn note(&mut self, note: String) {
        self.notes.push(note);
    }

    fn finish(self) {
        let notes = self.notes.join("\n");
        let failures = self.failures.join("\n");
        assert!(
            self.failures.is_empty(),
            "{}:\n{notes}\n{failures}",
            self.phase
        );
        if !notes.is_empty() {
            eprintln!("{}:\n{notes}", self.phase);
        }
    }
}

/// `Ok` only for HTTP 200 with exactly the expected answer bag.
async fn verdict(config: &Config, case: &Case) -> Result<(), String> {
    let (name, query, vars, want) = case;
    let (status, body) = ask(config, query).await;
    let got = bag(&body, vars);
    if status == StatusCode::OK && got.as_ref() == Some(want) {
        return Ok(());
    }
    Err(format!(
        "{name}: {status} {got:?} != {want:?}\n  {body}\n  {query}"
    ))
}

async fn check_answers(config: &Config, report: &mut Report, left: &[Row], right: &[Row]) {
    for case in cases(left, right) {
        if let Err(failure) = verdict(config, &case).await {
            report.failures.push(failure);
        }
    }
}

/// Every query must be the typed problem with no valid-looking answer, not
/// even a partial one.
async fn check_problems(
    config: &Config,
    report: &mut Report,
    queries: Vec<(&'static str, String)>,
    want: (StatusCode, &str),
) {
    for (label, query) in queries {
        let (status, body) = ask(config, &query).await;
        let problem = problem_code(&body);
        let typed = (status, problem.as_str()) == want;
        let ok = typed && !body.contains("bindings") && !body.contains(ITEM);
        let failure = format!("{label}: {status} {problem}: {body}\n  {query}");
        report.check(ok, failure);
    }
}

/// Which slots carry a sealed SQLite generation in the ordinary runtime.
fn sealed(config: &Config) -> Result<Vec<(SourceId, bool)>, String> {
    let lease = config
        .runtime_lease()
        .map_err(|error| format!("lease: {error:?}"))?;
    let required = lease
        .generation_requirements(ids())
        .map_err(|error| format!("requirements: {error:?}"))?;
    let mut slots = Vec::new();
    for req in &required {
        let sqlite = matches!(req, GenerationRequirement::Sqlite { .. });
        slots.push((req.source_id(), sqlite));
    }
    slots.sort_by_key(|(id, _)| *id);
    Ok(slots)
}

/// Both slots must be sealed; a failure names every slot that is not.
fn check_sealed(config: &Config, report: &mut Report) {
    let [left, right] = ids();
    let want: Result<Vec<(SourceId, bool)>, String> = Ok(vec![(left, true), (right, true)]);
    let got = sealed(config);
    let failure = format!("sealed: {got:?} != {want:?}");
    report.check(got == want, failure);
}

/// Readiness value unchanged, actually `Ready`, and `/readyz` answers ready.
async fn check_ready(
    config: &Config,
    report: &mut Report,
    ready: crate::activation::RuntimeReadiness,
) {
    let after = config.runtime_readiness().unwrap();
    let is_ready = matches!(after, crate::activation::RuntimeReadiness::Ready { .. });
    let failure = format!("readiness {ready:?} -> {after:?}");
    report.check(after == ready && is_ready, failure);
    let (status, body) = readyz(config).await;
    let ok = status == StatusCode::OK && body == r#"{"status":"ready"}"#;
    report.check(ok, format!("readyz: {status} {body}"));
}

async fn scenario(journals: [&str; 2], drifted: usize) {
    let side = ["left", "right"][drifted];
    let name = format!("{}/{} {side} drift", journals[LEFT], journals[RIGHT]);
    let pairs = [(NEW_LEFT, INITIAL_RIGHT), (INITIAL_LEFT, NEW_RIGHT)];
    let (left, right) = pairs[drifted];
    let stale = cases(INITIAL_LEFT, INITIAL_RIGHT);
    let now = cases(left, right);
    for ((label, _, _, before), (_, _, _, after)) in stale.iter().zip(&now) {
        assert_ne!(before, after, "{name}: drift must change {label}");
    }
    let all_labels = labels_expected(INITIAL_LEFT, INITIAL_RIGHT);
    let shared = all_labels.iter().filter(|l| *l == "shared").count();
    assert_eq!(shared, 3, "within- and cross-source duplicates");

    let fixture = Fixture::new(journals);
    assert!(!fixture.opts.require_verified_generation);
    assert!(fixture.opts.reload_interval.is_zero());

    let old = build(&fixture.opts).await;
    let ready = old.runtime_readiness().unwrap();
    let mut report = Report::new(format!("{name}: initial"));
    check_sealed(&old, &mut report);
    check_answers(&old, &mut report, INITIAL_LEFT, INITIAL_RIGHT).await;
    check_problems(&old, &mut report, unsupported_shapes(), UNSUPPORTED).await;
    check_ready(&old, &mut report, ready).await;
    report.finish();

    fixture.drift(drifted, [left, right][drifted]);
    let mut report = Report::new(format!("{name}: after drift"));
    check_problems(&old, &mut report, admitted(), UNAVAILABLE).await;
    check_sealed(&old, &mut report);
    check_ready(&old, &mut report, ready).await;
    report.finish();

    let fresh = build(&fixture.opts).await;
    let fresh_ready = fresh.runtime_readiness().unwrap();
    let mut report = Report::new(format!("{name}: rebuilt"));
    check_sealed(&fresh, &mut report);
    check_answers(&fresh, &mut report, left, right).await;
    check_problems(&fresh, &mut report, unsupported_shapes(), UNSUPPORTED).await;
    check_problems(&old, &mut report, admitted(), UNAVAILABLE).await;
    check_ready(&old, &mut report, ready).await;
    check_ready(&fresh, &mut report, fresh_ready).await;
    report.finish();
}

#[tokio::test]
async fn wal_delete_left_drift() {
    scenario(["WAL", "DELETE"], LEFT).await;
}

#[tokio::test]
async fn wal_delete_right_drift() {
    scenario(["WAL", "DELETE"], RIGHT).await;
}

#[tokio::test]
async fn delete_wal_left_drift() {
    scenario(["DELETE", "WAL"], LEFT).await;
}

#[tokio::test]
async fn delete_wal_right_drift() {
    scenario(["DELETE", "WAL"], RIGHT).await;
}

#[tokio::test]
async fn wal_wal_left_drift() {
    scenario(["WAL", "WAL"], LEFT).await;
}

#[tokio::test]
async fn wal_wal_right_drift() {
    scenario(["WAL", "WAL"], RIGHT).await;
}

#[tokio::test]
async fn delete_delete_left_drift() {
    scenario(["DELETE", "DELETE"], LEFT).await;
}

#[tokio::test]
async fn delete_delete_right_drift() {
    scenario(["DELETE", "DELETE"], RIGHT).await;
}

/// Proves the retained-byte ceiling alone caused the failures at 4096: the
/// finite product defaults answer every shape exactly; `retained(4096)` alone
/// fails a nonempty set of shapes; the full old-fixture limit set fails the
/// identical set; and a bisection finds each failing shape's minimal ceiling
/// strictly below the product default.
#[tokio::test]
async fn retained_bytes_alone_explain_the_failures() {
    let fixture = Fixture::new(["WAL", "DELETE"]);
    let mut report = Report::new("retained-byte ladder".to_owned());
    let all = cases(INITIAL_LEFT, INITIAL_RIGHT);

    let config = build(&fixture.opts).await;
    let ready = config.runtime_readiness().unwrap();
    check_sealed(&config, &mut report);
    for _ in 0..3 {
        check_answers(&config, &mut report, INITIAL_LEFT, INITIAL_RIGHT).await;
    }
    check_ready(&config, &mut report, ready).await;

    let small = build_tuned(&fixture.opts, retained(SMALL_RETAINED)).await;
    let old = build_tuned(&fixture.opts, old_fixture_limits).await;
    let small_ready = small.runtime_readiness().unwrap();
    let mut failing = Vec::new();
    for (index, case) in all.iter().enumerate() {
        if verdict(&small, case).await.is_err() {
            failing.push(index);
        }
    }
    let names: Vec<&str> = failing.iter().map(|&index| all[index].0).collect();
    report.note(format!("retained {SMALL_RETAINED} fails: {names:?}"));
    report.check(
        !failing.is_empty(),
        "no case fails at the small ceiling".to_owned(),
    );
    for (index, case) in all.iter().enumerate() {
        let failed = verdict(&old, case).await.is_err();
        let expected = failing.contains(&index);
        let message = format!("old fixture: {} failed={failed}, want {expected}", case.0);
        report.check(failed == expected, message);
    }
    check_ready(&small, &mut report, small_ready).await;

    let default = crate::DEFAULT_QUERY_LIMITS.max_retained_bytes();
    for index in failing {
        let case = &all[index];
        let (mut fail, mut pass) = (SMALL_RETAINED, default);
        while pass - fail > 1 {
            let mid = fail + (pass - fail) / 2;
            let config = build_tuned(&fixture.opts, retained(mid)).await;
            if verdict(&config, case).await.is_ok() {
                pass = mid;
            } else {
                fail = mid;
            }
        }
        report.note(format!(
            "{}: fails at {fail} bytes, passes at {pass}",
            case.0
        ));
        let failure = format!("{}: minimal ceiling {pass} not below {default}", case.0);
        report.check(pass < default, failure);
    }

    let recovered = build(&fixture.opts).await;
    let recovered_ready = recovered.runtime_readiness().unwrap();
    check_sealed(&recovered, &mut report);
    check_answers(&recovered, &mut report, INITIAL_LEFT, INITIAL_RIGHT).await;
    check_ready(&recovered, &mut report, recovered_ready).await;

    // `PRAGMA journal_mode=TRUNCATE` does not persist: readers open in DELETE
    // mode, so both slots are sealed and answer exactly like the pairs above.
    let truncate = Fixture::new(["TRUNCATE", "TRUNCATE"]);
    let config = build(&truncate.opts).await;
    check_sealed(&config, &mut report);
    check_answers(&config, &mut report, INITIAL_LEFT, INITIAL_RIGHT).await;
    report.finish();
}

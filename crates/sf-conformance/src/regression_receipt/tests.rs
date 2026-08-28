mod invariants;

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::Path;

use super::format;
use super::model::{
    BaselineRow, Disposition, ExpectedBaseline, InputBinding, Profile, Suite, Surface, TestIdentity,
};
use super::runner::{self, ObservedOutcome, ObservedTest, RunFailure, SuiteRunner};
use super::{parse_mode, Mode};

#[derive(Default)]
struct FakeRunner {
    discovered: Vec<String>,
    observed: Vec<ObservedTest>,
    executed_exclusions: RefCell<Vec<Vec<String>>>,
}

impl SuiteRunner for FakeRunner {
    fn discover(&self, _root: &Path, _suite: &Suite) -> Result<Vec<String>, RunFailure> {
        Ok(self.discovered.clone())
    }

    fn execute(
        &self,
        _root: &Path,
        _suite: &Suite,
        excluded: &[String],
    ) -> Result<Vec<ObservedTest>, RunFailure> {
        self.executed_exclusions
            .borrow_mut()
            .push(excluded.to_vec());
        Ok(self.observed.clone())
    }
}

#[test]
fn canonical_inventory_and_baseline_round_trip() {
    let profile = profile_with_tests(standard_tests());
    let profile_text = format::render_profile(&profile);
    let baseline = baseline_for(&profile);
    let baseline_text = format::render_baseline(&baseline);

    assert_eq!(
        (
            format::parse_profile(&profile_text),
            format::parse_baseline(&baseline_text),
        ),
        (Ok(profile), Ok(baseline))
    );
}

#[test]
fn inventory_parser_rejects_oversized_input() {
    let oversized = format!("{}\n", "x".repeat(format::MAX_INVENTORY_BYTES as usize));

    assert_eq!(
        format::parse_profile(&oversized).unwrap_err(),
        format!(
            "profile inventory exceeds {} bytes",
            format::MAX_INVENTORY_BYTES
        )
    );
}

#[test]
fn inventory_parser_rejects_more_than_four_hundred_tests() {
    let profile = profile_with_tests(
        (0..401)
            .map(|index| required_test(&format!("case-{index:03}")))
            .collect(),
    );

    assert_eq!(
        format::parse_profile(&format::render_profile(&profile)).unwrap_err(),
        "line 409: too many test identities"
    );
}

#[test]
fn baseline_parser_rejects_non_passed_expected_outcome() {
    let baseline = baseline_for(&profile_with_tests(standard_tests()));
    let malformed = format::render_baseline(&baseline).replace("\tpassed\n", "\tfailed\n");

    assert!(format::parse_baseline(&malformed)
        .unwrap_err()
        .contains("baseline outcome must be exactly passed"));
}

#[test]
fn baseline_parser_rejects_per_identity_digest_mutation() {
    let baseline = baseline_for(&profile_with_tests(standard_tests()));
    let malformed = format::render_baseline(&baseline).replace("required_case", "replacement_case");

    assert_eq!(
        format::parse_baseline(&malformed).unwrap_err(),
        "baseline outcomes digest mismatch"
    );
}

#[test]
fn libtest_parser_captures_each_observed_outcome() {
    let output = "\nrunning 3 tests\n\
test alpha ... ok\n\
test beta ... FAILED\n\
test gamma ... ignored\n\n\
test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 2 filtered out; finished in 0.01s\n";

    let (parsed, summary) = runner::parse_test_run(output).unwrap();

    assert_eq!(
        (parsed, summary.passed, summary.failed, summary.ignored),
        (
            vec![
                observed("alpha", ObservedOutcome::Passed),
                observed("beta", ObservedOutcome::Failed),
                observed("gamma", ObservedOutcome::Ignored),
            ],
            1,
            1,
            1
        )
    );
}

#[test]
fn injected_runner_captures_one_row_per_required_test() {
    let profile = profile_with_tests(standard_tests());
    let runner = passing_runner();

    let rows = runner::execute_profile(&profile, Path::new("."), &runner).unwrap();

    assert_eq!(
        (rows, runner.executed_exclusions.into_inner()),
        (
            vec![BaselineRow {
                suite_id: "suite".to_owned(),
                test_name: "required_case".to_owned(),
            }],
            vec![vec!["resource_case".to_owned()]]
        )
    );
}

#[test]
fn injected_runner_rejects_discovered_identity_drift() {
    let profile = profile_with_tests(standard_tests());
    let runner = FakeRunner {
        discovered: vec!["added_case".to_owned(), "required_case".to_owned()],
        observed: vec![observed("required_case", ObservedOutcome::Passed)],
        ..FakeRunner::default()
    };

    let failure = runner::execute_profile(&profile, Path::new("."), &runner).unwrap_err();

    assert_eq!(
        failure.message,
        "suite suite test identity drift: missing=[\"resource_case\"], added=[\"added_case\"]"
    );
}

#[test]
fn injected_runner_rejects_failed_required_test() {
    let profile = profile_with_tests(standard_tests());
    let mut runner = passing_runner();
    runner.observed[0].outcome = ObservedOutcome::Failed;

    let failure = runner::execute_profile(&profile, Path::new("."), &runner).unwrap_err();

    assert_eq!(
        failure.message,
        "suite suite required test required_case observed failed"
    );
}

#[test]
fn injected_runner_rejects_ignored_required_test() {
    let profile = profile_with_tests(standard_tests());
    let mut runner = passing_runner();
    runner.observed[0].outcome = ObservedOutcome::Ignored;

    let failure = runner::execute_profile(&profile, Path::new("."), &runner).unwrap_err();

    assert_eq!(
        failure.message,
        "suite suite required test required_case observed ignored"
    );
}

#[test]
fn injected_runner_rejects_non_isolated_exclusion() {
    let profile = profile_with_tests(vec![
        required_test("alpha::resource_case"),
        excluded_test("resource_case"),
    ]);
    let runner = FakeRunner {
        discovered: vec![
            "alpha::resource_case".to_owned(),
            "resource_case".to_owned(),
        ],
        observed: vec![observed("alpha::resource_case", ObservedOutcome::Passed)],
        ..FakeRunner::default()
    };

    let failure = runner::execute_profile(&profile, Path::new("."), &runner).unwrap_err();

    assert_eq!(
        (failure.message, runner.executed_exclusions.borrow().len()),
        (
            "suite suite exclusion \"resource_case\" also filters a required test".to_owned(),
            0
        )
    );
}

#[test]
fn help_cannot_be_combined_with_execution_mode() {
    let error = parse_mode([OsString::from("--help"), OsString::from("--check")]).unwrap_err();

    assert_eq!(
        error,
        "--help cannot be combined with --check or --generate"
    );
}

#[test]
fn denied_build_and_provider_environment_is_exact() {
    assert_eq!(
        super::environment::DENIED_EXACT,
        &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "AZURE_OPENAI_API_KEY",
            "CARGO",
            "CARGO_BUILD_RUSTFLAGS",
            "CARGO_BUILD_TARGET",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
            "CARGO_TARGET_DIR",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "DATABASE_URL",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "MSSQL_SA_PASSWORD",
            "MYSQL_DATABASE",
            "MYSQL_HOST",
            "MYSQL_PASSWORD",
            "MYSQL_PWD",
            "MYSQL_ROOT_PASSWORD",
            "MYSQL_URL",
            "MYSQL_USER",
            "OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "PGDATABASE",
            "PGHOST",
            "PGPASSWORD",
            "PGPORT",
            "PGUSER",
            "POSTGRES_DB",
            "POSTGRES_PASSWORD",
            "POSTGRES_USER",
            "REQUESTY_API_KEY",
            "RUSTC",
            "RUSTC_BOOTSTRAP",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
            "RUSTDOC",
            "RUSTDOCFLAGS",
            "RUSTFLAGS",
            "RUST_LOG",
            "RUST_MIN_STACK",
            "RUST_TEST_NOCAPTURE",
            "SF_MSSQL_URL",
            "SF_MYSQL_URL",
            "SF_PG_URL",
        ]
    );
    assert_eq!(super::environment::DENIED_PREFIXES, &["CARGO_PROFILE_"]);
}

fn passing_runner() -> FakeRunner {
    FakeRunner {
        discovered: vec!["required_case".to_owned(), "resource_case".to_owned()],
        observed: vec![observed("required_case", ObservedOutcome::Passed)],
        ..FakeRunner::default()
    }
}

fn observed(name: &str, outcome: ObservedOutcome) -> ObservedTest {
    ObservedTest {
        name: name.to_owned(),
        outcome,
    }
}

fn profile_with_tests(tests: Vec<TestIdentity>) -> Profile {
    Profile {
        profile_id: "test-profile".to_owned(),
        surface: Surface::SparqlQuery,
        backend: "sqlite".to_owned(),
        inputs: vec![InputBinding {
            path: "input.txt".to_owned(),
            byte_length: 6,
            sha256: format::sha256(b"source"),
        }],
        suites: vec![Suite {
            id: "suite".to_owned(),
            package: "package".to_owned(),
            target: "target".to_owned(),
            timeout_seconds: 10,
        }],
        tests,
    }
}

fn standard_tests() -> Vec<TestIdentity> {
    vec![
        required_test("required_case"),
        excluded_test("resource_case"),
    ]
}

fn required_test(name: &str) -> TestIdentity {
    TestIdentity {
        suite_id: "suite".to_owned(),
        name: name.to_owned(),
        disposition: Disposition::Include,
        reason: "required".to_owned(),
    }
}

fn excluded_test(name: &str) -> TestIdentity {
    TestIdentity {
        suite_id: "suite".to_owned(),
        name: name.to_owned(),
        disposition: Disposition::Exclude,
        reason: "timing-performance".to_owned(),
    }
}

fn baseline_for(profile: &Profile) -> ExpectedBaseline {
    let rows = vec![BaselineRow {
        suite_id: "suite".to_owned(),
        test_name: "required_case".to_owned(),
    }];
    ExpectedBaseline {
        profile_id: profile.profile_id.clone(),
        surface: profile.surface,
        backend: profile.backend.clone(),
        inventory_path: "tests/inventory.tsv".to_owned(),
        inventory_sha256: format::sha256(b"inventory\n"),
        outcomes_sha256: format::outcomes_digest(&rows),
        suite_count: 1,
        excluded_count: 1,
        rows,
    }
}

#[test]
fn parse_mode_requires_exactly_one_mode() {
    assert_eq!(
        (
            parse_mode([OsString::from("--check")]),
            parse_mode([OsString::from("--generate")]),
        ),
        (Ok(Some(Mode::Check)), Ok(Some(Mode::Generate)))
    );
}

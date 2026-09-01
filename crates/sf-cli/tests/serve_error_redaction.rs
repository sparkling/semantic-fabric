//! Black-box CLI receipts for the source-reference and secret boundary.

use std::process::{Command, Output};

use sf_serve::MAX_SOURCE_INPUT_BYTES;

const SECRET: &str = "sf_secret_NEVER_EXPOSE_c913";

fn missing_mapping() -> String {
    std::env::temp_dir()
        .join(format!(
            "sf_cli_missing_mapping_{}_{}.ttl",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ))
        .to_string_lossy()
        .into_owned()
}

fn serve_command(mapping: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"));
    command.args(["serve", "--mapping", mapping]);
    command
}

fn run(mut command: Command) -> Output {
    command.output().expect("run semantic-fabric")
}

fn assert_absent(output: &Output, needle: &str) {
    for (name, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        assert!(
            !bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes()),
            "sensitive value escaped byte-for-byte on {name}: {}",
            String::from_utf8_lossy(bytes)
        );
    }
}

fn assert_opaque_source_failure(output: Output, secret: Option<&str>) {
    assert!(!output.status.success(), "source boundary must fail");
    assert_eq!(output.status.code(), Some(1));
    if let Some(secret) = secret {
        assert_absent(&output, secret);
    }
    assert!(output.stdout.len() < 512, "unbounded stdout surface");
    assert!(output.stderr.len() < 512, "unbounded stderr surface");
    let stderr = String::from_utf8(output.stderr).expect("stderr UTF-8");
    assert!(stderr.contains("startup-source"), "stderr={stderr:?}");
    assert!(stderr.contains("correlation sf-"), "stderr={stderr:?}");
}

#[test]
fn source_and_source_env_are_required_and_mutually_exclusive() {
    let mapping = missing_mapping();
    let missing = run(serve_command(&mapping));
    assert_eq!(missing.status.code(), Some(2));
    let stderr = String::from_utf8(missing.stderr).expect("clap stderr UTF-8");
    assert!(stderr.contains("--source"), "stderr={stderr:?}");
    assert!(stderr.contains("--source-env"), "stderr={stderr:?}");

    let mut both = serve_command(&mapping);
    both.args([
        "--source",
        "sqlite::memory:",
        "--source-env",
        "SF_SOURCE_REF_BOTH",
    ]);
    let both = run(both);
    assert_eq!(both.status.code(), Some(2));
    let stderr = String::from_utf8(both.stderr).expect("clap stderr UTF-8");
    assert!(stderr.contains("cannot be used with"), "stderr={stderr:?}");
}

#[test]
fn invalid_missing_empty_and_oversized_environment_values_fail_at_startup() {
    let mapping = missing_mapping();

    let mut invalid_name = serve_command(&mapping);
    invalid_name.args(["--source-env", "SOURCE-NAME"]);
    assert_opaque_source_failure(run(invalid_name), None);

    let missing_name = "SF_SOURCE_REF_TEST_MISSING_71D2";
    let mut missing = serve_command(&mapping);
    missing
        .args(["--source-env", missing_name])
        .env_remove(missing_name);
    assert_opaque_source_failure(run(missing), None);

    let empty_name = "SF_SOURCE_REF_TEST_EMPTY_71D2";
    let mut empty = serve_command(&mapping);
    empty.args(["--source-env", empty_name]).env(empty_name, "");
    assert_opaque_source_failure(run(empty), None);

    let oversized_name = "SF_SOURCE_REF_TEST_OVERSIZED_71D2";
    let mut oversized = serve_command(&mapping);
    oversized
        .args(["--source-env", oversized_name])
        .env(oversized_name, "x".repeat(MAX_SOURCE_INPUT_BYTES + 1));
    assert_opaque_source_failure(run(oversized), None);
}

#[cfg(unix)]
#[test]
fn non_utf8_environment_value_fails_at_startup() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mapping = missing_mapping();
    let name = "SF_SOURCE_REF_TEST_NON_UTF8_71D2";
    let mut command = serve_command(&mapping);
    command
        .args(["--source-env", name])
        .env(name, OsString::from_vec(vec![b'p', b'g', b':', 0xff]));
    assert_opaque_source_failure(run(command), None);
}

#[test]
fn inline_pg_and_mysql_credentials_are_rejected_before_mapping_io() {
    let mapping = missing_mapping();
    let sources = [
        format!("pg:host=database.invalid user=test password={SECRET}"),
        format!("mysql://user:{SECRET}@database.invalid/db"),
    ];

    for source in sources {
        let mut command = serve_command(&mapping);
        command.args(["--source", &source]);
        assert_opaque_source_failure(run(command), Some(SECRET));
    }
}

#[test]
fn secret_free_inline_source_families_reach_the_mapping_boundary() {
    let mapping = missing_mapping();
    for source in [
        "sqlite::memory:",
        "pg:host=database.invalid user=test",
        "mysql://user@database.invalid/db",
    ] {
        let mut command = serve_command(&mapping);
        command.args(["--source", source]);
        let output = run(command);
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).expect("stderr UTF-8");
        assert!(
            stderr.contains("startup-configuration"),
            "source {source:?} did not pass preparation: {stderr:?}"
        );
        assert!(!stderr.contains("startup-source"), "stderr={stderr:?}");
    }
}

#[test]
fn environment_injection_admits_credentials_but_never_prints_them() {
    let mapping = missing_mapping();
    let name = "SF_SOURCE_REF_TEST_CREDENTIAL_71D2";
    for source in [
        format!("pg:host=database.invalid user=test password={SECRET}"),
        format!("mysql://user:{SECRET}@database.invalid/db"),
    ] {
        let mut command = serve_command(&mapping);
        command
            .args(["--source-env", name])
            .env(name, source.as_str());
        let output = run(command);
        assert_absent(&output, SECRET);
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).expect("stderr UTF-8");
        assert!(
            stderr.contains("startup-configuration"),
            "stderr={stderr:?}"
        );
    }
}

#[test]
fn malformed_environment_sources_keep_public_errors_opaque() {
    let mapping = missing_mapping();
    let name = "SF_SOURCE_REF_TEST_MALFORMED_71D2";
    for source in [
        format!("pg:host=localhost port=not-a-port password={SECRET}"),
        format!("mysql://user:{SECRET}@localhost:not-a-port/db"),
    ] {
        let mut command = serve_command(&mapping);
        command
            .args(["--source-env", name])
            .env(name, source.as_str());
        assert_opaque_source_failure(run(command), Some(SECRET));
    }
}

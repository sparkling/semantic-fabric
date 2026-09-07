//! Public fail-before-file/network boundary for unsafe source TLS and trust input.
use std::process::Command;

#[test]
fn source_tls_failures_are_opaque_and_precede_mapping_io() {
    for source in [
        "pg:host=db.example sslmode=disable",
        "mysql://db.example/db?require_ssl=true&verify_ca=false",
        "mysql://db.example/db?require_ssl=true&verify_identity=false",
        "mysql://db.example/db?socket=%2Ftmp%2Fmysql.sock",
    ] {
        let output = command().args(["--source", source]).output().unwrap();
        assert_failure(output);
    }
    for value in ["private-malformed-root".to_owned(), "x".repeat(65537)] {
        let output = command()
            .args([
                "--source",
                "pg:host=db.example",
                "--source-tls-roots-env",
                "SF_TEST_ROOTS",
            ])
            .env("SF_TEST_ROOTS", value)
            .output()
            .unwrap();
        assert_failure(output);
    }
    let output = command()
        .args([
            "--source",
            "pg:host=db.example",
            "--source-tls-roots-env",
            "SF_MISSING_ROOTS",
        ])
        .output()
        .unwrap();
    assert_failure(output);
}

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"));
    command.env_clear().args([
        "serve",
        "--mapping",
        "/nonexistent/tls-test-mapping",
        "--ontology",
        "/nonexistent/tls-test-ontology",
    ]);
    command
}

fn assert_failure(output: std::process::Output) {
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.len() < 512);
    let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(event["failure"], "startup-source");
    let text = String::from_utf8(output.stderr).unwrap();
    for forbidden in [
        "private-malformed-root",
        "SF_TEST_ROOTS",
        "SF_MISSING_ROOTS",
        "db.example",
    ] {
        assert!(!text.contains(forbidden));
    }
}

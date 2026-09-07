//! Non-serving commands retain their pre-telemetry CLI output surface.

#![cfg(feature = "development-tools")]

use std::process::Command;

#[test]
fn running_benchmark_is_stdout_only_and_contains_no_json_telemetry() {
    let output = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"))
        .arg("bench")
        .output()
        .expect("run real benchmark subcommand");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("benchmark output is UTF-8");
    assert!(
        stdout.contains("GTFS-Madrid OBDA benchmark"),
        "output={stdout}"
    );
    assert_eq!(
        stdout.matches("all queries + streaming CONSTRUCT").count(),
        2
    );
    assert!(!stdout.contains("\"event\":"), "output={stdout}");
    assert!(!stdout.contains("sf.compiler.stage"), "output={stdout}");
}

//! Non-serving commands retain their pre-telemetry CLI output surface.

use std::process::Command;

#[test]
fn non_serve_help_is_stdout_only_and_contains_no_json_telemetry() {
    for command in ["conformance", "bench"] {
        let output = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"))
            .args([command, "--help"])
            .output()
            .expect("run semantic-fabric help");
        assert!(output.status.success(), "command={command}");
        assert!(output.stderr.is_empty(), "command={command}");
        let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
        assert!(
            stdout.contains("Usage:"),
            "command={command}, output={stdout}"
        );
        assert!(!stdout.contains("\"event\":"), "command={command}");
        assert!(!stdout.contains("sf.compiler.stage"), "command={command}");
    }
}

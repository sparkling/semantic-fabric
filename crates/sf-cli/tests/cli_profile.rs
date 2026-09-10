//! The serving artifact keeps the public server, not development commands.

use std::process::{Command, Output};

fn invoke(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_semantic-fabric"))
        .args(args)
        .env_clear()
        .output()
        .expect("run CLI profile check")
}

#[test]
fn binary_version_matches_the_nonzero_workspace_version() {
    let version = env!("CARGO_PKG_VERSION");
    assert_ne!(version.split('-').next().unwrap(), "0.0.0");
    let output = invoke(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("semantic-fabric {version}")
    );
}

#[test]
fn serving_help_retains_required_database_and_security_options() {
    let output = invoke(&["serve", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for option in [
        "--source-env",
        "--source-env-2",
        "--mapping",
        "--ontology",
        "--auth-subjects-env",
        "--source-tls-roots-env",
        "--max-source-work",
        "--shutdown-timeout-secs",
        "--reload-interval-secs",
        "--require-verified-generation",
    ] {
        assert!(help.contains(option), "missing serving option: {option}");
    }
}

#[test]
fn developer_commands_match_the_compiled_profile() {
    let output = invoke(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for name in ["bench", "conformance"] {
        let enabled = cfg!(feature = "development-tools");
        assert_eq!(
            help.lines().any(|line| line.trim_start().starts_with(name)),
            enabled
        );
        let output = invoke(&[name, "--help"]);
        assert_eq!(output.status.success(), enabled, "command: {name}");
        if !enabled {
            assert_eq!(output.status.code(), Some(2));
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8(output.stderr)
                .unwrap()
                .contains("unrecognized subcommand"));
            // Reject actual execution too, before any developer work can run.
            assert_eq!(invoke(&[name]).status.code(), Some(2));
        }
    }
}

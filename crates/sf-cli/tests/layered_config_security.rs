//! Real process regressions for the configuration/authorization boundary.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BINARY: &str = env!("CARGO_BIN_EXE_semantic-fabric");

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("sf-config-security-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        Self { root }
    }
    fn config(&self, value: &str) -> PathBuf {
        let path = self.root.join("serve.toml");
        std::fs::write(&path, value).unwrap();
        path
    }
    fn command(&self) -> Command {
        let mut command = Command::new(BINARY);
        command
            .env_clear()
            .args(["serve", "--config"])
            .arg(self.root.join("serve.toml"));
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.root.join("serve.toml"));
        let _ = std::fs::remove_dir(&self.root);
    }
}

fn bounded_output(mut command: Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if timed_out {
        let _ = child.kill();
    }
    let output = child.wait_with_output().unwrap();
    assert!(!timed_out, "configuration process exceeded the test bound");
    output
}

#[test]
fn rotating_token_reference_still_resolves_row_policy_before_source_io() {
    let fixture = Fixture::new();
    fixture.config("[source]\nsource_env = 'MISSING_SOURCE'\n[mappings]\nmapping = 'missing.ttl'\n[graphs]\nontology = 'missing.ttl'\n[security]\nauth_token_env = 'OLD_TOKEN'\npg_rls_context_env = 'MISSING_ROW_POLICY'\n");
    for env_override in [false, true] {
        let mut command = fixture.command();
        command.env("NEW_TOKEN", "test-only-new-token-0123456789012345");
        if env_override {
            command.env("SEMANTIC_FABRIC_AUTH_TOKEN_ENV", "NEW_TOKEN");
        } else {
            command.args(["--auth-token-env", "NEW_TOKEN"]);
        }
        command.arg("--");
        let output = bounded_output(command);
        assert_eq!(output.status.code(), Some(1));
        let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(
            event["failure"], "startup-configuration",
            "policy must fail before the source boundary"
        );
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stderr.contains("MISSING_ROW_POLICY"));
        assert!(!stderr.contains("test-only-new-token"));
    }
}

#[test]
fn bad_layer_values_cannot_escape_in_clap_diagnostics() {
    let fixture = Fixture::new();
    for (section, key, value) in [
        ("observability", "log_level", "private-config-value"),
        (
            "observability",
            "log_level",
            &"private-config-value".repeat(1000),
        ),
    ] {
        fixture.config(&format!("[{section}]\n{key} = '{value}'\n"));
        let output = bounded_output(fixture.command());
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            output.stderr,
            b"semantic-fabric: startup configuration is invalid\n"
        );
        assert!(output.stdout.is_empty());
    }
    fixture.config("");
    for variable in ["SEMANTIC_FABRIC_TIMEOUT_SECS", "SEMANTIC_FABRIC_METRICS"] {
        let mut command = fixture.command();
        command.env(variable, "private-env-value");
        let output = bounded_output(command);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            output.stderr,
            b"semantic-fabric: startup configuration is invalid\n"
        );
    }
}

#[test]
fn help_is_available_with_a_broken_configuration() {
    let fixture = Fixture::new();
    let mut command = fixture.command();
    command
        .arg("--help")
        .env("SEMANTIC_FABRIC_TIMEOUT_SECS", "invalid");
    let output = bounded_output(command);
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("--config <CONFIG>"));
    assert!(output.stderr.is_empty());
}

#[test]
fn argument_terminator_cannot_skip_a_selected_configuration() {
    let fixture = Fixture::new();
    let mut command = fixture.command();
    command.args([
        "--source=sqlite::memory:",
        "--mapping=missing.ttl",
        "--ontology=missing.ttl",
        "--",
    ]);
    let output = bounded_output(command);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stderr,
        b"semantic-fabric: startup configuration cannot be read\n"
    );
}

#[cfg(unix)]
#[test]
fn nonregular_configuration_is_rejected_without_waiting_for_a_writer() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = Fixture::new();
    let path = fixture.root.join("serve.toml");
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: name points to an owned, NUL-terminated path in this private fixture.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let output = bounded_output(fixture.command());
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stderr,
        b"semantic-fabric: startup configuration cannot be read\n"
    );
    assert!(output.stdout.is_empty());
}

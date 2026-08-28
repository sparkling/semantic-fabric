#![cfg(target_os = "linux")]

use std::time::Duration;

use sf_bench::performance::subprocess::{canonical_program, BoundedCommand};

fn command(program: &str) -> BoundedCommand {
    BoundedCommand {
        program: canonical_program(std::path::Path::new(program)).unwrap(),
        args: Vec::new(),
        current_dir: std::env::current_dir().unwrap(),
        stdin: Vec::new(),
        timeout: Duration::from_secs(2),
        maximum_output: 4_096,
        observe_linux_identity: false,
    }
}

#[test]
fn should_strip_the_entire_inherited_environment() {
    let output = command("/usr/bin/env").run().unwrap();

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.identity, None);
}

#[test]
fn should_observe_linux_pid_and_start_time_when_requested() {
    let mut spec = command("/usr/bin/sleep");
    spec.args = vec!["0.02".into()];
    spec.observe_linux_identity = true;

    let output = spec.run().unwrap();
    let identity = output.identity.unwrap();

    assert!(identity.pid > 0);
    assert!(identity.start_time_ticks > 0);
}

#[test]
fn should_kill_a_subprocess_that_exceeds_its_output_bound() {
    let mut spec = command("/usr/bin/yes");
    spec.maximum_output = 128;

    let error = spec.run().unwrap_err();

    assert!(error.to_string().contains("output bound"));
}

#[test]
fn should_kill_a_subprocess_that_exceeds_its_timeout() {
    let mut spec = command("/usr/bin/sleep");
    spec.args = vec!["5".into()];
    spec.timeout = Duration::from_millis(20);

    let error = spec.run().unwrap_err();

    assert!(error.to_string().contains("timeout"));
}

#[test]
fn should_kill_descendants_in_the_timed_out_process_group() {
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("descendant.pid");
    let mut spec = command("/usr/bin/sh");
    spec.args = vec![
        "-c".into(),
        "sleep 30 & echo $! > \"$1\"; wait".into(),
        "sh".into(),
        pid_file.display().to_string(),
    ];
    spec.timeout = Duration::from_millis(500);

    let error = spec.run().unwrap_err();
    let descendant: u32 = std::fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let stat = std::fs::read_to_string(format!("/proc/{descendant}/stat"));
    let running = stat.ok().is_some_and(|value| {
        value
            .rsplit_once(") ")
            .and_then(|(_, fields)| fields.chars().next())
            .is_some_and(|state| state != 'Z')
    });

    assert!(error.to_string().contains("timeout"));
    assert!(
        !running,
        "descendant {descendant} survived process-group reap"
    );
}

#[test]
fn should_reject_a_symlink_program_target() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("env-link");
    std::os::unix::fs::symlink("/usr/bin/env", &link).unwrap();
    let mut spec = command("/usr/bin/env");
    spec.program = link;

    assert!(spec.run().is_err());
}

#[test]
fn should_reject_a_hard_link_program_target() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("program");
    let alias = dir.path().join("alias");
    std::fs::write(&program, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::hard_link(&program, &alias).unwrap();
    let mut spec = command("/usr/bin/env");
    spec.program = program;

    let error = spec.run().unwrap_err();

    assert!(error.to_string().contains("hard link"));
}

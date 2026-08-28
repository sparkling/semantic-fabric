use std::process::{Command, Output};

fn query(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sparql-query-regression-baseline"))
        .args(arguments)
        .output()
        .expect("run query regression baseline CLI")
}

#[test]
fn help_alone_is_successful_and_does_not_execute() {
    let output = query(&["--help"]);

    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("--check | --generate")
    );
}

#[test]
fn help_combined_with_check_is_rejected() {
    let output = query(&["--help", "--check"]);

    assert!(
        !output.status.success()
            && String::from_utf8_lossy(&output.stderr).contains("--help cannot be combined")
    );
}

#[test]
fn multiple_modes_are_rejected() {
    let output = query(&["--check", "--generate"]);

    assert!(
        !output.status.success()
            && String::from_utf8_lossy(&output.stderr).contains("choose exactly one")
    );
}

#[test]
fn missing_mode_is_rejected() {
    let output = query(&[]);

    assert!(
        !output.status.success()
            && String::from_utf8_lossy(&output.stderr).contains("choose exactly one")
    );
}

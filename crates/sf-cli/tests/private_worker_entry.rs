#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

const BINARY: &str = env!("CARGO_BIN_EXE_semantic-fabric");
const PRIVATE_WORKER_NAME: &str = "sf-parser-worker-v1";
const PRIVATE_WORKER_MODE: &str = "--sf-private-parser-worker-v1";
const PRIVATE_QUERY_V1_TRANSPORT_NAME: &str = "sf-query-v1-transport-peer-v1";
const PRIVATE_QUERY_V1_TRANSPORT_MODE: &str = "--sf-private-query-v1-transport-peer-v1";
const PRIVATE_QUERY_V1_TRANSPORT_MUTANT_NAME: &str = "sf-query-v1-transport-mutant-peer-v1";
const PRIVATE_QUERY_V1_TRANSPORT_MUTANT_MODE: &str =
    "--sf-private-query-v1-transport-mutant-peer-v1";
const PRIVATE_WORKER_REJECTED_EXIT_CODE: i32 = 78;

fn run(argument_zero: &str, arguments: &[&str], clear_environment: bool) -> Output {
    let mut command = Command::new(BINARY);
    command.arg0(argument_zero).args(arguments);
    if clear_environment {
        command.env_clear();
    }
    command.output().expect("run semantic-fabric")
}

fn assert_private_rejection(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(PRIVATE_WORKER_REJECTED_EXIT_CODE)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn ordinary_help_still_reaches_clap() {
    let output = run("semantic-fabric", &["--help"], false);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
}

#[test]
fn exact_private_tuple_without_supervisor_envelope_exits_silently() {
    for (name, mode) in [
        (PRIVATE_WORKER_NAME, PRIVATE_WORKER_MODE),
        (
            PRIVATE_QUERY_V1_TRANSPORT_NAME,
            PRIVATE_QUERY_V1_TRANSPORT_MODE,
        ),
        (
            PRIVATE_QUERY_V1_TRANSPORT_MUTANT_NAME,
            PRIVATE_QUERY_V1_TRANSPORT_MUTANT_MODE,
        ),
    ] {
        assert_private_rejection(&run(name, &[mode], true));
    }
}

#[test]
fn exact_private_tuple_rejects_a_nonempty_environment() {
    let mut command = Command::new(BINARY);
    command
        .arg0(PRIVATE_WORKER_NAME)
        .arg(PRIVATE_WORKER_MODE)
        .env_clear()
        .env("SF_PRIVATE_WORKER_ENV_CANARY", "present");
    assert_private_rejection(&command.output().expect("run private environment canary"));
}

#[cfg(target_os = "linux")]
#[test]
fn exact_private_tuple_rejects_a_malformed_raw_environment_entry() {
    use std::ffi::CString;
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::FromRawFd;

    let executable = CString::new(BINARY).expect("binary path has no NUL");
    let argument_zero = CString::new(PRIVATE_WORKER_NAME).unwrap();
    let argument_one = CString::new(PRIVATE_WORKER_MODE).unwrap();
    let malformed_environment = CString::new("MALFORMED_WITHOUT_EQUALS").unwrap();
    let arguments = [
        argument_zero.as_ptr(),
        argument_one.as_ptr(),
        std::ptr::null(),
    ];
    let environment = [malformed_environment.as_ptr(), std::ptr::null()];
    let mut pipe = [-1; 2];

    // SAFETY: all pointer arrays and strings are built before fork and remain
    // live until the child execs. The child performs only raw syscalls.
    unsafe {
        assert_eq!(libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC), 0);
        let child = libc::fork();
        assert!(child >= 0, "fork raw-environment canary");
        if child == 0 {
            libc::close(pipe[0]);
            if libc::dup2(pipe[1], libc::STDOUT_FILENO) < 0
                || libc::dup2(pipe[1], libc::STDERR_FILENO) < 0
            {
                libc::_exit(126);
            }
            libc::close(pipe[1]);
            libc::execve(
                executable.as_ptr(),
                arguments.as_ptr(),
                environment.as_ptr(),
            );
            libc::_exit(127);
        }

        libc::close(pipe[1]);
        let mut output = Vec::new();
        File::from_raw_fd(pipe[0])
            .read_to_end(&mut output)
            .expect("read raw-environment child output");
        let mut status = 0;
        assert_eq!(libc::waitpid(child, &mut status, 0), child);
        assert!(libc::WIFEXITED(status));
        assert_eq!(libc::WEXITSTATUS(status), PRIVATE_WORKER_REJECTED_EXIT_CODE);
        assert!(output.is_empty());
    }
}

#[test]
fn malformed_reserved_invocations_never_reach_clap() {
    assert_private_rejection(&run(PRIVATE_WORKER_NAME, &[], true));
    assert_private_rejection(&run("semantic-fabric", &[PRIVATE_WORKER_MODE], true));
    assert_private_rejection(&run(
        PRIVATE_WORKER_NAME,
        &[PRIVATE_WORKER_MODE, "extra"],
        true,
    ));
    assert_private_rejection(&run(
        PRIVATE_QUERY_V1_TRANSPORT_NAME,
        &[PRIVATE_QUERY_V1_TRANSPORT_MUTANT_MODE],
        true,
    ));
    assert_private_rejection(&run(
        PRIVATE_QUERY_V1_TRANSPORT_MUTANT_NAME,
        &["--sf-private-query-v1-transport-mutant-peer-v1x"],
        true,
    ));
}

#[test]
fn near_match_remains_an_ordinary_cli_invocation() {
    let output = run("sf-parser-worker-v1x", &["--help"], true);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
}

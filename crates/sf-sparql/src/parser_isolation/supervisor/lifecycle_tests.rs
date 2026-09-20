//! Containment and reap-ordering tests for `lifecycle.rs`, split out to
//! keep that module within the repository's 500-line source limit.

mod tests {
    use std::fs::File;
    use std::os::fd::{AsRawFd, OwnedFd};

    use super::super::*;
    use crate::parser_isolation::profile::v1_limits;
    use crate::parser_isolation::supervisor::executable::PreparedParserExecutable;
    use crate::parser_isolation::supervisor::linux::spawn_fixture;

    #[test]
    fn pidfd_poll_error_is_contained_and_reaped_before_recovery() {
        let executable = prepared_cat();
        let mut child = live_cat(&executable);
        let pidfd = child.duplicate_pidfd().expect("duplicate pidfd");

        let error = child
            .wait_until_deadline_with(
                |_, _| Err(SupervisorError::InvalidState("injected pidfd poll failure")),
                |_| Ok(()),
            )
            .expect_err("poll failure must fail closed");

        assert!(matches!(
            error,
            SupervisorError::InvalidState("injected pidfd poll failure")
        ));
        assert!(child.child.is_none());
        assert!(!pidfd_targets_live_process(&pidfd));
        successful_round_trip(&executable);
    }

    #[test]
    fn exited_inspection_error_is_contained_and_reaped_before_recovery() {
        let executable = prepared_cat();
        let mut child = live_cat(&executable);
        let pidfd = child.duplicate_pidfd().expect("duplicate pidfd");

        let error = child
            .wait_until_deadline_with(
                |_, _| Ok(true),
                |_| {
                    Err(SupervisorError::InvalidState(
                        "injected exited inspection failure",
                    ))
                },
            )
            .expect_err("inspection failure must fail closed");

        assert!(matches!(
            error,
            SupervisorError::InvalidState("injected exited inspection failure")
        ));
        assert!(child.child.is_none());
        assert!(!pidfd_targets_live_process(&pidfd));
        successful_round_trip(&executable);
    }

    fn prepared_cat() -> PreparedParserExecutable {
        PreparedParserExecutable::from_file_for_test(
            File::open("/bin/cat").expect("open cat fixture"),
            true,
        )
        .expect("prepare cat fixture")
    }

    fn live_cat(executable: &PreparedParserExecutable) -> ParserWorkerProcess {
        spawn_fixture(executable, v1_limits(), &[b"cat", b"-"]).expect("launch cat fixture")
    }

    fn successful_round_trip(executable: &PreparedParserExecutable) {
        let mut child = live_cat(executable);
        child.write_all_until_deadline(b"R").expect("write byte");
        let mut output = [0_u8; 1];
        child
            .read_exact_until_deadline(&mut output)
            .expect("read echoed byte");
        assert_eq!(output, [b'R']);
        child.close_stdin();
        assert!(child.wait_until_deadline().expect("reap cat").success());
    }

    fn pidfd_targets_live_process(pidfd: &OwnedFd) -> bool {
        // SAFETY: signal zero only probes the process identified by this live pidfd.
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                0,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            true
        } else {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            false
        }
    }
}

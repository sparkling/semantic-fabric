//! Boundary and recovery tests for cumulative, deadline-aware worker I/O.

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod linux_tests {
    use std::fs::File;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::time::{Duration, Instant};

    use super::super::executable::PreparedParserExecutable;
    use super::super::linux::spawn_fixture;
    use super::super::{v1_limits, SupervisorError};
    use crate::parser_isolation::protocol::ParserWorkerLimits;

    #[test]
    fn cumulative_input_accepts_zero_and_exact_n_then_reaps_on_n_plus_one() {
        let executable = prepared("/bin/cat");
        let mut values = v1_limits().values();
        values.max_input_bytes = 2;
        values.max_output_bytes = 2;
        let mut child = spawn_fixture(
            &executable,
            ParserWorkerLimits::new(values).unwrap(),
            &[b"cat", b"-"],
        )
        .expect("launch bounded echo worker");

        child
            .write_all_until_deadline(&[])
            .expect("zero-byte write is canonical");
        child
            .read_exact_until_deadline(&mut [])
            .expect("zero-byte read is canonical");
        assert_eq!(child.sent_bytes(), 0);
        assert_eq!(child.received_bytes(), 0);
        for byte in [b'A', b'B'] {
            child.write_all_until_deadline(&[byte]).unwrap();
            let mut echoed = [0_u8; 1];
            child.read_exact_until_deadline(&mut echoed).unwrap();
            assert_eq!(echoed, [byte]);
        }
        assert_eq!(child.sent_bytes(), 2);
        assert_eq!(child.received_bytes(), 2);
        let pidfd = child.duplicate_pidfd().unwrap();

        assert!(matches!(
            child.write_all_until_deadline(b"C"),
            Err(SupervisorError::InvalidState(
                "parser worker input exceeds its cumulative byte limit"
            ))
        ));
        assert!(child.child.is_none(), "failed I/O must reap the child");
        assert!(!pidfd_targets_live_process(&pidfd));
        successful_round_trip(&executable);
    }

    #[test]
    fn cumulative_output_rejects_n_plus_one_before_reading_it() {
        let executable = prepared("/bin/cat");
        let mut values = v1_limits().values();
        values.max_input_bytes = 2;
        values.max_output_bytes = 1;
        let mut child = spawn_fixture(
            &executable,
            ParserWorkerLimits::new(values).unwrap(),
            &[b"cat", b"-"],
        )
        .expect("launch bounded echo worker");
        child.write_all_until_deadline(b"AB").unwrap();
        let mut first = [0_u8; 1];
        child.read_exact_until_deadline(&mut first).unwrap();
        assert_eq!(first, [b'A']);
        assert_eq!(child.received_bytes(), 1);
        let pidfd = child.duplicate_pidfd().unwrap();

        assert!(matches!(
            child.read_exact_until_deadline(&mut [0_u8; 1]),
            Err(SupervisorError::InvalidState(
                "parser worker output exceeds its cumulative byte limit"
            ))
        ));
        assert!(child.child.is_none(), "failed I/O must reap the child");
        assert!(!pidfd_targets_live_process(&pidfd));
    }

    #[test]
    fn output_eof_is_exact_and_trailing_bytes_are_contained_before_recovery() {
        let shell = prepared("/bin/sh");
        let mut values = v1_limits().values();
        values.max_output_bytes = 1;
        let exact_output_limit = ParserWorkerLimits::new(values).unwrap();
        let mut clean = spawn_fixture(&shell, exact_output_limit, &[b"sh", b"-c", b"printf R"])
            .expect("launch clean EOF fixture");
        let mut frame = [0_u8; 1];
        clean.read_exact_until_deadline(&mut frame).unwrap();
        assert_eq!(frame, [b'R']);
        assert_eq!(clean.received_bytes(), 1);
        clean.expect_stdout_eof_until_deadline().unwrap();
        assert_eq!(clean.received_bytes(), 1);
        assert!(clean.wait_until_deadline().unwrap().success());

        let mut trailing = spawn_fixture(&shell, exact_output_limit, &[b"sh", b"-c", b"printf RZ"])
            .expect("launch trailing-output fixture");
        trailing.read_exact_until_deadline(&mut frame).unwrap();
        assert_eq!(frame, [b'R']);
        let pidfd = trailing.duplicate_pidfd().unwrap();
        assert!(matches!(
            trailing.expect_stdout_eof_until_deadline(),
            Err(SupervisorError::InvalidState(
                "parser worker emitted trailing protocol output"
            ))
        ));
        assert_eq!(trailing.received_bytes(), 1);
        assert!(trailing.child.is_none(), "trailing output must reap");
        assert!(!pidfd_targets_live_process(&pidfd));
        successful_round_trip(&prepared("/bin/cat"));
    }

    #[test]
    fn local_observation_accepts_silent_live_peer_and_contains_early_output() {
        let executable = prepared("/bin/cat");
        let mut silent = spawn_fixture(&executable, v1_limits(), &[b"cat", b"-"])
            .expect("launch silent live fixture");
        silent
            .observe_alive_and_silent_until(Instant::now() + Duration::from_millis(20))
            .expect("cat stays live and silent while stdin is held open");
        assert!(silent.child.is_some());
        assert_eq!(silent.received_bytes(), 0);
        silent.close_stdin();
        assert!(silent.wait_until_deadline().unwrap().success());

        let mut noisy = spawn_fixture(&executable, v1_limits(), &[b"cat", b"-"])
            .expect("launch early-output fixture");
        noisy.write_all_until_deadline(b"X").unwrap();
        let pidfd = noisy.duplicate_pidfd().unwrap();
        assert!(matches!(
            noisy.observe_alive_and_silent_until(Instant::now() + Duration::from_millis(100)),
            Err(SupervisorError::InvalidState(
                "parser worker emitted result bytes before request EOF"
            ))
        ));
        assert!(noisy.child.is_none(), "early output must be reaped");
        assert!(!pidfd_targets_live_process(&pidfd));
        successful_round_trip(&executable);
    }

    #[test]
    fn stalled_read_uses_the_spawn_deadline_and_recovers() {
        let executable = prepared("/bin/sh");
        let mut child = spawn_stalled_shell(&executable, v1_limits().values().max_input_bytes);
        let pidfd = child.duplicate_pidfd().unwrap();

        assert!(matches!(
            child.read_exact_until_deadline(&mut [0_u8; 1]),
            Err(SupervisorError::DeadlineExceeded)
        ));
        assert!(child.child.is_none());
        assert!(!pidfd_targets_live_process(&pidfd));
        successful_round_trip(&prepared("/bin/cat"));
    }

    #[test]
    fn partial_stalled_write_is_bounded_by_the_same_deadline() {
        const INPUT_BYTES: usize = 2 * 1024 * 1024;
        let executable = prepared("/bin/sh");
        // This witness requires progress before expiry, unlike stalled_read.
        // Allocate before spawning and allow test scheduling under parallel load;
        // still require expiry at the original, never-reset spawn deadline.
        let input = vec![b'X'; INPUT_BYTES];
        let mut values = short_limits(INPUT_BYTES as u64).values();
        values.wall_time_millis = 2_000;
        let mut child = spawn_fixture(
            &executable,
            ParserWorkerLimits::new(values).unwrap(),
            &[b"sh", b"-c", b"while :; do :; done"],
        )
        .expect("launch partial-write fixture");
        let deadline = child.wall_deadline;
        let pidfd = child.duplicate_pidfd().unwrap();

        assert!(matches!(
            child.write_all_until_deadline(&input),
            Err(SupervisorError::DeadlineExceeded)
        ));
        assert!(child.sent_bytes() > 0);
        assert!(child.sent_bytes() < INPUT_BYTES as u64);
        assert_eq!(child.wall_deadline, deadline);
        assert!(Instant::now() >= deadline);
        assert!(child.child.is_none());
        assert!(!pidfd_targets_live_process(&pidfd));
    }

    #[test]
    fn truncated_output_is_closed_and_reaped_without_reflecting_bytes() {
        let executable = prepared("/bin/sh");
        // This proves exact EOF classification, not progress within 100ms.
        let mut child = spawn_fixture(&executable, v1_limits(), &[b"sh", b"-c", b"printf X"])
            .expect("launch truncating fixture");
        let pidfd = child.duplicate_pidfd().unwrap();
        let mut output = [0_u8; 2];

        let error = child
            .read_exact_until_deadline(&mut output)
            .expect_err("one output byte cannot satisfy a two-byte frame");
        assert!(
            matches!(
                error,
                SupervisorError::InvalidState(
                    "parser worker output ended before the fixed read completed"
                ) | SupervisorError::InvalidState(
                    "parser worker exited before fixed I/O completed"
                )
            ),
            "{error:?}"
        );
        assert_eq!(child.received_bytes(), 1);
        assert!(child.child.is_none());
        assert!(!pidfd_targets_live_process(&pidfd));
    }

    #[test]
    fn closed_input_pipe_is_contained_as_an_error() {
        let executable = prepared("/bin/true");
        for expired in [false, true] {
            // This tests a closed pipe, not 100ms scheduler responsiveness.
            // wait_for_exit permits 1s; a shorter worker deadline made error
            // precedence depend on parallel test load. Test both states explicitly.
            let mut limits = v1_limits().values();
            limits.max_input_bytes = 1;
            let mut child = spawn_fixture(
                &executable,
                ParserWorkerLimits::new(limits).unwrap(),
                &[b"true"],
            )
            .expect("launch immediate-exit fixture");
            let pidfd = child.duplicate_pidfd().unwrap();
            wait_for_exit(&pidfd);
            if expired {
                child.wall_deadline = Instant::now();
            }
            let error = child
                .write_all_until_deadline(b"X")
                .expect_err("a closed input pipe cannot accept a frame");
            if expired {
                assert!(
                    matches!(error, SupervisorError::DeadlineExceeded),
                    "{error:?}"
                );
            } else {
                assert!(
                    matches!(
                        error,
                        SupervisorError::InvalidState(
                            "parser worker exited before fixed I/O completed"
                        ) | SupervisorError::Operation {
                            operation: "write parser worker input",
                            ..
                        }
                    ),
                    "{error:?}"
                );
            }
            assert!(child.child.is_none());
            assert!(!pidfd_targets_live_process(&pidfd));
        }
    }

    #[test]
    fn buffered_output_cannot_reset_or_outlive_the_spawn_deadline() {
        let executable = prepared("/bin/cat");
        let mut values = v1_limits().values();
        values.wall_time_millis = 250;
        let mut child = spawn_fixture(
            &executable,
            ParserWorkerLimits::new(values).unwrap(),
            &[b"cat", b"-"],
        )
        .expect("launch deadline fixture");
        child.write_all_until_deadline(b"D").unwrap();
        let deadline = child.wall_deadline;
        while Instant::now() < deadline {
            std::hint::spin_loop();
        }
        let pidfd = child.duplicate_pidfd().unwrap();

        assert!(matches!(
            child.read_exact_until_deadline(&mut [0_u8; 1]),
            Err(SupervisorError::DeadlineExceeded)
        ));
        assert!(child.child.is_none());
        assert!(!pidfd_targets_live_process(&pidfd));
    }

    #[test]
    fn zero_length_io_cannot_bypass_the_spawn_deadline() {
        let executable = prepared("/bin/cat");
        for read in [false, true] {
            let mut values = v1_limits().values();
            values.wall_time_millis = 1;
            let mut child = spawn_fixture(
                &executable,
                ParserWorkerLimits::new(values).unwrap(),
                &[b"cat", b"-"],
            )
            .expect("launch deadline fixture");
            let deadline = child.wall_deadline;
            while Instant::now() < deadline {
                std::hint::spin_loop();
            }
            let pidfd = child.duplicate_pidfd().unwrap();

            let result = if read {
                child.read_exact_until_deadline(&mut [])
            } else {
                child.write_all_until_deadline(&[])
            };
            assert!(matches!(result, Err(SupervisorError::DeadlineExceeded)));
            assert!(child.child.is_none());
            assert!(!pidfd_targets_live_process(&pidfd));
        }
    }

    fn spawn_stalled_shell(
        executable: &PreparedParserExecutable,
        max_input_bytes: u64,
    ) -> super::super::lifecycle::ParserWorkerProcess {
        spawn_fixture(
            executable,
            short_limits(max_input_bytes),
            &[b"sh", b"-c", b"while :; do :; done"],
        )
        .expect("launch stalled fixture")
    }

    fn short_limits(max_input_bytes: u64) -> ParserWorkerLimits {
        let mut values = v1_limits().values();
        values.wall_time_millis = 100;
        values.max_input_bytes = max_input_bytes;
        ParserWorkerLimits::new(values).unwrap()
    }

    fn prepared(path: &str) -> PreparedParserExecutable {
        PreparedParserExecutable::from_file_for_test(
            File::open(path).expect("open executable fixture"),
            true,
        )
        .expect("prepare executable fixture")
    }

    fn successful_round_trip(executable: &PreparedParserExecutable) {
        let mut child =
            spawn_fixture(executable, v1_limits(), &[b"cat", b"-"]).expect("launch echo");
        child.write_all_until_deadline(b"R").unwrap();
        let mut output = [0_u8; 1];
        child.read_exact_until_deadline(&mut output).unwrap();
        assert_eq!(output, [b'R']);
        child.close_stdin();
        assert!(child.wait_until_deadline().unwrap().success());
    }

    fn pidfd_targets_live_process(pidfd: &OwnedFd) -> bool {
        // SAFETY: signal zero probes the process identified by this live pidfd.
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

    fn wait_for_exit(pidfd: &OwnedFd) {
        let mut descriptor = libc::pollfd {
            fd: pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: descriptor is a valid writable one-element pollfd array.
        assert_eq!(unsafe { libc::poll(&mut descriptor, 1, 1_000) }, 1);
        assert_ne!(descriptor.revents & libc::POLLIN, 0);
    }
}

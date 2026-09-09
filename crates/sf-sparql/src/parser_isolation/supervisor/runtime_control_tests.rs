//! Owned pipe fixtures prove request cancellation stops and reaps real work.
use super::{
    executable::PreparedParserExecutable, linux::spawn_fixture, v1_limits, SupervisorError,
};
use crate::parser_isolation::runtime::from_prepared_for_test;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Timed {
    budget: QueryBudget,
    until: Instant,
}

#[test]
fn runtime_omits_only_diagnostic_sha_and_preserves_held_launch_identity() {
    let runtime =
        PreparedParserExecutable::from_file_for_runtime(std::fs::File::open("/bin/cat").unwrap())
            .unwrap();
    let evidence =
        PreparedParserExecutable::from_file_for_evidence(std::fs::File::open("/bin/cat").unwrap())
            .unwrap();
    assert!(runtime.identity().fingerprint().is_none());
    assert!(evidence.identity().fingerprint().is_some());
    assert_eq!(runtime.identity().device(), evidence.identity().device());
    assert_eq!(runtime.identity().inode(), evidence.identity().inode());
    assert_eq!(
        runtime.identity().build_identity(),
        evidence.identity().build_identity()
    );
    runtime.duplicate_for_launch(64).unwrap();
}
impl QueryControl for Timed {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        if Instant::now() >= self.until {
            self.budget.terminate(QueryControlError::DeadlineExceeded);
        }
        self.budget.checkpoint()
    }
    fn consume(&self, charge: QueryCharge, n: u64) -> Result<(), QueryControlError> {
        self.checkpoint()?;
        self.budget.consume(charge, n)
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

#[test]
fn runtime_build_identity_is_bounded_independently_of_debug_image_size() {
    use super::executable::{TempDirectory, MAX_EXECUTABLE_BYTES};
    use std::fs::{File, OpenOptions};

    let directory = TempDirectory::new();
    let path = directory.path().join("large-elf");
    std::fs::copy("/bin/cat", &path).unwrap();
    let file = OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(MAX_EXECUTABLE_BYTES + 1).unwrap();
    drop(file);
    let runtime = PreparedParserExecutable::from_file_for_runtime(File::open(&path).unwrap())
        .expect("bounded build-ID reads do not scan trailing debug bytes");
    assert!(runtime.identity().fingerprint().is_none());
    assert_eq!(runtime.identity().byte_len(), MAX_EXECUTABLE_BYTES + 1);
    assert_eq!(
        runtime.identity().build_identity(),
        PreparedParserExecutable::from_file_for_runtime(File::open("/bin/cat").unwrap())
            .unwrap()
            .identity()
            .build_identity()
    );
    runtime.duplicate_for_launch(64).unwrap();
    assert!(matches!(
        PreparedParserExecutable::from_file_for_evidence(File::open(&path).unwrap()),
        Err(SupervisorError::InvalidExecutable(
            "descriptor exceeds the fixed fingerprint byte ceiling"
        ))
    ));
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(MAX_EXECUTABLE_BYTES + 2)
        .unwrap();
    assert!(matches!(
        runtime.duplicate_for_launch(64),
        Err(SupervisorError::InvalidExecutable(
            "launch descriptor identity drifted"
        ))
    ));
}

#[test]
fn request_cancel_and_deadline_interrupt_pipe_wait_and_reap_before_return() {
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let executable = PreparedParserExecutable::from_file_for_evidence(
            std::fs::File::open("/bin/cat").unwrap(),
        )
        .unwrap();
        let mut child = spawn_fixture(&executable, v1_limits(), &[b"cat", b"-"]).unwrap();
        // No input is sent. The live echo fixture cannot complete this read.
        let pidfd = child.duplicate_pidfd().unwrap();
        let runtime = from_prepared_for_test(executable);
        let budget = QueryBudget::new(QueryLimits::new(100, 100, 100, 100));
        let control: Arc<dyn QueryControl> = if reason == QueryControlError::DeadlineExceeded {
            Arc::new(Timed {
                budget: budget.clone(),
                until: Instant::now() + Duration::from_millis(30),
            })
        } else {
            Arc::new(budget.clone())
        };
        let cancellation = (reason == QueryControlError::Cancelled).then(|| {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                budget.terminate(reason);
            })
        });
        let start = Instant::now();
        let result = runtime.with_request(control, || {
            let error = child.read_exact_until_deadline(&mut [0_u8; 1]).unwrap_err();
            assert!(matches!(error, SupervisorError::RequestControl(actual) if actual == reason));
            assert!(
                child.child.is_none(),
                "request may return only after exact reap"
            );
            Ok(())
        });
        assert!(matches!(result, Err(crate::Error::QueryControl(actual)) if actual == reason));
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "must not wait for the 15-second worker cap"
        );
        use std::os::fd::AsRawFd;
        let alive = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                0,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        assert_eq!(alive, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        if let Some(cancellation) = cancellation {
            cancellation.join().unwrap();
        }
    }
}

//! Pure, descriptor-free boundary arithmetic checks for the bounded-step
//! primitives, split out of `io_tests.rs` to keep every module within the
//! repository's 500-line source limit.

/// Pure, executable-free boundary checks for the bounded-step primitives.
/// Lives here (rather than inline in `io.rs`) to keep that module within the
/// repository's 500-line source limit.
mod unit_tests {
    use std::time::{Duration, Instant};

    use super::super::super::io::{BoundedWorkerIo, INPUT_LIMIT_MESSAGE, OUTPUT_LIMIT_MESSAGE};
    use super::super::super::io_bounds::{
        admitted_total, ensure_deadline_not_passed, prospective_total, step_poll_timeout_ms,
    };
    use super::super::super::SupervisorError;

    #[test]
    fn prospective_totals_are_checked_before_io() {
        assert_eq!(prospective_total(3, 2, 5), Some(5));
        assert_eq!(prospective_total(3, 3, 5), None);
        assert_eq!(prospective_total(u64::MAX, 1, u64::MAX), None);
    }

    #[test]
    fn receive_budget_is_prospected_without_mutating_accounting() {
        let io = BoundedWorkerIo::descriptorless_for_test(0, 3, 1, 5);

        let future = Instant::now() + Duration::from_secs(1);
        assert!(io.ensure_can_receive(future, 2).is_ok());
        assert!(io.ensure_can_receive(future, 3).is_err());
        assert!(io.ensure_can_receive(Instant::now(), 0).is_err());
        assert_eq!(io.received, 3);
    }

    /// The direct bounded-step path (the async SQL-canonicalize peer) cannot
    /// reach `write_step`/`read_step` without a reservation, and the
    /// reservation applies exactly the same whole-transfer admission the
    /// synchronous `write_all`/`read_exact` loops always applied. A transfer
    /// that cannot fit the immutable cumulative cap is therefore refused
    /// BEFORE any byte moves, never after partial I/O.
    #[test]
    fn step_reservations_admit_whole_transfers_against_cumulative_caps() {
        let io = BoundedWorkerIo::descriptorless_for_test(3, 3, 5, 5);
        let future = Instant::now() + Duration::from_secs(1);

        // Exact fit is admitted; one byte more is refused up front.
        assert!(io.reserve_write(future, &[0_u8; 2]).is_ok());
        assert!(matches!(
            io.reserve_write(future, &[0_u8; 3]),
            Err(SupervisorError::InvalidState(message))
                if message == INPUT_LIMIT_MESSAGE
        ));
        assert!(io.reserve_read(future, 2).is_ok());
        assert!(matches!(
            io.reserve_read(future, 3),
            Err(SupervisorError::InvalidState(message))
                if message == OUTPUT_LIMIT_MESSAGE
        ));

        // Admission never mutates accounting, and an elapsed lifetime refuses
        // to start a transfer at all.
        assert_eq!(io.sent, 3);
        assert_eq!(io.received, 3);
        assert!(matches!(
            io.reserve_write(Instant::now(), &[]),
            Err(SupervisorError::DeadlineExceeded)
        ));
        assert!(matches!(
            io.reserve_read(Instant::now(), 0),
            Err(SupervisorError::DeadlineExceeded)
        ));
    }

    /// A reserved transfer records the total it must land on, so a completed
    /// transfer is verified against its own admission rather than trusting
    /// per-syscall arithmetic alone.
    #[test]
    fn completed_transfers_are_verified_against_their_admitted_total() {
        let mut io = BoundedWorkerIo::descriptorless_for_test(1, 1, 8, 8);
        let future = Instant::now() + Duration::from_secs(1);
        let write = io.reserve_write(future, &[0_u8; 3]).expect("admitted");
        let read = io.reserve_read(future, 3).expect("admitted");

        io.sent = 4;
        io.received = 4;
        assert!(io.verify_write_total(write).is_ok());
        assert!(io.verify_read_total(read).is_ok());

        io.sent = 5;
        io.received = 5;
        assert!(io.verify_write_total(write).is_err());
        assert!(io.verify_read_total(read).is_err());
    }

    /// One bounded step never polls past the immutable lifetime, and refuses
    /// to begin at all once it has elapsed -- independent of whatever
    /// (possibly larger) cap the caller supplies.
    #[test]
    fn step_poll_timeout_is_clamped_to_the_remaining_lifetime() {
        let deadline = Instant::now() + Duration::from_millis(50);
        let capped = step_poll_timeout_ms(deadline, Some(10)).expect("live");
        assert!(capped <= 10, "caller cap must apply: {capped}");

        // With no caller cap the remaining lifetime alone bounds the poll.
        let uncapped = step_poll_timeout_ms(deadline, None).expect("live");
        assert!(uncapped <= 51, "lifetime must bound the poll: {uncapped}");

        // A caller cap larger than the remaining lifetime cannot extend it.
        let short = Instant::now() + Duration::from_millis(2);
        let clamped = step_poll_timeout_ms(short, Some(10_000)).expect("live");
        assert!(clamped <= 3, "lifetime must win over the cap: {clamped}");

        assert!(matches!(
            step_poll_timeout_ms(Instant::now(), Some(10)),
            Err(SupervisorError::DeadlineExceeded)
        ));
        assert!(matches!(
            ensure_deadline_not_passed(Instant::now()),
            Err(SupervisorError::DeadlineExceeded)
        ));
    }

    /// Per-syscall accounting is bounded by the immutable ceiling, not just
    /// `checked_add`: a peer that somehow delivered more than its admitted
    /// share is refused rather than silently inflating the counters.
    #[test]
    fn admitted_totals_refuse_to_exceed_the_immutable_ceiling() {
        assert_eq!(admitted_total(3, 2, 5, INPUT_LIMIT_MESSAGE).unwrap(), 5);
        assert!(admitted_total(3, 3, 5, INPUT_LIMIT_MESSAGE).is_err());
        assert!(admitted_total(u64::MAX, 1, u64::MAX, OUTPUT_LIMIT_MESSAGE).is_err());
        // `checked_add` alone would accept both of the refusals above; the
        // ceiling comparison is what makes them fail.
        assert!(3_u64.checked_add(3).is_some());
    }

    /// The step methods must use the ceiling-bounded accumulator, not bare
    /// `checked_add`. Verified structurally: the reservation admits a
    /// transfer only while it fits, and the accumulator then refuses any
    /// byte beyond that same ceiling, so the two agree on the boundary.
    #[test]
    fn step_accounting_and_admission_agree_on_the_same_ceiling() {
        let io = BoundedWorkerIo::descriptorless_for_test(0, 0, 4, 4);
        let future = Instant::now() + Duration::from_secs(1);

        // Admission accepts exactly up to the ceiling...
        assert!(io.reserve_write(future, &[0_u8; 4]).is_ok());
        assert!(io.reserve_write(future, &[0_u8; 5]).is_err());
        // ...and per-syscall accounting refuses at exactly the same point,
        // so no observed write can carry the counter past the ceiling even
        // if a peer delivered more than its admitted share.
        assert_eq!(
            admitted_total(0, 4, io.max_sent(), INPUT_LIMIT_MESSAGE).unwrap(),
            4
        );
        assert!(admitted_total(0, 5, io.max_sent(), INPUT_LIMIT_MESSAGE).is_err());
        assert!(admitted_total(3, 2, io.max_sent(), INPUT_LIMIT_MESSAGE).is_err());
    }
}

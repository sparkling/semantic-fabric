//! Byte-exact private parser-worker entry discrimination.
//!
//! This entry runs before `sf-cli` invokes Clap or creates application thread
//! pools. The reserved tuple is only a routing sentinel, never authentication;
//! the future worker must verify its inherited kernel envelope before reading
//! peer-controlled bytes. Until that verifier exists, every reserved invocation
//! terminates silently and cannot fall through to the public CLI.

use std::ffi::{OsStr, OsString};

pub(super) const PRIVATE_WORKER_NAME: &str = "sf-parser-worker-v1";
pub(super) const PRIVATE_WORKER_MODE: &str = "--sf-private-parser-worker-v1";

const PRIVATE_WORKER_REJECTED_EXIT_CODE: i32 = 78;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrivateInvocation {
    Ordinary,
    ExactPrivate,
    MalformedReserved,
}

fn classify_private_invocation(arguments: impl IntoIterator<Item = OsString>) -> PrivateInvocation {
    let mut arguments = arguments.into_iter();
    let argument_zero = arguments.next();
    let argument_one = arguments.next();
    let has_argument_two = arguments.next().is_some();
    let reserved_name = argument_zero.as_deref() == Some(OsStr::new(PRIVATE_WORKER_NAME));
    let reserved_mode = argument_one.as_deref() == Some(OsStr::new(PRIVATE_WORKER_MODE));

    if reserved_name && reserved_mode && !has_argument_two {
        PrivateInvocation::ExactPrivate
    } else if reserved_name || reserved_mode {
        PrivateInvocation::MalformedReserved
    } else {
        PrivateInvocation::Ordinary
    }
}

/// Dispatch the reserved parser-worker invocation before public CLI parsing.
///
/// Ordinary invocations return. Reserved or malformed-reserved invocations
/// never reach Clap and emit no diagnostics containing peer-controlled bytes.
pub fn dispatch_private_parser_worker_v1() {
    match classify_private_invocation(std::env::args_os()) {
        PrivateInvocation::Ordinary => {}
        PrivateInvocation::MalformedReserved => reject_private_invocation(),
        PrivateInvocation::ExactPrivate => {
            if !private_environment_is_empty() {
                reject_private_invocation();
            }
            run_private_worker_v1();
        }
    }
}

#[cfg(target_os = "linux")]
fn private_environment_is_empty() -> bool {
    unsafe extern "C" {
        static mut environ: *mut *mut libc::c_char;
    }

    // `std::env::vars_os` filters malformed raw entries that contain no `=`.
    // The private exec contract is stronger: require the raw envp vector itself
    // to exist and contain no first entry. Application code has not created
    // threads at this first-statement boundary, so no in-process environment
    // mutation is permitted to race this read.
    unsafe {
        let environment = environ;
        raw_environment_is_empty(environment.cast_const().cast())
    }
}

#[cfg(target_os = "linux")]
unsafe fn raw_environment_is_empty(environment: *const *const libc::c_char) -> bool {
    !environment.is_null() && unsafe { (*environment).is_null() }
}

#[cfg(not(target_os = "linux"))]
fn private_environment_is_empty() -> bool {
    // V1 has no qualified worker outside Linux. Keep exact private invocations
    // on unsupported targets fail-closed rather than weakening this predicate.
    false
}

fn reject_private_invocation() -> ! {
    #[cfg(unix)]
    unsafe {
        // Do not run Rust destructors or process-wide atexit handlers on this
        // private fail-closed boundary.
        libc::_exit(PRIVATE_WORKER_REJECTED_EXIT_CODE)
    }
    #[cfg(not(unix))]
    std::process::exit(PRIVATE_WORKER_REJECTED_EXIT_CODE)
}

fn run_private_worker_v1() -> ! {
    // The final envelope verifier, policy installation, and Ready exchange are
    // deliberately a later slice. Exact private routing is fail-closed until
    // that complete transition exists.
    reject_private_invocation()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(arguments: &[&str]) -> PrivateInvocation {
        classify_private_invocation(arguments.iter().copied().map(OsString::from))
    }

    #[test]
    fn only_the_exact_two_argument_tuple_selects_private_mode() {
        assert_eq!(
            classify(&[PRIVATE_WORKER_NAME, PRIVATE_WORKER_MODE]),
            PrivateInvocation::ExactPrivate
        );
        assert_eq!(classify(&[]), PrivateInvocation::Ordinary);
        assert_eq!(classify(&["semantic-fabric"]), PrivateInvocation::Ordinary);
        assert_eq!(
            classify(&[PRIVATE_WORKER_NAME]),
            PrivateInvocation::MalformedReserved
        );
        assert_eq!(
            classify(&["semantic-fabric", PRIVATE_WORKER_MODE]),
            PrivateInvocation::MalformedReserved
        );
        assert_eq!(
            classify(&[PRIVATE_WORKER_NAME, "--help"]),
            PrivateInvocation::MalformedReserved
        );
        assert_eq!(
            classify(&[PRIVATE_WORKER_NAME, PRIVATE_WORKER_MODE, "extra"]),
            PrivateInvocation::MalformedReserved
        );
    }

    #[test]
    fn near_matches_and_reserved_tokens_in_later_positions_are_ordinary() {
        assert_eq!(
            classify(&["sf-parser-worker-v1x", "--sf-private-parser-worker-v1x",]),
            PrivateInvocation::Ordinary
        );
        assert_eq!(
            classify(&["semantic-fabric", "serve", PRIVATE_WORKER_MODE]),
            PrivateInvocation::Ordinary
        );
    }

    #[test]
    fn one_exact_reserved_position_with_a_near_companion_fails_closed() {
        assert_eq!(
            classify(&["sf-parser-worker-v1x", PRIVATE_WORKER_MODE]),
            PrivateInvocation::MalformedReserved
        );
        assert_eq!(
            classify(&[PRIVATE_WORKER_NAME, "--sf-private-parser-worker-v1x"]),
            PrivateInvocation::MalformedReserved
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_arguments_are_compared_without_lossy_conversion() {
        use std::os::unix::ffi::OsStringExt;

        let non_utf8 = OsString::from_vec(vec![0xff]);
        assert_eq!(
            classify_private_invocation([OsString::from(PRIVATE_WORKER_NAME), non_utf8.clone(),]),
            PrivateInvocation::MalformedReserved
        );
        assert_eq!(
            classify_private_invocation([non_utf8, OsString::from(PRIVATE_WORKER_MODE),]),
            PrivateInvocation::MalformedReserved
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn raw_environment_requires_one_present_null_pointer_slot() {
        use std::ffi::CString;

        let null_vector = std::ptr::null();
        assert!(!unsafe { raw_environment_is_empty(null_vector) });

        let empty = [std::ptr::null()];
        assert!(unsafe { raw_environment_is_empty(empty.as_ptr()) });

        let well_formed = CString::new("NAME=value").unwrap();
        let well_formed_vector = [well_formed.as_ptr(), std::ptr::null()];
        assert!(!unsafe { raw_environment_is_empty(well_formed_vector.as_ptr()) });

        let malformed = CString::new("MALFORMED_WITHOUT_EQUALS").unwrap();
        let malformed_vector = [malformed.as_ptr(), std::ptr::null()];
        assert!(!unsafe { raw_environment_is_empty(malformed_vector.as_ptr()) });
    }
}

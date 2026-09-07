//! Byte-exact private parser-worker entry discrimination.
//!
//! This entry runs before `sf-cli` invokes Clap or creates application thread
//! pools. The reserved tuple is only a routing sentinel, never authentication.
//! On qualified Linux, the worker verifies and repairs its inherited kernel
//! envelope, stacks a default-kill control-ready policy candidate, and only then
//! reads Hello. Unprepared or malformed reserved invocations terminate silently
//! and cannot fall through to the public CLI. The parser peer accepts no parse
//! request. Independently gated peers return a fixed parser-free QueryV1 fixture,
//! a real parser-produced QueryV1 for the internally sealed corpus, or an
//! aggregate-only parser observation. All remain qualification evidence.

use std::ffi::{OsStr, OsString};

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod linux;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod policy_candidate;

pub(super) const PRIVATE_WORKER_NAME: &str = "sf-parser-worker-v1";
pub(super) const PRIVATE_WORKER_MODE: &str = "--sf-private-parser-worker-v1";
pub(super) const PRIVATE_QUERY_V1_TRANSPORT_NAME: &str = "sf-query-v1-transport-peer-v1";
pub(super) const PRIVATE_QUERY_V1_TRANSPORT_MODE: &str = "--sf-private-query-v1-transport-peer-v1";
pub(super) const PRIVATE_QUERY_V1_TRANSPORT_MUTANT_NAME: &str =
    "sf-query-v1-transport-mutant-peer-v1";
pub(super) const PRIVATE_QUERY_V1_TRANSPORT_MUTANT_MODE: &str =
    "--sf-private-query-v1-transport-mutant-peer-v1";
pub(super) const PRIVATE_PARSER_OBSERVATION_NAME: &str = "sf-parser-observation-peer-v1";
pub(super) const PRIVATE_PARSER_OBSERVATION_MODE: &str = "--sf-private-parser-observation-peer-v1";
pub(super) const PRIVATE_PARSER_QUERY_V1_NAME: &str = "sf-parser-query-v1-peer-v1";
pub(super) const PRIVATE_PARSER_QUERY_V1_MODE: &str = "--sf-private-parser-query-v1-peer-v1";

const PRIVATE_WORKER_REJECTED_EXIT_CODE: i32 = 78;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrivateInvocation {
    Ordinary,
    ExactPrivate(PrivatePeer),
    MalformedReserved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrivatePeer {
    Parser,
    QueryV1Transport,
    QueryV1TransportMutant,
    ParserObservation,
    ParserQueryV1,
}

fn classify_private_invocation(arguments: impl IntoIterator<Item = OsString>) -> PrivateInvocation {
    let mut arguments = arguments.into_iter();
    let argument_zero = arguments.next();
    let argument_one = arguments.next();
    let has_argument_two = arguments.next().is_some();
    let peer = private_peer_for_tuple(argument_zero.as_deref(), argument_one.as_deref());
    let reserved =
        is_reserved_token(argument_zero.as_deref()) || is_reserved_token(argument_one.as_deref());

    if let Some(peer) = peer.filter(|_| !has_argument_two) {
        PrivateInvocation::ExactPrivate(peer)
    } else if reserved {
        PrivateInvocation::MalformedReserved
    } else {
        PrivateInvocation::Ordinary
    }
}

fn private_peer_for_tuple(name: Option<&OsStr>, mode: Option<&OsStr>) -> Option<PrivatePeer> {
    [
        (
            PRIVATE_WORKER_NAME,
            PRIVATE_WORKER_MODE,
            PrivatePeer::Parser,
        ),
        (
            PRIVATE_QUERY_V1_TRANSPORT_NAME,
            PRIVATE_QUERY_V1_TRANSPORT_MODE,
            PrivatePeer::QueryV1Transport,
        ),
        (
            PRIVATE_QUERY_V1_TRANSPORT_MUTANT_NAME,
            PRIVATE_QUERY_V1_TRANSPORT_MUTANT_MODE,
            PrivatePeer::QueryV1TransportMutant,
        ),
        (
            PRIVATE_PARSER_OBSERVATION_NAME,
            PRIVATE_PARSER_OBSERVATION_MODE,
            PrivatePeer::ParserObservation,
        ),
        (
            PRIVATE_PARSER_QUERY_V1_NAME,
            PRIVATE_PARSER_QUERY_V1_MODE,
            PrivatePeer::ParserQueryV1,
        ),
    ]
    .into_iter()
    .find_map(|(expected_name, expected_mode, peer)| {
        (name == Some(OsStr::new(expected_name)) && mode == Some(OsStr::new(expected_mode)))
            .then_some(peer)
    })
}

fn is_reserved_token(argument: Option<&OsStr>) -> bool {
    [
        PRIVATE_WORKER_NAME,
        PRIVATE_WORKER_MODE,
        PRIVATE_QUERY_V1_TRANSPORT_NAME,
        PRIVATE_QUERY_V1_TRANSPORT_MODE,
        PRIVATE_QUERY_V1_TRANSPORT_MUTANT_NAME,
        PRIVATE_QUERY_V1_TRANSPORT_MUTANT_MODE,
        PRIVATE_PARSER_OBSERVATION_NAME,
        PRIVATE_PARSER_OBSERVATION_MODE,
        PRIVATE_PARSER_QUERY_V1_NAME,
        PRIVATE_PARSER_QUERY_V1_MODE,
    ]
    .into_iter()
    .any(|reserved| argument == Some(OsStr::new(reserved)))
}

/// Dispatch the reserved parser-worker invocation before public CLI parsing.
///
/// Ordinary invocations return. Reserved or malformed-reserved invocations
/// never reach Clap and emit no diagnostics containing peer-controlled bytes.
pub fn dispatch_private_parser_worker_v1() {
    match classify_private_invocation(std::env::args_os()) {
        PrivateInvocation::Ordinary => {}
        PrivateInvocation::MalformedReserved => reject_private_invocation(),
        PrivateInvocation::ExactPrivate(peer) => {
            if !private_environment_is_empty() {
                reject_private_invocation();
            }
            run_private_peer_v1(peer);
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

fn run_private_peer_v1(peer: PrivatePeer) -> ! {
    match peer {
        PrivatePeer::Parser => run_private_worker_v1(),
        PrivatePeer::QueryV1Transport => run_query_v1_transport_worker_v1(),
        PrivatePeer::QueryV1TransportMutant => run_query_v1_transport_mutant_worker_v1(),
        PrivatePeer::ParserObservation => run_parser_observation_worker_v1(),
        PrivatePeer::ParserQueryV1 => run_parser_query_v1_worker_v1(),
    }
}

fn run_private_worker_v1() -> ! {
    #[cfg(all(
        feature = "parser-worker-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    ))]
    {
        linux::run()
    }
    #[cfg(not(all(
        feature = "parser-worker-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )))]
    reject_private_invocation()
}

fn run_query_v1_transport_worker_v1() -> ! {
    #[cfg(all(
        feature = "query-v1-transport-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    ))]
    {
        linux::run_query_v1_transport()
    }
    #[cfg(not(all(
        feature = "query-v1-transport-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )))]
    reject_private_invocation()
}

fn run_query_v1_transport_mutant_worker_v1() -> ! {
    #[cfg(all(
        feature = "query-v1-transport-mutant-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    ))]
    {
        linux::run_query_v1_transport_mutant()
    }
    #[cfg(not(all(
        feature = "query-v1-transport-mutant-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )))]
    reject_private_invocation()
}

fn run_parser_observation_worker_v1() -> ! {
    #[cfg(all(
        feature = "parser-worker-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    ))]
    {
        linux::run_parser_observation()
    }
    #[cfg(not(all(
        feature = "parser-worker-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )))]
    reject_private_invocation()
}

fn run_parser_query_v1_worker_v1() -> ! {
    #[cfg(all(
        feature = "parser-worker-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    ))]
    {
        linux::run_parser_query_v1()
    }
    #[cfg(not(all(
        feature = "parser-worker-evidence",
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )))]
    reject_private_invocation()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(arguments: &[&str]) -> PrivateInvocation {
        classify_private_invocation(arguments.iter().copied().map(OsString::from))
    }

    #[test]
    fn only_exact_reserved_tuples_select_private_peers() {
        for (name, mode, peer) in [
            (
                PRIVATE_WORKER_NAME,
                PRIVATE_WORKER_MODE,
                PrivatePeer::Parser,
            ),
            (
                PRIVATE_QUERY_V1_TRANSPORT_NAME,
                PRIVATE_QUERY_V1_TRANSPORT_MODE,
                PrivatePeer::QueryV1Transport,
            ),
            (
                PRIVATE_QUERY_V1_TRANSPORT_MUTANT_NAME,
                PRIVATE_QUERY_V1_TRANSPORT_MUTANT_MODE,
                PrivatePeer::QueryV1TransportMutant,
            ),
            (
                PRIVATE_PARSER_OBSERVATION_NAME,
                PRIVATE_PARSER_OBSERVATION_MODE,
                PrivatePeer::ParserObservation,
            ),
            (
                PRIVATE_PARSER_QUERY_V1_NAME,
                PRIVATE_PARSER_QUERY_V1_MODE,
                PrivatePeer::ParserQueryV1,
            ),
        ] {
            assert_eq!(
                classify(&[name, mode]),
                PrivateInvocation::ExactPrivate(peer)
            );
        }
        assert_eq!(classify(&[]), PrivateInvocation::Ordinary);
        assert_eq!(classify(&["semantic-fabric"]), PrivateInvocation::Ordinary);
    }

    #[test]
    fn every_reserved_partial_cross_and_extended_tuple_is_malformed() {
        let pairs = [
            (PRIVATE_WORKER_NAME, PRIVATE_WORKER_MODE),
            (
                PRIVATE_QUERY_V1_TRANSPORT_NAME,
                PRIVATE_QUERY_V1_TRANSPORT_MODE,
            ),
            (
                PRIVATE_QUERY_V1_TRANSPORT_MUTANT_NAME,
                PRIVATE_QUERY_V1_TRANSPORT_MUTANT_MODE,
            ),
            (
                PRIVATE_PARSER_OBSERVATION_NAME,
                PRIVATE_PARSER_OBSERVATION_MODE,
            ),
            (PRIVATE_PARSER_QUERY_V1_NAME, PRIVATE_PARSER_QUERY_V1_MODE),
        ];
        for (name, mode) in pairs {
            assert_eq!(classify(&[name]), PrivateInvocation::MalformedReserved);
            assert_eq!(
                classify(&["semantic-fabric", mode]),
                PrivateInvocation::MalformedReserved
            );
            assert_eq!(
                classify(&[name, "--help"]),
                PrivateInvocation::MalformedReserved
            );
            assert_eq!(
                classify(&[name, mode, "extra"]),
                PrivateInvocation::MalformedReserved
            );
        }
        for (name, _) in pairs {
            for (_, mode) in pairs {
                if private_peer_for_tuple(Some(OsStr::new(name)), Some(OsStr::new(mode))).is_none()
                {
                    assert_eq!(
                        classify(&[name, mode]),
                        PrivateInvocation::MalformedReserved
                    );
                }
            }
        }
    }

    #[test]
    fn near_matches_and_reserved_tokens_in_later_positions_are_ordinary() {
        assert_eq!(
            classify(&["sf-parser-worker-v1x", "--sf-private-parser-worker-v1x",]),
            PrivateInvocation::Ordinary
        );
        assert_eq!(
            classify(&[
                "sf-query-v1-transport-peer-v1x",
                "--sf-private-query-v1-transport-peer-v1x",
            ]),
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
        assert_eq!(
            classify(&[
                "sf-query-v1-transport-peer-v1x",
                PRIVATE_QUERY_V1_TRANSPORT_MODE,
            ]),
            PrivateInvocation::MalformedReserved
        );
        assert_eq!(
            classify(&[
                PRIVATE_QUERY_V1_TRANSPORT_MUTANT_NAME,
                "--sf-private-query-v1-transport-mutant-peer-v1x",
            ]),
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

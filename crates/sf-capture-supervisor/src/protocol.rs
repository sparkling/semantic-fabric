// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fmt, str::FromStr};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use crate::AuthorityError;

pub const SCHEMA_VERSION_V2: u8 = 2;
pub const TRANSACTION_KIND_V2: &str = "programme-capture-v2";
pub const RUN_EVENT_RECORD_KIND_V2: &str = "supervisor-run-event-v2";
pub const RUN_EVENT_DIGEST_DOMAIN_V2: &str =
    "semantic-fabric/programme-capture/supervisor-run-event-digest-v2";
pub const RESOURCE_CONFLICT_SET_DOMAIN_V2: &str =
    "semantic-fabric/programme-capture/supervisor-resource-conflict-set-v2";
pub const KERNEL_PROPOSAL_DIGEST_DOMAIN_V1: &str =
    "semantic-fabric/programme-capture/supervisor-authority-kernel-proposal-v1";
pub const MAX_REQUEST_BYTES_V2: usize = 32_768;
pub const MAX_EVENT_BYTES_V2: usize = 65_536;
pub const MAX_RESULT_BYTES_V2: usize = 196_608;
pub const MAX_RESOURCE_MEMBERS_V2: usize = 64;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Digest(String);

impl Digest {
    pub fn parse(value: impl Into<String>) -> Result<Self, AuthorityError> {
        let value = value.into();
        if value.len() != 64
            || value
                .bytes()
                .any(|byte| !matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            || value.bytes().all(|byte| byte == b'0')
        {
            return Err(AuthorityError::InvalidInput("digest"));
        }
        Ok(Self(value))
    }

    pub fn sha256(bytes: impl AsRef<[u8]>) -> Self {
        Self(format!("{:x}", Sha256::digest(bytes.as_ref())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for Digest {
    type Err = AuthorityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct OpaqueId(String);

impl OpaqueId {
    pub fn parse(value: impl Into<String>) -> Result<Self, AuthorityError> {
        let value = value.into();
        if !(8..=128).contains(&value.len())
            || value
                .bytes()
                .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'_' && byte != b'-')
        {
            return Err(AuthorityError::InvalidInput("opaque ID"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OpaqueId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for OpaqueId {
    type Err = AuthorityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for OpaqueId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalInstant {
    text: String,
    unix_millis: i64,
}

impl Serialize for CanonicalInstant {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for CanonicalInstant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

impl CanonicalInstant {
    pub fn from_unix_millis(unix_millis: i64) -> Result<Self, AuthorityError> {
        let instant = DateTime::<Utc>::from_timestamp_millis(unix_millis)
            .ok_or(AuthorityError::InvalidInput("service timestamp"))?;
        Ok(Self {
            text: instant.to_rfc3339_opts(SecondsFormat::Millis, true),
            unix_millis,
        })
    }

    pub fn parse(value: &str) -> Result<Self, AuthorityError> {
        let parsed = DateTime::parse_from_rfc3339(value)
            .map_err(|_| AuthorityError::InvalidInput("canonical timestamp"))?
            .with_timezone(&Utc);
        let normalized = parsed.to_rfc3339_opts(SecondsFormat::Millis, true);
        if normalized != value {
            return Err(AuthorityError::InvalidInput("canonical timestamp"));
        }
        Self::from_unix_millis(parsed.timestamp_millis())
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn unix_millis(&self) -> i64 {
        self.unix_millis
    }

    pub(crate) fn validate(&self) -> Result<(), AuthorityError> {
        let canonical = Self::from_unix_millis(self.unix_millis)
            .map_err(|_| AuthorityError::CorruptState("service timestamp"))?;
        if canonical.text != self.text {
            return Err(AuthorityError::CorruptState("service timestamp"));
        }
        Ok(())
    }
}

macro_rules! wire_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $wire)] $variant),+
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }

        impl FromStr for $name {
            type Err = AuthorityError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($wire => Ok(Self::$variant)),+,
                    _ => Err(AuthorityError::CorruptState(stringify!($name))),
                }
            }
        }
    };
}

wire_enum!(EventKind {
    ClaimRegistered => "claim-registered-v2",
    RunnerLeaseGranted => "runner-lease-granted-v2",
    AttemptStartCommitted => "capture-attempt-start-committed-v2",
    RunTerminal => "capture-run-terminal-v2",
    AttemptTerminal => "capture-attempt-terminal-v2",
    FinalWitness => "capture-final-witness-v2",
});

wire_enum!(RunPhase {
    Registered => "registered",
    Leased => "leased",
    AttemptStarted => "attempt-started",
    PreStartTerminal => "pre-start-terminal",
    CandidateSuccessAwaitingFinal => "candidate-success-awaiting-final",
    FailedFinalOptional => "failed-final-optional",
    FinalWitnessed => "final-witnessed",
});

wire_enum!(RunTerminalStage {
    Registration => "registration",
    PreLease => "pre-lease",
    LeasedPreStart => "leased-pre-start",
});

wire_enum!(RunTerminalOutcome {
    RegistrationChangedReplay => "registration-changed-replay-v2",
    RegistrationAuthenticatedDenial => "registration-authenticated-denial-v2",
    PreLeaseAdmissionFailed => "pre-lease-admission-failed-v2",
    PreLeasePreReviewFailed => "pre-lease-pre-review-failed-v2",
    PreLeaseRunnerUnavailable => "pre-lease-runner-unavailable-v2",
    PreLeasePolicyFailed => "pre-lease-policy-failed-v2",
    PreLeaseInternalFailure => "pre-lease-internal-failure-v2",
    LeasedPreStartExpired => "leased-pre-start-expired-v2",
    LeasedPreStartAdmissionRevoked => "leased-pre-start-admission-revoked-v2",
    LeasedPreStartPreflightFailed => "leased-pre-start-preflight-failed-v2",
    LeasedPreStartInternalFailure => "leased-pre-start-internal-failure-v2",
});

wire_enum!(AttemptOutcome {
    CandidateComplete => "capture-candidate-complete-v2",
    ProcessFailed => "process-failed-v2",
    Timeout => "attempt-timeout-v2",
    RunnerLost => "runner-lost-v2",
    OutputMissing => "output-missing-v2",
    OutputInvalid => "output-invalid-v2",
    CleanupFailed => "cleanup-failed-v2",
    EgressViolation => "egress-violation-v2",
    FenceInvalidated => "fence-invalidated-v2",
    InternalFailure => "attempt-internal-failure-v2",
});

wire_enum!(ProcessDisposition {
    ExitedZero => "exited-zero",
    ExitedNonzero => "exited-nonzero",
    Terminated => "terminated",
    RunnerLost => "runner-lost",
    Unknown => "unknown",
});

wire_enum!(EgressDisposition {
    IsolatedNoViolation => "isolated-no-violation",
    ViolationDetected => "violation-detected",
    Unknown => "unknown",
});

wire_enum!(LeaseDisposition {
    SpentNeverReusable => "spent-never-reusable",
});

wire_enum!(ResourceDisposition {
    HeldPreStart => "held-pre-start",
    AttemptStarted => "attempt-started",
    ReleasedUnstarted => "released-unstarted",
    ReleasedAfterCleanup => "released-after-cleanup",
    Quarantined => "quarantined",
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeferredCapability {
    HttpMtlsTransport,
    ServiceSigner,
    TransparencyLog,
    CheckpointWitnessQuorum,
    SemanticWitnessQuorum,
    ControlledRunnerLaunch,
}

pub const UNAVAILABLE_CAPABILITIES: &[DeferredCapability] = &[
    DeferredCapability::HttpMtlsTransport,
    DeferredCapability::ServiceSigner,
    DeferredCapability::TransparencyLog,
    DeferredCapability::CheckpointWitnessQuorum,
    DeferredCapability::SemanticWitnessQuorum,
    DeferredCapability::ControlledRunnerLaunch,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_and_state_vocabularies_are_closed() {
        let events = [
            (EventKind::ClaimRegistered, "claim-registered-v2"),
            (EventKind::RunnerLeaseGranted, "runner-lease-granted-v2"),
            (
                EventKind::AttemptStartCommitted,
                "capture-attempt-start-committed-v2",
            ),
            (EventKind::RunTerminal, "capture-run-terminal-v2"),
            (EventKind::AttemptTerminal, "capture-attempt-terminal-v2"),
            (EventKind::FinalWitness, "capture-final-witness-v2"),
        ];
        for (value, wire) in events {
            assert_eq!(value.as_str(), wire);
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{wire}\"")
            );
            assert_eq!(wire.parse::<EventKind>().unwrap(), value);
        }
        assert!("claim-registered-v3".parse::<EventKind>().is_err());
        assert!(serde_json::from_str::<RunPhase>("\"retrying\"").is_err());
        assert_eq!(
            LeaseDisposition::SpentNeverReusable.as_str(),
            "spent-never-reusable"
        );
        assert!(serde_json::from_str::<ResourceDisposition>("\"released\"").is_err());
    }

    #[test]
    fn authority_scalars_reject_noncanonical_deserialization() {
        assert!(serde_json::from_str::<Digest>("\"ABCDEF\"").is_err());
        assert!(serde_json::from_str::<Digest>(
            "\"0000000000000000000000000000000000000000000000000000000000000000\""
        )
        .is_err());
        assert!(serde_json::from_str::<OpaqueId>("\"../escape\"").is_err());
        assert!(
            serde_json::from_str::<CanonicalInstant>("\"2026-08-29T14:00:00.000+02:00\"").is_err()
        );
    }

    #[test]
    fn canonical_instant_is_a_wire_string_not_an_internal_object() {
        let instant = CanonicalInstant::parse("2026-08-29T12:00:00.000Z").unwrap();
        assert_eq!(
            serde_json::to_string(&instant).unwrap(),
            "\"2026-08-29T12:00:00.000Z\""
        );
        assert_eq!(
            serde_json::from_str::<CanonicalInstant>("\"2026-08-29T12:00:00.000Z\"").unwrap(),
            instant
        );
    }
}

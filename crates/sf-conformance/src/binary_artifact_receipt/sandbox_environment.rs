//! One typed authority for both sandbox execution and environment attestation.

use std::ffi::OsString;

use sha2::{Digest, Sha256};

use super::model::ENVIRONMENT_DIGEST_FORMAT;

const NAMES: [&str; 11] = [
    "CARGO_HOME",
    "CARGO_INCREMENTAL",
    "CARGO_NET_OFFLINE",
    "HOME",
    "LC_ALL",
    "PATH",
    "RUSTC",
    "RUSTUP_HOME",
    "SOURCE_DATE_EPOCH",
    "TMPDIR",
    "TZ",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExactBuildEnvironment {
    values: Vec<(&'static str, String)>,
}

impl ExactBuildEnvironment {
    pub(super) fn new(source_date_epoch: u64) -> Result<Self, String> {
        if source_date_epoch == 0 || source_date_epoch > i64::MAX as u64 {
            return Err("SOURCE_DATE_EPOCH is outside supported Unix timestamp bounds".to_owned());
        }
        let values = vec![
            ("CARGO_HOME", "/cargo-home".to_owned()),
            ("CARGO_INCREMENTAL", "0".to_owned()),
            ("CARGO_NET_OFFLINE", "true".to_owned()),
            ("HOME", "/home/harness".to_owned()),
            ("LC_ALL", "C".to_owned()),
            ("PATH", "/toolchain/bin:/usr/bin".to_owned()),
            ("RUSTC", "/toolchain/bin/rustc".to_owned()),
            ("RUSTUP_HOME", "/toolchain".to_owned()),
            ("SOURCE_DATE_EPOCH", source_date_epoch.to_string()),
            ("TMPDIR", "/tmp".to_owned()),
            ("TZ", "UTC".to_owned()),
        ];
        if values.iter().map(|(name, _)| *name).collect::<Vec<_>>() != NAMES {
            return Err("exact build environment names are not canonical".to_owned());
        }
        Ok(Self { values })
    }

    pub(super) fn append_bwrap_arguments(&self, arguments: &mut Vec<OsString>) {
        for (name, value) in &self.values {
            arguments.extend([OsString::from("--setenv"), OsString::from(name)]);
            arguments.push(value.into());
        }
    }

    pub(super) fn sha256(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(ENVIRONMENT_DIGEST_FORMAT.as_bytes());
        digest.update([0]);
        for (name, value) in &self.values {
            digest.update(name.as_bytes());
            digest.update([0]);
            digest.update(value.as_bytes());
            digest.update([0]);
        }
        format!("{:x}", digest.finalize())
    }

    #[cfg(test)]
    fn records(&self) -> String {
        self.values
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("|")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_values_and_digest_are_exact_and_epoch_bound() {
        let first = ExactBuildEnvironment::new(1_700_000_000).unwrap();
        assert_eq!(first.records(), "CARGO_HOME=/cargo-home|CARGO_INCREMENTAL=0|CARGO_NET_OFFLINE=true|HOME=/home/harness|LC_ALL=C|PATH=/toolchain/bin:/usr/bin|RUSTC=/toolchain/bin/rustc|RUSTUP_HOME=/toolchain|SOURCE_DATE_EPOCH=1700000000|TMPDIR=/tmp|TZ=UTC");
        assert_eq!(first.sha256().len(), 64);
        assert_ne!(
            first.sha256(),
            ExactBuildEnvironment::new(1_700_000_001).unwrap().sha256()
        );
        assert!(ExactBuildEnvironment::new(0).is_err());
    }
}

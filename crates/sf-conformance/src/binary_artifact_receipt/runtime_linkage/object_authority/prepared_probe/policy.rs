use std::path::{Path, PathBuf};

use crate::binary_artifact_receipt::runtime_elf::RuntimeElfPolicyIdentity;

const MAX_POLICY_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct ExpectedBwrapIdentity {
    pub(super) path: PathBuf,
    pub(super) sha256: String,
    pub(super) byte_length: u64,
    pub(super) executable_policy: String,
}

impl ExpectedBwrapIdentity {
    pub(in super::super) fn new(
        path: &Path,
        sha256: &str,
        byte_length: u64,
        executable_policy: &str,
    ) -> Result<Self, String> {
        super::super::super::validate_absolute_path(path, "expected bubblewrap executable")?;
        super::super::super::validate_sha256(sha256)?;
        validate_policy_id(executable_policy, "expected bubblewrap executable policy")?;
        if byte_length == 0 {
            return Err("expected bubblewrap identity is outside policy".to_owned());
        }
        Ok(Self {
            path: path.to_path_buf(),
            sha256: sha256.to_owned(),
            byte_length,
            executable_policy: executable_policy.to_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct ExpectedRuntimeElfPolicy {
    id: String,
    sha256: String,
}

impl ExpectedRuntimeElfPolicy {
    pub(in super::super) fn new(id: &str, sha256: &str) -> Result<Self, String> {
        validate_policy_id(id, "expected runtime ELF policy")?;
        super::super::super::validate_sha256(sha256)?;
        Ok(Self {
            id: id.to_owned(),
            sha256: sha256.to_owned(),
        })
    }

    pub(super) fn assert_matches(&self, actual: &RuntimeElfPolicyIdentity) -> Result<(), String> {
        if self.id != actual.id() || self.sha256 != actual.sha256() {
            Err("prepared runtime ELF policy differs from pinned expectation".to_owned())
        } else {
            Ok(())
        }
    }
}

fn validate_policy_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_POLICY_ID_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-._".contains(&byte)
        })
    {
        Err(format!("{label} is outside policy"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_policy_expectation_is_bounded_and_canonical() {
        assert!(ExpectedRuntimeElfPolicy::new("policy-v1", &"0".repeat(64)).is_ok());
        for id in ["", "Policy-v1", "policy v1"] {
            assert!(ExpectedRuntimeElfPolicy::new(id, &"0".repeat(64)).is_err());
        }
        assert!(ExpectedRuntimeElfPolicy::new("policy-v1", &"A".repeat(64)).is_err());
    }
}

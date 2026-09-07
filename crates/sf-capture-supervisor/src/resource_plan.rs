// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::BTreeMap;

use crate::{
    AuthorityError, Digest, GrantLease, LeaseState, ResourceDisposition, ResourceOwner,
    ResourcePriorState, ResourceState, ResourceTransition, RunKey, RESOURCE_CONFLICT_SET_DOMAIN_V2,
};

pub(crate) fn lease_resource_transition(
    command: &GrantLease,
    resources: &[ResourceState],
) -> Result<ResourceTransition, AuthorityError> {
    let by_id: BTreeMap<_, _> = resources
        .iter()
        .map(|resource| (resource.resource_id.clone(), resource))
        .collect();
    if by_id.len() != resources.len() || by_id.keys().any(|id| !command.resource_ids.contains(id)) {
        return Err(AuthorityError::CorruptState("resource snapshot"));
    }
    for state in by_id.values() {
        if !state.reusable() {
            return Err(AuthorityError::ResourceUnavailable);
        }
    }
    let prior_max = by_id.values().map(|state| state.fence).max().unwrap_or(0);
    let fence = prior_max
        .checked_add(1)
        .ok_or(AuthorityError::CorruptState("resource fence overflow"))?;
    Ok(ResourceTransition {
        runner_enrollment_record_digest: command.runner.enrollment_record_digest.clone(),
        physical_parent_id: command.physical_parent_id.clone(),
        conflict_set_digest: conflict_set_digest(command)?,
        fence,
        members: command
            .resource_ids
            .iter()
            .map(|resource_id| {
                let prior = by_id.get(resource_id).copied();
                ResourcePriorState {
                    resource_id: resource_id.clone(),
                    event_digest: prior.map(|value| value.last_event_digest.clone()),
                    fence: prior.map(|value| value.fence),
                }
            })
            .collect(),
    })
}

fn conflict_set_digest(command: &GrantLease) -> Result<Digest, AuthorityError> {
    let value = serde_json::json!({
        "domain": RESOURCE_CONFLICT_SET_DOMAIN_V2,
        "runnerEnrollmentRecordDigest": command.runner.enrollment_record_digest,
        "physicalParentId": command.physical_parent_id,
        "resourceIds": command.resource_ids,
    });
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| AuthorityError::CorruptState("conflict-set digest"))?;
    Ok(Digest::sha256(bytes))
}

pub(crate) fn continuation_resource_transition(
    key: &RunKey,
    lease: &LeaseState,
    resources: &[ResourceState],
    expected_disposition: ResourceDisposition,
) -> Result<ResourceTransition, AuthorityError> {
    if resources.len() != lease.resource_ids.len() {
        return Err(AuthorityError::CorruptState("resource continuation count"));
    }
    let owner = ResourceOwner::from(key);
    let mut members = Vec::with_capacity(resources.len());
    for (expected_id, resource) in lease.resource_ids.iter().zip(resources) {
        if &resource.resource_id != expected_id
            || resource.fence != lease.fence
            || resource.owner.as_ref() != Some(&owner)
            || resource.disposition != expected_disposition
        {
            return Err(AuthorityError::CorruptState("resource continuation"));
        }
        members.push(ResourcePriorState {
            resource_id: resource.resource_id.clone(),
            event_digest: Some(resource.last_event_digest.clone()),
            fence: Some(resource.fence),
        });
    }
    Ok(ResourceTransition {
        runner_enrollment_record_digest: lease.runner_enrollment_record_digest.clone(),
        physical_parent_id: lease.physical_parent_id.clone(),
        conflict_set_digest: lease.conflict_set_digest.clone(),
        fence: lease.fence,
        members,
    })
}

pub(crate) fn transition_resources(
    transition: &ResourceTransition,
    key: &RunKey,
    event_digest: &Digest,
    disposition: ResourceDisposition,
) -> Vec<ResourceState> {
    transition
        .members
        .iter()
        .map(|member| ResourceState {
            resource_id: member.resource_id.clone(),
            fence: transition.fence,
            last_event_digest: event_digest.clone(),
            disposition,
            owner: Some(ResourceOwner::from(key)),
        })
        .collect()
}

pub(crate) fn continue_resources(
    resources: &[ResourceState],
    event_digest: &Digest,
    disposition: ResourceDisposition,
) -> Vec<ResourceState> {
    let keep_owner = matches!(
        disposition,
        ResourceDisposition::HeldPreStart | ResourceDisposition::AttemptStarted
    );
    resources
        .iter()
        .map(|resource| ResourceState {
            resource_id: resource.resource_id.clone(),
            fence: resource.fence,
            last_event_digest: event_digest.clone(),
            disposition,
            owner: if keep_owner {
                resource.owner.clone()
            } else {
                None
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpaqueId, ReviewPair, RunnerIdentity};

    fn digest(value: &str) -> Digest {
        Digest::sha256(value)
    }

    #[test]
    fn conflict_set_digest_matches_node_oracle_vector() {
        let command = GrantLease {
            registration_event_digest: digest("registration"),
            claim_digest: digest("claim"),
            admission_challenge_digest: digest("challenge"),
            admission_evidence_digest: digest("admission"),
            runner: RunnerIdentity {
                runner_id: OpaqueId::parse("runner_20260829").unwrap(),
                enrollment_record_digest: digest("runner-enrollment-record"),
                session_id: OpaqueId::parse("session_20260829").unwrap(),
                boot_id: OpaqueId::parse("boot_id_20260829").unwrap(),
                key_epoch: 1,
                key_fingerprint: digest("key"),
                possession_proof_digest: digest("proof"),
            },
            host_evidence_digest: digest("host"),
            runner_profile_digest: digest("profile"),
            control_policy_digest: digest("policy"),
            pre_review: ReviewPair {
                codex_receipt_digest: digest("codex"),
                claude_receipt_digest: digest("claude"),
            },
            lease_id: OpaqueId::parse("lease_20260829").unwrap(),
            lease_duration_millis: 1_000,
            physical_parent_id: OpaqueId::parse("numa_parent_20260829").unwrap(),
            resource_ids: vec![
                OpaqueId::parse("numa_parent_20260829").unwrap(),
                OpaqueId::parse("resource_cpu_20260829").unwrap(),
            ],
        };
        assert_eq!(
            conflict_set_digest(&command).unwrap().as_str(),
            "c6b084c78aef497bf24503de4284d174a74dce90feea25f56c7dd80bc1caa5eb"
        );
    }
}

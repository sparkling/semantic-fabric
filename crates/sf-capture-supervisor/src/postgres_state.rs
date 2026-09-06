// SPDX-License-Identifier: MIT OR Apache-2.0

use deadpool_postgres::{Object, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use tokio_postgres::{IsolationLevel, Row};

use crate::{AuthorityError, AuthorityRequest, Digest, OpaqueId, Operation, RunState, StoredEvent};

pub(crate) fn requested_resource_ids(
    request: &AuthorityRequest,
    run: Option<&RunState>,
) -> Vec<OpaqueId> {
    match &request.operation {
        Operation::GrantLease(value) => value.resource_ids.clone(),
        Operation::StartAttempt(_)
        | Operation::PreStartTerminal(_)
        | Operation::AttemptTerminal(_) => run
            .and_then(|value| value.lease.as_ref())
            .map(|lease| lease.resource_ids.clone())
            .unwrap_or_default(),
        Operation::Register(_) | Operation::FinalWitness(_) => Vec::new(),
    }
}

pub(crate) async fn serializable<'a>(
    client: &'a mut Object,
    read_only: bool,
) -> Result<Transaction<'a>, AuthorityError> {
    client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .read_only(read_only)
        .deferrable(read_only)
        .start()
        .await
        .map_err(pg)
}

pub(crate) fn decode_state<T: DeserializeOwned>(
    row: &Row,
    bytes_index: usize,
    digest_index: usize,
) -> Result<T, AuthorityError> {
    let state_bytes = bytes(row, bytes_index)?;
    let stored_digest = Digest::parse(text(row, digest_index)?)?;
    if Digest::sha256(&state_bytes) != stored_digest {
        return Err(AuthorityError::CorruptState("state digest"));
    }
    serde_json::from_slice(&state_bytes).map_err(|_| AuthorityError::CorruptState("state JSON"))
}

pub(crate) async fn persist_outbox(
    tx: &Transaction<'_>,
    authority: &Digest,
    event: &StoredEvent,
) -> Result<(), AuthorityError> {
    let global = event.global_sequence.to_string();
    tx.execute(
        "INSERT INTO sf_capture_authority_v1.semantic_outbox \
         (authority_digest, global_sequence, event_digest, event_envelope) \
         VALUES ($1, $2::text::numeric, $3, $4)",
        &[
            &authority.as_str(),
            &global.as_str(),
            &event.event_digest.as_str(),
            &event.event_envelope,
        ],
    )
    .await
    .map_err(pg)?;
    Ok(())
}

pub(crate) async fn advance_authority_head(
    tx: &Transaction<'_>,
    authority: &Digest,
    current: u64,
) -> Result<(), AuthorityError> {
    let next = current
        .checked_add(1)
        .ok_or(AuthorityError::CorruptState("global sequence overflow"))?;
    let next_text = next.to_string();
    let current_text = current.to_string();
    let changed = tx
        .execute(
            "UPDATE sf_capture_authority_v1.authority_heads \
             SET next_global_sequence = $2::text::numeric WHERE authority_digest = $1 \
             AND next_global_sequence = $3::text::numeric",
            &[
                &authority.as_str(),
                &next_text.as_str(),
                &current_text.as_str(),
            ],
        )
        .await
        .map_err(pg)?;
    if changed != 1 {
        return Err(AuthorityError::CorruptState(
            "authority head compare-and-set",
        ));
    }
    Ok(())
}

pub(crate) fn encode_state<T: Serialize>(value: &T) -> Result<(Vec<u8>, Digest), AuthorityError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| AuthorityError::CorruptState("state serialization"))?;
    let digest = Digest::sha256(&bytes);
    Ok((bytes, digest))
}

pub(crate) fn parse_u64(value: &str, label: &'static str) -> Result<u64, AuthorityError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || value.bytes().any(|byte| !byte.is_ascii_digit())
    {
        return Err(AuthorityError::CorruptState(label));
    }
    value
        .parse()
        .map_err(|_| AuthorityError::CorruptState(label))
}

pub(crate) fn text(row: &Row, index: usize) -> Result<&str, AuthorityError> {
    row.try_get(index).map_err(pg)
}

pub(crate) fn bytes(row: &Row, index: usize) -> Result<Vec<u8>, AuthorityError> {
    row.try_get(index).map_err(pg)
}

pub(crate) fn pg(_error: tokio_postgres::Error) -> AuthorityError {
    AuthorityError::Postgres("database operation failed")
}

pub(crate) fn pg_pool(_error: deadpool_postgres::PoolError) -> AuthorityError {
    AuthorityError::Postgres("connection pool unavailable")
}

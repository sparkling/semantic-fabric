// SPDX-License-Identifier: MIT OR Apache-2.0

use std::str::FromStr;

use async_trait::async_trait;
use deadpool_postgres::{Pool, Transaction};
use tokio_postgres::Row;

use crate::postgres_binding::PrimaryBinding;
use crate::postgres_state::{
    advance_authority_head, bytes, decode_state, encode_state, parse_u64, persist_outbox, pg,
    pg_pool, requested_resource_ids, serializable, text,
};
use crate::{
    exact_match,
    planner::{finalize_transition, plan_transition},
    validate_materialized, ApplyOutcome, AuthorityError, AuthorityRequest, AuthorityStore, Digest,
    EventKind, EventMaterializerPort, ExactResult, OpaqueId, Operation, ResourceDisposition,
    ResourceState, RunPhase, RunState, StoredEvent,
};

pub const MIGRATION_0001: &str = include_str!("../migrations/0001_authority_v1.sql");

pub struct PostgresAuthorityStore {
    authority_digest: Digest,
    writer_pool: Pool,
    recovery_pool: Pool,
    primary_binding: PrimaryBinding,
}

impl PostgresAuthorityStore {
    pub async fn bind(
        authority_digest: Digest,
        writer_pool: Pool,
        recovery_pool: Pool,
    ) -> Result<Self, AuthorityError> {
        let primary_binding = PrimaryBinding::from_pools(&writer_pool, &recovery_pool).await?;
        Ok(Self {
            authority_digest,
            writer_pool,
            recovery_pool,
            primary_binding,
        })
    }

    pub async fn migrate(pool: &Pool) -> Result<(), AuthorityError> {
        let client = pool.get().await.map_err(pg_pool)?;
        client.batch_execute(MIGRATION_0001).await.map_err(pg)?;
        Ok(())
    }

    /// Provisioning is explicit and idempotent. Ordinary mutation never
    /// creates an authority head implicitly.
    pub async fn provision_authority(&self) -> Result<(), AuthorityError> {
        let mut client = self.writer_pool.get().await.map_err(pg_pool)?;
        self.primary_binding.verify(&client).await?;
        let tx = serializable(&mut client, false).await?;
        tx.execute(
            "INSERT INTO sf_capture_authority_v1.authority_heads \
             (authority_digest, next_global_sequence) VALUES ($1, 1) \
             ON CONFLICT (authority_digest) DO NOTHING",
            &[&self.authority_digest.as_str()],
        )
        .await
        .map_err(pg)?;
        let row = tx
            .query_one(
                "SELECT next_global_sequence::text FROM \
                 sf_capture_authority_v1.authority_heads WHERE authority_digest = $1 FOR UPDATE",
                &[&self.authority_digest.as_str()],
            )
            .await
            .map_err(pg)?;
        parse_u64(row.try_get::<_, &str>(0).map_err(pg)?, "authority head")?;
        tx.commit()
            .await
            .map_err(|_| AuthorityError::CommitOutcomeUnknown)
    }

    async fn apply_transaction(
        &self,
        request: &AuthorityRequest,
        materializer: &dyn EventMaterializerPort,
    ) -> Result<ApplyOutcome, AuthorityError> {
        let mut client = self.writer_pool.get().await.map_err(pg_pool)?;
        self.primary_binding.verify(&client).await?;
        let tx = serializable(&mut client, false).await?;
        if let Some(stored) = lookup_exact(&tx, &self.authority_digest, request).await? {
            let result = exact_match(&stored, request)?;
            tx.commit()
                .await
                .map_err(|_| AuthorityError::CommitOutcomeUnknown)?;
            return Ok(ApplyOutcome::Recovered(result));
        }

        let next_global_sequence = lock_authority_head(&tx, &self.authority_digest).await?;
        lock_run_slot(&tx, &self.authority_digest, request).await?;
        if let Some(stored) = lookup_exact(&tx, &self.authority_digest, request).await? {
            let result = exact_match(&stored, request)?;
            tx.commit()
                .await
                .map_err(|_| AuthorityError::CommitOutcomeUnknown)?;
            return Ok(ApplyOutcome::Recovered(result));
        }

        let run = load_run(&tx, &self.authority_digest, request).await?;
        let resource_ids = requested_resource_ids(request, run.as_ref());
        let resources = lock_and_load_resources(
            &tx,
            &self.authority_digest,
            &resource_ids,
            matches!(request.operation, Operation::GrantLease(_)),
        )
        .await?;
        let now_millis: i64 = tx
            .query_one(
                "SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint",
                &[],
            )
            .await
            .map_err(pg)?
            .try_get(0)
            .map_err(pg)?;
        let plan = plan_transition(
            request,
            run.as_ref(),
            &resources,
            next_global_sequence,
            now_millis,
        )?;
        let materialized = materializer.materialize(&plan.proposal).await?;
        let stored = validate_materialized(request, &plan.proposal, materialized)?;
        let (next_run, next_resources) = finalize_transition(&plan, &stored.event_digest)?;
        persist_event(&tx, &self.authority_digest, &stored).await?;
        persist_run(&tx, &self.authority_digest, &next_run).await?;
        for resource in &next_resources {
            persist_resource(&tx, &self.authority_digest, resource).await?;
        }
        persist_outbox(&tx, &self.authority_digest, &stored).await?;
        advance_authority_head(&tx, &self.authority_digest, next_global_sequence).await?;
        tx.commit()
            .await
            .map_err(|_| AuthorityError::CommitOutcomeUnknown)?;
        Ok(ApplyOutcome::Committed(stored.exact_result()))
    }
}

#[async_trait]
impl AuthorityStore for PostgresAuthorityStore {
    async fn apply(
        &self,
        request: &AuthorityRequest,
        materializer: &dyn EventMaterializerPort,
    ) -> Result<ApplyOutcome, AuthorityError> {
        request.validate()?;
        match self.apply_transaction(request, materializer).await {
            Ok(outcome) => Ok(outcome),
            Err(error @ AuthorityError::Postgres(_))
            | Err(error @ AuthorityError::CommitOutcomeUnknown) => {
                match self.recover_exact(request).await {
                    Ok(result) => Ok(ApplyOutcome::Recovered(result)),
                    Err(_) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn recover_exact(
        &self,
        request: &AuthorityRequest,
    ) -> Result<ExactResult, AuthorityError> {
        request.validate()?;
        let mut client = self.recovery_pool.get().await.map_err(pg_pool)?;
        self.primary_binding.verify(&client).await?;
        let tx = serializable(&mut client, true).await?;
        let stored = lookup_exact(&tx, &self.authority_digest, request)
            .await?
            .ok_or(AuthorityError::ExactResultMissing)?;
        let result = exact_match(&stored, request)?;
        tx.commit()
            .await
            .map_err(|_| AuthorityError::CommitOutcomeUnknown)?;
        Ok(result)
    }
}

async fn lock_authority_head(
    tx: &Transaction<'_>,
    authority: &Digest,
) -> Result<u64, AuthorityError> {
    let row = tx
        .query_opt(
            "SELECT next_global_sequence::text FROM sf_capture_authority_v1.authority_heads \
             WHERE authority_digest = $1 FOR UPDATE",
            &[&authority.as_str()],
        )
        .await
        .map_err(pg)?
        .ok_or(AuthorityError::AuthorityNotProvisioned)?;
    parse_u64(row.try_get::<_, &str>(0).map_err(pg)?, "global sequence")
}

async fn lock_run_slot(
    tx: &Transaction<'_>,
    authority: &Digest,
    request: &AuthorityRequest,
) -> Result<(), AuthorityError> {
    if matches!(request.operation, Operation::Register(_)) {
        tx.execute(
            "INSERT INTO sf_capture_authority_v1.run_slots \
             (authority_digest, project_authority_digest, run_id) VALUES ($1, $2, $3) \
             ON CONFLICT (authority_digest, project_authority_digest, run_id) DO NOTHING",
            &[
                &authority.as_str(),
                &request.project_authority_digest.as_str(),
                &request.run_id.as_str(),
            ],
        )
        .await
        .map_err(pg)?;
    }
    let found = tx
        .query_opt(
            "SELECT run_id FROM sf_capture_authority_v1.run_slots \
             WHERE authority_digest = $1 AND project_authority_digest = $2 AND run_id = $3 \
             FOR UPDATE",
            &[
                &authority.as_str(),
                &request.project_authority_digest.as_str(),
                &request.run_id.as_str(),
            ],
        )
        .await
        .map_err(pg)?;
    if found.is_none() {
        return Err(AuthorityError::UnknownRun);
    }
    Ok(())
}

async fn load_run(
    tx: &Transaction<'_>,
    authority: &Digest,
    request: &AuthorityRequest,
) -> Result<Option<RunState>, AuthorityError> {
    let row = tx
        .query_opt(
            "SELECT phase, last_run_sequence, state_bytes, state_sha256 FROM \
             sf_capture_authority_v1.runs WHERE authority_digest = $1 \
             AND project_authority_digest = $2 AND run_id = $3",
            &[
                &authority.as_str(),
                &request.project_authority_digest.as_str(),
                &request.run_id.as_str(),
            ],
        )
        .await
        .map_err(pg)?;
    row.map(decode_run).transpose()
}

async fn lock_and_load_resources(
    tx: &Transaction<'_>,
    authority: &Digest,
    resource_ids: &[OpaqueId],
    create_slots: bool,
) -> Result<Vec<ResourceState>, AuthorityError> {
    let mut states = Vec::with_capacity(resource_ids.len());
    for resource_id in resource_ids {
        if create_slots {
            tx.execute(
                "INSERT INTO sf_capture_authority_v1.resource_slots \
                 (authority_digest, resource_id) VALUES ($1, $2) \
                 ON CONFLICT (authority_digest, resource_id) DO NOTHING",
                &[&authority.as_str(), &resource_id.as_str()],
            )
            .await
            .map_err(pg)?;
        }
        let slot = tx
            .query_opt(
                "SELECT resource_id FROM sf_capture_authority_v1.resource_slots \
                 WHERE authority_digest = $1 AND resource_id = $2 FOR UPDATE",
                &[&authority.as_str(), &resource_id.as_str()],
            )
            .await
            .map_err(pg)?;
        if slot.is_none() {
            return Err(AuthorityError::CorruptState("missing resource slot"));
        }
        if let Some(row) = tx
            .query_opt(
                "SELECT fence::text, disposition, owner_project_authority_digest, \
                 owner_run_id, state_bytes, state_sha256 FROM \
                 sf_capture_authority_v1.resources \
                 WHERE authority_digest = $1 AND resource_id = $2",
                &[&authority.as_str(), &resource_id.as_str()],
            )
            .await
            .map_err(pg)?
        {
            states.push(decode_resource(row)?);
        }
    }
    Ok(states)
}

async fn lookup_exact(
    tx: &Transaction<'_>,
    authority: &Digest,
    request: &AuthorityRequest,
) -> Result<Option<StoredEvent>, AuthorityError> {
    tx.query_opt(
        "SELECT project_authority_digest, run_id, semantic_request_digest, canonical_request, \
         canonical_request_sha256, operation_digest, proposal_digest, event_kind, event_digest, \
         global_sequence::text, run_sequence, event_envelope, response_bytes, response_sha256 \
         FROM sf_capture_authority_v1.events WHERE authority_digest = $1 \
         AND project_authority_digest = $2 AND semantic_request_digest = $3",
        &[
            &authority.as_str(),
            &request.project_authority_digest.as_str(),
            &request.semantic_request_digest.as_str(),
        ],
    )
    .await
    .map_err(pg)?
    .map(decode_event)
    .transpose()
}

fn decode_event(row: Row) -> Result<StoredEvent, AuthorityError> {
    Ok(StoredEvent {
        project_authority_digest: Digest::parse(text(&row, 0)?)?,
        run_id: OpaqueId::parse(text(&row, 1)?)?,
        semantic_request_digest: Digest::parse(text(&row, 2)?)?,
        canonical_request: bytes(&row, 3)?,
        canonical_request_sha256: Digest::parse(text(&row, 4)?)?,
        operation_digest: Digest::parse(text(&row, 5)?)?,
        proposal_digest: Digest::parse(text(&row, 6)?)?,
        event_kind: EventKind::from_str(text(&row, 7)?)?,
        event_digest: Digest::parse(text(&row, 8)?)?,
        global_sequence: parse_u64(text(&row, 9)?, "stored global sequence")?,
        run_sequence: u64::try_from(row.try_get::<_, i16>(10).map_err(pg)?)
            .map_err(|_| AuthorityError::CorruptState("stored run sequence"))?,
        event_envelope: bytes(&row, 11)?,
        response_bytes: bytes(&row, 12)?,
        response_sha256: Digest::parse(text(&row, 13)?)?,
    })
}

fn decode_run(row: Row) -> Result<RunState, AuthorityError> {
    let phase = RunPhase::from_str(text(&row, 0)?)?;
    let sequence = u64::try_from(row.try_get::<_, i16>(1).map_err(pg)?)
        .map_err(|_| AuthorityError::CorruptState("run sequence"))?;
    let value: RunState = decode_state(&row, 2, 3)?;
    if value.phase != phase || value.last_run_sequence != sequence {
        return Err(AuthorityError::CorruptState("run row projection"));
    }
    value.validate()?;
    Ok(value)
}

fn decode_resource(row: Row) -> Result<ResourceState, AuthorityError> {
    let fence = parse_u64(text(&row, 0)?, "resource fence")?;
    let disposition = ResourceDisposition::from_str(text(&row, 1)?)?;
    let owner_project: Option<&str> = row.try_get(2).map_err(pg)?;
    let owner_run: Option<&str> = row.try_get(3).map_err(pg)?;
    if owner_project.is_some() != owner_run.is_some() {
        return Err(AuthorityError::CorruptState("resource owner columns"));
    }
    let value: ResourceState = decode_state(&row, 4, 5)?;
    let owner_matches = match (&value.owner, owner_project, owner_run) {
        (None, None, None) => true,
        (Some(owner), Some(project), Some(run)) => {
            owner.project_authority_digest.as_str() == project && owner.run_id.as_str() == run
        }
        _ => false,
    };
    if value.fence != fence || value.disposition != disposition || !owner_matches {
        return Err(AuthorityError::CorruptState("resource row projection"));
    }
    value.validate()?;
    Ok(value)
}

async fn persist_event(
    tx: &Transaction<'_>,
    authority: &Digest,
    event: &StoredEvent,
) -> Result<(), AuthorityError> {
    let global = event.global_sequence.to_string();
    let run_sequence = i16::try_from(event.run_sequence)
        .map_err(|_| AuthorityError::CorruptState("run sequence"))?;
    tx.execute(
        "INSERT INTO sf_capture_authority_v1.events \
         (authority_digest, project_authority_digest, run_id, global_sequence, run_sequence, \
          event_kind, semantic_request_digest, canonical_request, canonical_request_sha256, \
          operation_digest, proposal_digest, event_digest, event_envelope, response_bytes, \
          response_sha256) VALUES ($1, $2, $3, $4::text::numeric, $5, $6, $7, $8, $9, \
          $10, $11, $12, $13, $14, $15)",
        &[
            &authority.as_str(),
            &event.project_authority_digest.as_str(),
            &event.run_id.as_str(),
            &global.as_str(),
            &run_sequence,
            &event.event_kind.as_str(),
            &event.semantic_request_digest.as_str(),
            &event.canonical_request,
            &event.canonical_request_sha256.as_str(),
            &event.operation_digest.as_str(),
            &event.proposal_digest.as_str(),
            &event.event_digest.as_str(),
            &event.event_envelope,
            &event.response_bytes,
            &event.response_sha256.as_str(),
        ],
    )
    .await
    .map_err(pg)?;
    Ok(())
}

async fn persist_run(
    tx: &Transaction<'_>,
    authority: &Digest,
    run: &RunState,
) -> Result<(), AuthorityError> {
    let (state_bytes, state_sha256) = encode_state(run)?;
    let sequence = i16::try_from(run.last_run_sequence)
        .map_err(|_| AuthorityError::CorruptState("run sequence"))?;
    tx.execute(
        "INSERT INTO sf_capture_authority_v1.runs \
         (authority_digest, project_authority_digest, run_id, phase, last_run_sequence, \
          state_bytes, state_sha256) VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (authority_digest, project_authority_digest, run_id) DO UPDATE SET \
          phase = EXCLUDED.phase, last_run_sequence = EXCLUDED.last_run_sequence, \
          state_bytes = EXCLUDED.state_bytes, state_sha256 = EXCLUDED.state_sha256",
        &[
            &authority.as_str(),
            &run.key.project_authority_digest.as_str(),
            &run.key.run_id.as_str(),
            &run.phase.as_str(),
            &sequence,
            &state_bytes,
            &state_sha256.as_str(),
        ],
    )
    .await
    .map_err(pg)?;
    Ok(())
}

async fn persist_resource(
    tx: &Transaction<'_>,
    authority: &Digest,
    resource: &ResourceState,
) -> Result<(), AuthorityError> {
    let (state_bytes, state_sha256) = encode_state(resource)?;
    let fence = resource.fence.to_string();
    let owner_project = resource
        .owner
        .as_ref()
        .map(|value| value.project_authority_digest.as_str());
    let owner_run = resource.owner.as_ref().map(|value| value.run_id.as_str());
    tx.execute(
        "INSERT INTO sf_capture_authority_v1.resources \
         (authority_digest, resource_id, fence, disposition, owner_project_authority_digest, \
          owner_run_id, state_bytes, state_sha256) \
         VALUES ($1, $2, $3::text::numeric, $4, $5, $6, $7, $8) \
         ON CONFLICT (authority_digest, resource_id) DO UPDATE SET fence = EXCLUDED.fence, \
          disposition = EXCLUDED.disposition, \
          owner_project_authority_digest = EXCLUDED.owner_project_authority_digest, \
          owner_run_id = EXCLUDED.owner_run_id, state_bytes = EXCLUDED.state_bytes, \
          state_sha256 = EXCLUDED.state_sha256",
        &[
            &authority.as_str(),
            &resource.resource_id.as_str(),
            &fence.as_str(),
            &resource.disposition.as_str(),
            &owner_project,
            &owner_run,
            &state_bytes,
            &state_sha256.as_str(),
        ],
    )
    .await
    .map_err(pg)?;
    Ok(())
}

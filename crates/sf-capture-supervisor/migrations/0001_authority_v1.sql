-- SPDX-License-Identifier: MIT OR Apache-2.0
-- M0 capture-authority kernel only. Roles, RLS, transport, signing,
-- transparency publication and witnesses are deliberately separate migrations.

CREATE SCHEMA IF NOT EXISTS sf_capture_authority_v1;

CREATE TABLE IF NOT EXISTS sf_capture_authority_v1.authority_heads (
    authority_digest text PRIMARY KEY,
    next_global_sequence numeric(20, 0) NOT NULL,
    CHECK (authority_digest ~ '^[0-9a-f]{64}$' AND authority_digest !~ '^0+$'),
    CHECK (next_global_sequence BETWEEN 1 AND 18446744073709551615)
);

CREATE TABLE IF NOT EXISTS sf_capture_authority_v1.run_slots (
    authority_digest text NOT NULL REFERENCES sf_capture_authority_v1.authority_heads,
    project_authority_digest text NOT NULL,
    run_id text NOT NULL,
    PRIMARY KEY (authority_digest, project_authority_digest, run_id),
    CHECK (project_authority_digest ~ '^[0-9a-f]{64}$'
        AND project_authority_digest !~ '^0+$'),
    CHECK (run_id ~ '^[A-Za-z0-9_-]{8,128}$')
);

CREATE TABLE IF NOT EXISTS sf_capture_authority_v1.runs (
    authority_digest text NOT NULL,
    project_authority_digest text NOT NULL,
    run_id text NOT NULL,
    phase text NOT NULL,
    last_run_sequence smallint NOT NULL,
    state_bytes bytea NOT NULL,
    state_sha256 text NOT NULL,
    PRIMARY KEY (authority_digest, project_authority_digest, run_id),
    FOREIGN KEY (authority_digest, project_authority_digest, run_id)
        REFERENCES sf_capture_authority_v1.run_slots,
    CHECK (phase IN (
        'registered', 'leased', 'attempt-started', 'pre-start-terminal',
        'candidate-success-awaiting-final', 'failed-final-optional', 'final-witnessed'
    )),
    CHECK (last_run_sequence BETWEEN 0 AND 4),
    CHECK (octet_length(state_bytes) BETWEEN 2 AND 65536),
    CHECK (state_sha256 ~ '^[0-9a-f]{64}$' AND state_sha256 !~ '^0+$')
);

CREATE TABLE IF NOT EXISTS sf_capture_authority_v1.resource_slots (
    authority_digest text NOT NULL REFERENCES sf_capture_authority_v1.authority_heads,
    resource_id text NOT NULL,
    PRIMARY KEY (authority_digest, resource_id),
    CHECK (resource_id ~ '^[A-Za-z0-9_-]{8,128}$')
);

CREATE TABLE IF NOT EXISTS sf_capture_authority_v1.resources (
    authority_digest text NOT NULL,
    resource_id text NOT NULL,
    fence numeric(20, 0) NOT NULL,
    disposition text NOT NULL,
    owner_project_authority_digest text,
    owner_run_id text,
    state_bytes bytea NOT NULL,
    state_sha256 text NOT NULL,
    PRIMARY KEY (authority_digest, resource_id),
    FOREIGN KEY (authority_digest, resource_id)
        REFERENCES sf_capture_authority_v1.resource_slots,
    FOREIGN KEY (authority_digest, owner_project_authority_digest, owner_run_id)
        REFERENCES sf_capture_authority_v1.run_slots,
    CHECK (resource_id ~ '^[A-Za-z0-9_-]{8,128}$'),
    CHECK (fence BETWEEN 1 AND 18446744073709551615),
    CHECK (disposition IN (
        'held-pre-start', 'attempt-started', 'released-unstarted',
        'released-after-cleanup', 'quarantined'
    )),
    CHECK ((owner_project_authority_digest IS NULL) = (owner_run_id IS NULL)),
    CHECK (
        (disposition IN ('held-pre-start', 'attempt-started')
            AND owner_project_authority_digest IS NOT NULL)
        OR
        (disposition IN ('released-unstarted', 'released-after-cleanup', 'quarantined')
            AND owner_project_authority_digest IS NULL)
    ),
    CHECK (octet_length(state_bytes) BETWEEN 2 AND 65536),
    CHECK (state_sha256 ~ '^[0-9a-f]{64}$' AND state_sha256 !~ '^0+$')
);

CREATE TABLE IF NOT EXISTS sf_capture_authority_v1.events (
    authority_digest text NOT NULL REFERENCES sf_capture_authority_v1.authority_heads,
    project_authority_digest text NOT NULL,
    run_id text NOT NULL,
    global_sequence numeric(20, 0) NOT NULL,
    run_sequence smallint NOT NULL,
    event_kind text NOT NULL,
    semantic_request_digest text NOT NULL,
    canonical_request bytea NOT NULL,
    canonical_request_sha256 text NOT NULL,
    operation_digest text NOT NULL,
    proposal_digest text NOT NULL,
    event_digest text NOT NULL,
    event_envelope bytea NOT NULL,
    response_bytes bytea NOT NULL,
    response_sha256 text NOT NULL,
    PRIMARY KEY (authority_digest, global_sequence),
    UNIQUE (authority_digest, project_authority_digest, run_id, run_sequence),
    UNIQUE (authority_digest, project_authority_digest, semantic_request_digest),
    FOREIGN KEY (authority_digest, project_authority_digest, run_id)
        REFERENCES sf_capture_authority_v1.run_slots,
    CHECK (global_sequence BETWEEN 1 AND 18446744073709551615),
    CHECK (run_sequence BETWEEN 0 AND 4),
    CHECK (event_kind IN (
        'claim-registered-v2', 'runner-lease-granted-v2',
        'capture-attempt-start-committed-v2', 'capture-run-terminal-v2',
        'capture-attempt-terminal-v2', 'capture-final-witness-v2'
    )),
    CHECK (semantic_request_digest ~ '^[0-9a-f]{64}$'
        AND semantic_request_digest !~ '^0+$'),
    CHECK (canonical_request_sha256 ~ '^[0-9a-f]{64}$'
        AND canonical_request_sha256 !~ '^0+$'),
    CHECK (operation_digest ~ '^[0-9a-f]{64}$' AND operation_digest !~ '^0+$'),
    CHECK (proposal_digest ~ '^[0-9a-f]{64}$' AND proposal_digest !~ '^0+$'),
    CHECK (event_digest ~ '^[0-9a-f]{64}$' AND event_digest !~ '^0+$'),
    CHECK (response_sha256 ~ '^[0-9a-f]{64}$' AND response_sha256 !~ '^0+$'),
    CHECK (octet_length(canonical_request) BETWEEN 2 AND 32768),
    CHECK (octet_length(event_envelope) BETWEEN 1 AND 65536),
    CHECK (octet_length(response_bytes) BETWEEN 1 AND 196608)
);

CREATE TABLE IF NOT EXISTS sf_capture_authority_v1.semantic_outbox (
    authority_digest text NOT NULL,
    global_sequence numeric(20, 0) NOT NULL,
    event_digest text NOT NULL,
    event_envelope bytea NOT NULL,
    publication_state text NOT NULL DEFAULT 'pending',
    PRIMARY KEY (authority_digest, global_sequence),
    FOREIGN KEY (authority_digest, global_sequence)
        REFERENCES sf_capture_authority_v1.events,
    CHECK (publication_state = 'pending'),
    CHECK (event_digest ~ '^[0-9a-f]{64}$' AND event_digest !~ '^0+$')
);

CREATE SCHEMA sf_supervisor_v1 AUTHORIZATION sf_supervisor_owner_v1;
REVOKE ALL PRIVILEGES ON SCHEMA sf_supervisor_v1 FROM PUBLIC;
ALTER DEFAULT PRIVILEGES FOR ROLE sf_supervisor_owner_v1 REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;
ALTER DEFAULT PRIVILEGES FOR ROLE sf_supervisor_owner_v1 REVOKE USAGE ON TYPES FROM PUBLIC;
CREATE DOMAIN sf_supervisor_v1.configuration_bytes_v2 AS pg_catalog.bytea CONSTRAINT configuration_bytes_v2_check_v1 CHECK (((pg_catalog.octet_length(VALUE) >= 1) AND (pg_catalog.octet_length(VALUE) <= 131072)));
CREATE DOMAIN sf_supervisor_v1.ed25519_spki_der_v1 AS pg_catalog.bytea CONSTRAINT ed25519_spki_der_v1_check_v1 CHECK (((pg_catalog.octet_length(VALUE) = 44) AND (pg_catalog.substring(VALUE, 1, 12) = E'\\x302a300506032b6570032100'::pg_catalog.bytea)));
CREATE DOMAIN sf_supervisor_v1.event_envelope_bytes_v2 AS pg_catalog.bytea CONSTRAINT event_envelope_bytes_v2_check_v1 CHECK (((pg_catalog.octet_length(VALUE) >= 1) AND (pg_catalog.octet_length(VALUE) <= 65536)));
CREATE DOMAIN sf_supervisor_v1.opaque_id_v1 AS pg_catalog.text COLLATE pg_catalog."C" CONSTRAINT opaque_id_v1_check_v1 CHECK (((pg_catalog.octet_length(VALUE) >= 8) AND (pg_catalog.octet_length(VALUE) <= 128) AND (VALUE ~ '^[A-Za-z0-9_-]+$'::pg_catalog.text)));
CREATE DOMAIN sf_supervisor_v1.project_scope_role_v1 AS pg_catalog.text COLLATE pg_catalog."C" CONSTRAINT project_scope_role_v1_check_v1 CHECK ((VALUE = 'sf_supervisor_project_scope_v1'::pg_catalog.text));
CREATE DOMAIN sf_supervisor_v1.public_commitment_bytes_v2 AS pg_catalog.bytea CONSTRAINT public_commitment_bytes_v2_check_v1 CHECK (((pg_catalog.octet_length(VALUE) >= 1) AND (pg_catalog.octet_length(VALUE) <= 1024)));
CREATE DOMAIN sf_supervisor_v1.registration_result_bytes_v2 AS pg_catalog.bytea CONSTRAINT registration_result_bytes_v2_check_v1 CHECK (((pg_catalog.octet_length(VALUE) >= 1) AND (pg_catalog.octet_length(VALUE) <= 196608)));
CREATE DOMAIN sf_supervisor_v1.request_bytes_v2 AS pg_catalog.bytea CONSTRAINT request_bytes_v2_check_v1 CHECK (((pg_catalog.octet_length(VALUE) >= 1) AND (pg_catalog.octet_length(VALUE) <= 32768)));
CREATE DOMAIN sf_supervisor_v1.sha256_digest_v1 AS pg_catalog.bytea CONSTRAINT sha256_digest_v1_check_v1 CHECK (((pg_catalog.octet_length(VALUE) = 32) AND (VALUE <> E'\\x0000000000000000000000000000000000000000000000000000000000000000'::pg_catalog.bytea)));
CREATE DOMAIN sf_supervisor_v1.uint64_v1 AS pg_catalog.numeric CONSTRAINT uint64_v1_check_v1 CHECK (((pg_catalog.scale(VALUE) = 0) AND (VALUE >= (0)::pg_catalog.numeric) AND (VALUE <= '18446744073709551615'::pg_catalog.numeric)));
CREATE TABLE sf_supervisor_v1.authority_configurations (
  project_authority_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  project_scope_role sf_supervisor_v1.project_scope_role_v1 COLLATE pg_catalog."C" NOT NULL,
  configuration_epoch sf_supervisor_v1.uint64_v1 NOT NULL,
  configuration_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  genesis_authority_head_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  serialized_configuration sf_supervisor_v1.configuration_bytes_v2 NOT NULL,
  serialized_configuration_sha256 sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  project_principal_id sf_supervisor_v1.opaque_id_v1 COLLATE pg_catalog."C" NOT NULL,
  project_authentication_policy_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  service_principal_id sf_supervisor_v1.opaque_id_v1 COLLATE pg_catalog."C" NOT NULL,
  service_key_epoch sf_supervisor_v1.uint64_v1 NOT NULL,
  service_key_fingerprint sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  service_signing_spki_der sf_supervisor_v1.ed25519_spki_der_v1 NOT NULL,
  genesis_semantic_receipt_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  CONSTRAINT authority_configurations_digest_uq_v1 UNIQUE (project_authority_digest, project_scope_role, configuration_digest),
  CONSTRAINT authority_configurations_epoch_zero_check_v1 CHECK (((configuration_epoch)::pg_catalog.numeric = (0)::pg_catalog.numeric)),
  CONSTRAINT authority_configurations_genesis_receipt_uq_v1 UNIQUE (project_authority_digest, project_scope_role, configuration_epoch, configuration_digest, genesis_semantic_receipt_digest),
  CONSTRAINT authority_configurations_head_uq_v1 UNIQUE (project_authority_digest, project_scope_role, configuration_epoch, configuration_digest, genesis_authority_head_digest),
  CONSTRAINT authority_configurations_pk_v1 PRIMARY KEY (project_authority_digest, project_scope_role, configuration_epoch),
  CONSTRAINT authority_configurations_service_key_epoch_check_v1 CHECK (((service_key_epoch)::pg_catalog.numeric > (0)::pg_catalog.numeric))
) USING heap;
CREATE TABLE sf_supervisor_v1.authority_state (
  project_authority_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  project_scope_role sf_supervisor_v1.project_scope_role_v1 COLLATE pg_catalog."C" NOT NULL,
  singleton_key pg_catalog.bool NOT NULL,
  active_configuration_epoch sf_supervisor_v1.uint64_v1 NOT NULL,
  active_configuration_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  authority_head_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  last_global_sequence sf_supervisor_v1.uint64_v1 NOT NULL,
  next_global_sequence sf_supervisor_v1.uint64_v1 NOT NULL,
  last_event_digest sf_supervisor_v1.sha256_digest_v1,
  CONSTRAINT authority_state_last_event_check_v1 CHECK (((((last_global_sequence)::pg_catalog.numeric = (0)::pg_catalog.numeric) AND (last_event_digest IS NULL)) OR (((last_global_sequence)::pg_catalog.numeric >= (1)::pg_catalog.numeric) AND (last_event_digest IS NOT NULL)))),
  CONSTRAINT authority_state_pk_v1 PRIMARY KEY (project_authority_digest, project_scope_role),
  CONSTRAINT authority_state_sequence_successor_check_v1 CHECK (((next_global_sequence)::pg_catalog.numeric = ((last_global_sequence)::pg_catalog.numeric + (1)::pg_catalog.numeric))),
  CONSTRAINT authority_state_singleton_true_check_v1 CHECK ((singleton_key IS TRUE)),
  CONSTRAINT authority_state_singleton_uq_v1 UNIQUE (singleton_key)
) USING heap;
CREATE TABLE sf_supervisor_v1.publication_outbox (
  project_authority_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  project_scope_role sf_supervisor_v1.project_scope_role_v1 COLLATE pg_catalog."C" NOT NULL,
  event_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  public_commitment_leaf_bytes sf_supervisor_v1.public_commitment_bytes_v2 NOT NULL,
  public_commitment_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  publication_state pg_catalog.text COLLATE pg_catalog."C" NOT NULL,
  CONSTRAINT publication_outbox_commitment_uq_v1 UNIQUE (project_authority_digest, project_scope_role, public_commitment_digest),
  CONSTRAINT publication_outbox_pk_v1 PRIMARY KEY (project_authority_digest, project_scope_role, event_digest),
  CONSTRAINT publication_outbox_state_check_v1 CHECK ((publication_state = 'pending'::pg_catalog.text))
) USING heap;
CREATE TABLE sf_supervisor_v1.registration_results (
  project_authority_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  project_scope_role sf_supervisor_v1.project_scope_role_v1 COLLATE pg_catalog."C" NOT NULL,
  semantic_request_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  run_id sf_supervisor_v1.opaque_id_v1 COLLATE pg_catalog."C" NOT NULL,
  original_registration_request_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  original_registration_request_sha256 sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  original_registration_event_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  serialized_request sf_supervisor_v1.request_bytes_v2 NOT NULL,
  serialized_request_sha256 sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  response_status pg_catalog.int2 NOT NULL,
  response_content_type pg_catalog.text COLLATE pg_catalog."C" NOT NULL,
  serialized_response sf_supervisor_v1.registration_result_bytes_v2 NOT NULL,
  serialized_response_sha256 sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  current_event_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  CONSTRAINT registration_results_content_type_check_v1 CHECK ((response_content_type = 'application/json; charset=utf-8'::pg_catalog.text)),
  CONSTRAINT registration_results_current_event_uq_v1 UNIQUE (project_authority_digest, project_scope_role, current_event_digest),
  CONSTRAINT registration_results_pk_v1 PRIMARY KEY (project_authority_digest, project_scope_role, semantic_request_digest),
  CONSTRAINT registration_results_response_status_check_v1 CHECK (((response_status = 201) OR (response_status = 409))),
  CONSTRAINT registration_results_run_request_uq_v1 UNIQUE (project_authority_digest, project_scope_role, run_id, semantic_request_digest),
  CONSTRAINT registration_results_run_status_uq_v1 UNIQUE (project_authority_digest, project_scope_role, run_id, response_status),
  CONSTRAINT registration_results_status_provenance_check_v1 CHECK ((((response_status <> 201) OR (((semantic_request_digest)::pg_catalog.bytea = (original_registration_request_digest)::pg_catalog.bytea) AND ((serialized_request_sha256)::pg_catalog.bytea = (original_registration_request_sha256)::pg_catalog.bytea) AND ((current_event_digest)::pg_catalog.bytea = (original_registration_event_digest)::pg_catalog.bytea))) AND ((response_status <> 409) OR (((semantic_request_digest)::pg_catalog.bytea <> (original_registration_request_digest)::pg_catalog.bytea) AND ((current_event_digest)::pg_catalog.bytea <> (original_registration_event_digest)::pg_catalog.bytea)))))
) USING heap;
CREATE TABLE sf_supervisor_v1.registration_runs (
  project_authority_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  project_scope_role sf_supervisor_v1.project_scope_role_v1 COLLATE pg_catalog."C" NOT NULL,
  run_id sf_supervisor_v1.opaque_id_v1 COLLATE pg_catalog."C" NOT NULL,
  original_registration_request_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  original_registration_request_sha256 sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  original_registration_event_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  last_run_event_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  last_run_global_sequence sf_supervisor_v1.uint64_v1 NOT NULL,
  current_controller_state_head_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  last_run_sequence sf_supervisor_v1.uint64_v1 NOT NULL,
  first_changed_replay_request_digest sf_supervisor_v1.sha256_digest_v1,
  CONSTRAINT registration_runs_original_provenance_uq_v1 UNIQUE (project_authority_digest, project_scope_role, run_id, original_registration_request_digest, original_registration_request_sha256, original_registration_event_digest),
  CONSTRAINT registration_runs_pk_v1 PRIMARY KEY (project_authority_digest, project_scope_role, run_id),
  CONSTRAINT registration_runs_state_check_v1 CHECK (((((last_run_sequence)::pg_catalog.numeric = (0)::pg_catalog.numeric) AND ((last_run_event_digest)::pg_catalog.bytea = (original_registration_event_digest)::pg_catalog.bytea) AND (first_changed_replay_request_digest IS NULL)) OR (((last_run_sequence)::pg_catalog.numeric = (1)::pg_catalog.numeric) AND ((last_run_event_digest)::pg_catalog.bytea <> (original_registration_event_digest)::pg_catalog.bytea) AND (first_changed_replay_request_digest IS NOT NULL) AND ((first_changed_replay_request_digest)::pg_catalog.bytea <> (original_registration_request_digest)::pg_catalog.bytea))))
) USING heap;
CREATE TABLE sf_supervisor_v1.schema_migrations (
  migration_version pg_catalog.int4 NOT NULL,
  script_sha256 sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  catalog_contract_sha256 sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  authority_seed_sha256 sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  CONSTRAINT schema_migrations_pk_v1 PRIMARY KEY (migration_version),
  CONSTRAINT schema_migrations_version_positive_check_v1 CHECK ((migration_version > 0))
) USING heap;
CREATE TABLE sf_supervisor_v1.semantic_events (
  project_authority_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  project_scope_role sf_supervisor_v1.project_scope_role_v1 COLLATE pg_catalog."C" NOT NULL,
  event_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  event_kind pg_catalog.text COLLATE pg_catalog."C" NOT NULL,
  semantic_request_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  run_id sf_supervisor_v1.opaque_id_v1 COLLATE pg_catalog."C" NOT NULL,
  authority_configuration_epoch sf_supervisor_v1.uint64_v1 NOT NULL,
  authority_configuration_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  authority_head_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  global_sequence sf_supervisor_v1.uint64_v1 NOT NULL,
  run_sequence sf_supervisor_v1.uint64_v1 NOT NULL,
  previous_global_kind pg_catalog.text COLLATE pg_catalog."C" NOT NULL,
  previous_global_sequence sf_supervisor_v1.uint64_v1,
  previous_global_event_digest sf_supervisor_v1.sha256_digest_v1,
  previous_global_genesis_configuration_epoch sf_supervisor_v1.uint64_v1,
  previous_global_genesis_configuration_digest sf_supervisor_v1.sha256_digest_v1,
  previous_global_genesis_receipt_digest sf_supervisor_v1.sha256_digest_v1,
  previous_global_event_receipt_digest sf_supervisor_v1.sha256_digest_v1,
  previous_run_kind pg_catalog.text COLLATE pg_catalog."C" NOT NULL,
  previous_run_sequence sf_supervisor_v1.uint64_v1,
  previous_run_global_sequence sf_supervisor_v1.uint64_v1,
  previous_run_event_digest sf_supervisor_v1.sha256_digest_v1,
  prior_controller_state_head_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  resulting_controller_state_head_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  serialized_envelope sf_supervisor_v1.event_envelope_bytes_v2 NOT NULL,
  serialized_envelope_sha256 sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  CONSTRAINT semantic_events_global_reference_uq_v1 UNIQUE (project_authority_digest, project_scope_role, global_sequence, event_digest),
  CONSTRAINT semantic_events_global_sequence_check_v1 CHECK (((global_sequence)::pg_catalog.numeric >= (1)::pg_catalog.numeric)),
  CONSTRAINT semantic_events_global_sequence_uq_v1 UNIQUE (project_authority_digest, project_scope_role, global_sequence),
  CONSTRAINT semantic_events_kind_run_sequence_check_v1 CHECK ((((event_kind = 'claim-registered-v2'::pg_catalog.text) AND ((run_sequence)::pg_catalog.numeric = (0)::pg_catalog.numeric)) OR ((event_kind = 'capture-run-terminal-v2'::pg_catalog.text) AND ((run_sequence)::pg_catalog.numeric = (1)::pg_catalog.numeric)))),
  CONSTRAINT semantic_events_pk_v1 PRIMARY KEY (project_authority_digest, project_scope_role, event_digest),
  CONSTRAINT semantic_events_previous_global_check_v1 CHECK (((((global_sequence)::pg_catalog.numeric = (1)::pg_catalog.numeric) AND (previous_global_kind = 'authority-genesis'::pg_catalog.text) AND (previous_global_sequence IS NULL) AND (previous_global_event_digest IS NULL) AND (previous_global_genesis_configuration_epoch IS NOT NULL) AND (previous_global_genesis_configuration_digest IS NOT NULL) AND (previous_global_genesis_receipt_digest IS NOT NULL) AND (previous_global_event_receipt_digest IS NULL)) OR (((global_sequence)::pg_catalog.numeric > (1)::pg_catalog.numeric) AND (previous_global_kind = 'semantic-event'::pg_catalog.text) AND ((previous_global_sequence)::pg_catalog.numeric = ((global_sequence)::pg_catalog.numeric - (1)::pg_catalog.numeric)) AND (previous_global_event_digest IS NOT NULL) AND (previous_global_genesis_configuration_epoch IS NULL) AND (previous_global_genesis_configuration_digest IS NULL) AND (previous_global_genesis_receipt_digest IS NULL) AND (previous_global_event_receipt_digest IS NOT NULL)))),
  CONSTRAINT semantic_events_previous_run_check_v1 CHECK (((((run_sequence)::pg_catalog.numeric = (0)::pg_catalog.numeric) AND (previous_run_kind = 'run-genesis'::pg_catalog.text) AND (previous_run_sequence IS NULL) AND (previous_run_global_sequence IS NULL) AND (previous_run_event_digest IS NULL)) OR (((run_sequence)::pg_catalog.numeric = (1)::pg_catalog.numeric) AND (previous_run_kind = 'run-event'::pg_catalog.text) AND ((previous_run_sequence)::pg_catalog.numeric = (0)::pg_catalog.numeric) AND (previous_run_global_sequence IS NOT NULL) AND ((previous_run_global_sequence)::pg_catalog.numeric < (global_sequence)::pg_catalog.numeric) AND (previous_run_event_digest IS NOT NULL)))),
  CONSTRAINT semantic_events_request_provenance_uq_v1 UNIQUE (project_authority_digest, project_scope_role, semantic_request_digest, run_id, event_digest),
  CONSTRAINT semantic_events_run_sequence_uq_v1 UNIQUE (project_authority_digest, project_scope_role, run_id, run_sequence),
  CONSTRAINT semantic_events_run_snapshot_uq_v1 UNIQUE (project_authority_digest, project_scope_role, run_id, run_sequence, global_sequence, event_digest, resulting_controller_state_head_digest)
) USING heap;
CREATE TABLE sf_supervisor_v1.semantic_receipts (
  project_authority_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  project_scope_role sf_supervisor_v1.project_scope_role_v1 COLLATE pg_catalog."C" NOT NULL,
  event_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  semantic_receipt_digest sf_supervisor_v1.sha256_digest_v1 NOT NULL,
  CONSTRAINT semantic_receipts_digest_uq_v1 UNIQUE (project_authority_digest, project_scope_role, semantic_receipt_digest),
  CONSTRAINT semantic_receipts_event_receipt_uq_v1 UNIQUE (project_authority_digest, project_scope_role, event_digest, semantic_receipt_digest),
  CONSTRAINT semantic_receipts_pk_v1 PRIMARY KEY (project_authority_digest, project_scope_role, event_digest)
) USING heap;
ALTER TABLE ONLY sf_supervisor_v1.authority_state ADD CONSTRAINT authority_state_active_configuration_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, active_configuration_epoch, active_configuration_digest, authority_head_digest) REFERENCES sf_supervisor_v1.authority_configurations(project_authority_digest, project_scope_role, configuration_epoch, configuration_digest, genesis_authority_head_digest) ON UPDATE RESTRICT ON DELETE RESTRICT;
ALTER TABLE ONLY sf_supervisor_v1.authority_state ADD CONSTRAINT authority_state_last_event_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, last_global_sequence, last_event_digest) REFERENCES sf_supervisor_v1.semantic_events(project_authority_digest, project_scope_role, global_sequence, event_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.publication_outbox ADD CONSTRAINT publication_outbox_event_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, event_digest) REFERENCES sf_supervisor_v1.semantic_events(project_authority_digest, project_scope_role, event_digest) ON UPDATE RESTRICT ON DELETE RESTRICT;
ALTER TABLE ONLY sf_supervisor_v1.publication_outbox ADD CONSTRAINT publication_outbox_scope_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role) REFERENCES sf_supervisor_v1.authority_state(project_authority_digest, project_scope_role) ON UPDATE RESTRICT ON DELETE RESTRICT;
ALTER TABLE ONLY sf_supervisor_v1.registration_results ADD CONSTRAINT registration_results_current_event_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, semantic_request_digest, run_id, current_event_digest) REFERENCES sf_supervisor_v1.semantic_events(project_authority_digest, project_scope_role, semantic_request_digest, run_id, event_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.registration_results ADD CONSTRAINT registration_results_run_provenance_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, run_id, original_registration_request_digest, original_registration_request_sha256, original_registration_event_digest) REFERENCES sf_supervisor_v1.registration_runs(project_authority_digest, project_scope_role, run_id, original_registration_request_digest, original_registration_request_sha256, original_registration_event_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.registration_results ADD CONSTRAINT registration_results_scope_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role) REFERENCES sf_supervisor_v1.authority_state(project_authority_digest, project_scope_role) ON UPDATE RESTRICT ON DELETE RESTRICT;
ALTER TABLE ONLY sf_supervisor_v1.registration_runs ADD CONSTRAINT registration_runs_first_changed_result_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, run_id, first_changed_replay_request_digest) REFERENCES sf_supervisor_v1.registration_results(project_authority_digest, project_scope_role, run_id, semantic_request_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.registration_runs ADD CONSTRAINT registration_runs_last_event_snapshot_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, run_id, last_run_sequence, last_run_global_sequence, last_run_event_digest, current_controller_state_head_digest) REFERENCES sf_supervisor_v1.semantic_events(project_authority_digest, project_scope_role, run_id, run_sequence, global_sequence, event_digest, resulting_controller_state_head_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.registration_runs ADD CONSTRAINT registration_runs_original_event_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, original_registration_request_digest, run_id, original_registration_event_digest) REFERENCES sf_supervisor_v1.semantic_events(project_authority_digest, project_scope_role, semantic_request_digest, run_id, event_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.registration_runs ADD CONSTRAINT registration_runs_original_result_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, run_id, original_registration_request_digest) REFERENCES sf_supervisor_v1.registration_results(project_authority_digest, project_scope_role, run_id, semantic_request_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.registration_runs ADD CONSTRAINT registration_runs_scope_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role) REFERENCES sf_supervisor_v1.authority_state(project_authority_digest, project_scope_role) ON UPDATE RESTRICT ON DELETE RESTRICT;
ALTER TABLE ONLY sf_supervisor_v1.semantic_events ADD CONSTRAINT semantic_events_configuration_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, authority_configuration_epoch, authority_configuration_digest, authority_head_digest) REFERENCES sf_supervisor_v1.authority_configurations(project_authority_digest, project_scope_role, configuration_epoch, configuration_digest, genesis_authority_head_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.semantic_events ADD CONSTRAINT semantic_events_genesis_receipt_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, previous_global_genesis_configuration_epoch, previous_global_genesis_configuration_digest, previous_global_genesis_receipt_digest) REFERENCES sf_supervisor_v1.authority_configurations(project_authority_digest, project_scope_role, configuration_epoch, configuration_digest, genesis_semantic_receipt_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.semantic_events ADD CONSTRAINT semantic_events_previous_global_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, previous_global_sequence, previous_global_event_digest) REFERENCES sf_supervisor_v1.semantic_events(project_authority_digest, project_scope_role, global_sequence, event_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.semantic_events ADD CONSTRAINT semantic_events_previous_receipt_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, previous_global_event_digest, previous_global_event_receipt_digest) REFERENCES sf_supervisor_v1.semantic_receipts(project_authority_digest, project_scope_role, event_digest, semantic_receipt_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.semantic_events ADD CONSTRAINT semantic_events_previous_run_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, run_id, previous_run_sequence, previous_run_global_sequence, previous_run_event_digest, prior_controller_state_head_digest) REFERENCES sf_supervisor_v1.semantic_events(project_authority_digest, project_scope_role, run_id, run_sequence, global_sequence, event_digest, resulting_controller_state_head_digest) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.semantic_events ADD CONSTRAINT semantic_events_run_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, run_id) REFERENCES sf_supervisor_v1.registration_runs(project_authority_digest, project_scope_role, run_id) ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ONLY sf_supervisor_v1.semantic_events ADD CONSTRAINT semantic_events_scope_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role) REFERENCES sf_supervisor_v1.authority_state(project_authority_digest, project_scope_role) ON UPDATE RESTRICT ON DELETE RESTRICT;
ALTER TABLE ONLY sf_supervisor_v1.semantic_receipts ADD CONSTRAINT semantic_receipts_event_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role, event_digest) REFERENCES sf_supervisor_v1.semantic_events(project_authority_digest, project_scope_role, event_digest) ON UPDATE RESTRICT ON DELETE RESTRICT;
ALTER TABLE ONLY sf_supervisor_v1.semantic_receipts ADD CONSTRAINT semantic_receipts_scope_fk_v1 FOREIGN KEY (project_authority_digest, project_scope_role) REFERENCES sf_supervisor_v1.authority_state(project_authority_digest, project_scope_role) ON UPDATE RESTRICT ON DELETE RESTRICT;
REVOKE USAGE ON TYPE sf_supervisor_v1.authority_configurations FROM PUBLIC;
REVOKE USAGE ON TYPE sf_supervisor_v1.authority_state FROM PUBLIC;
REVOKE USAGE ON TYPE sf_supervisor_v1.publication_outbox FROM PUBLIC;
REVOKE USAGE ON TYPE sf_supervisor_v1.registration_results FROM PUBLIC;
REVOKE USAGE ON TYPE sf_supervisor_v1.registration_runs FROM PUBLIC;
REVOKE USAGE ON TYPE sf_supervisor_v1.schema_migrations FROM PUBLIC;
REVOKE USAGE ON TYPE sf_supervisor_v1.semantic_events FROM PUBLIC;
REVOKE USAGE ON TYPE sf_supervisor_v1.semantic_receipts FROM PUBLIC;
REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA sf_supervisor_v1 FROM PUBLIC;

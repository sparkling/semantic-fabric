// SPDX-License-Identifier: MIT

export function domainCheckExpressionV1(template: string): string {
  switch (template) {
    case 'configuration_bytes_v2_check_v1': return '((octet_length(VALUE) >= 1) AND (octet_length(VALUE) <= 131072))';
    case 'ed25519_spki_der_v1_check_v1': return '((octet_length(VALUE) = 44) AND ("substring"(VALUE, 1, 12) = \'\\x302a300506032b6570032100\'::bytea))';
    case 'event_envelope_bytes_v2_check_v1': return '((octet_length(VALUE) >= 1) AND (octet_length(VALUE) <= 65536))';
    case 'opaque_id_v1_check_v1': return '((octet_length(VALUE) >= 8) AND (octet_length(VALUE) <= 128) AND (VALUE ~ \'^[A-Za-z0-9_-]+$\'::text))';
    case 'project_scope_role_v1_check_v1': return '(VALUE = \'sf_supervisor_project_scope_v1\'::text)';
    case 'public_commitment_bytes_v2_check_v1': return '((octet_length(VALUE) >= 1) AND (octet_length(VALUE) <= 1024))';
    case 'registration_result_bytes_v2_check_v1': return '((octet_length(VALUE) >= 1) AND (octet_length(VALUE) <= 196608))';
    case 'request_bytes_v2_check_v1': return '((octet_length(VALUE) >= 1) AND (octet_length(VALUE) <= 32768))';
    case 'sha256_digest_v1_check_v1': return '((octet_length(VALUE) = 32) AND (VALUE <> \'\\x0000000000000000000000000000000000000000000000000000000000000000\'::bytea))';
    case 'uint64_v1_check_v1': return '((scale(VALUE) = 0) AND (VALUE >= (0)::numeric) AND (VALUE <= \'18446744073709551615\'::numeric))';
    default: throw invalid();
  }
}

export function domainBaseTypeV1(
  template: string,
): Readonly<{ type: 'bytea' | 'numeric' | 'text'; collation: 'C' | null }> {
  switch (template) {
    case 'opaque_id_v1_check_v1':
    case 'project_scope_role_v1_check_v1':
      return Object.freeze({ type: 'text', collation: 'C' });
    case 'uint64_v1_check_v1':
      return Object.freeze({ type: 'numeric', collation: null });
    case 'configuration_bytes_v2_check_v1':
    case 'ed25519_spki_der_v1_check_v1':
    case 'event_envelope_bytes_v2_check_v1':
    case 'public_commitment_bytes_v2_check_v1':
    case 'registration_result_bytes_v2_check_v1':
    case 'request_bytes_v2_check_v1':
    case 'sha256_digest_v1_check_v1':
      return Object.freeze({ type: 'bytea', collation: null });
    default: throw invalid();
  }
}

export function tableCheckExpressionV1(template: string): string {
  switch (template) {
    case 'authority_configurations_epoch_zero_check_v1': return '((configuration_epoch)::numeric = (0)::numeric)';
    case 'authority_configurations_service_key_epoch_check_v1': return '((service_key_epoch)::numeric > (0)::numeric)';
    case 'authority_state_last_event_check_v1': return '((((last_global_sequence)::numeric = (0)::numeric) AND (last_event_digest IS NULL)) OR (((last_global_sequence)::numeric >= (1)::numeric) AND (last_event_digest IS NOT NULL)))';
    case 'authority_state_sequence_successor_check_v1': return '((next_global_sequence)::numeric = ((last_global_sequence)::numeric + (1)::numeric))';
    case 'authority_state_singleton_true_check_v1': return '(singleton_key IS TRUE)';
    case 'publication_outbox_state_check_v1': return '(publication_state = \'pending\'::text)';
    case 'registration_results_content_type_check_v1': return '(response_content_type = \'application/json; charset=utf-8\'::text)';
    case 'registration_results_response_status_check_v1': return '((response_status = 201) OR (response_status = 409))';
    case 'registration_results_status_provenance_check_v1': return '(((response_status <> 201) OR (((semantic_request_digest)::bytea = (original_registration_request_digest)::bytea) AND ((serialized_request_sha256)::bytea = (original_registration_request_sha256)::bytea) AND ((current_event_digest)::bytea = (original_registration_event_digest)::bytea))) AND ((response_status <> 409) OR (((semantic_request_digest)::bytea <> (original_registration_request_digest)::bytea) AND ((current_event_digest)::bytea <> (original_registration_event_digest)::bytea))))';
    case 'registration_runs_state_check_v1': return '((((last_run_sequence)::numeric = (0)::numeric) AND ((last_run_event_digest)::bytea = (original_registration_event_digest)::bytea) AND (first_changed_replay_request_digest IS NULL)) OR (((last_run_sequence)::numeric = (1)::numeric) AND ((last_run_event_digest)::bytea <> (original_registration_event_digest)::bytea) AND (first_changed_replay_request_digest IS NOT NULL) AND ((first_changed_replay_request_digest)::bytea <> (original_registration_request_digest)::bytea)))';
    case 'schema_migrations_version_positive_check_v1': return '(migration_version > 0)';
    case 'semantic_events_global_sequence_check_v1': return '((global_sequence)::numeric >= (1)::numeric)';
    case 'semantic_events_kind_run_sequence_check_v1': return '(((event_kind = \'claim-registered-v2\'::text) AND ((run_sequence)::numeric = (0)::numeric)) OR ((event_kind = \'capture-run-terminal-v2\'::text) AND ((run_sequence)::numeric = (1)::numeric)))';
    case 'semantic_events_previous_global_check_v1': return '((((global_sequence)::numeric = (1)::numeric) AND (previous_global_kind = \'authority-genesis\'::text) AND (previous_global_sequence IS NULL) AND (previous_global_event_digest IS NULL) AND (previous_global_genesis_configuration_epoch IS NOT NULL) AND (previous_global_genesis_configuration_digest IS NOT NULL) AND (previous_global_genesis_receipt_digest IS NOT NULL) AND (previous_global_event_receipt_digest IS NULL)) OR (((global_sequence)::numeric > (1)::numeric) AND (previous_global_kind = \'semantic-event\'::text) AND ((previous_global_sequence)::numeric = ((global_sequence)::numeric - (1)::numeric)) AND (previous_global_event_digest IS NOT NULL) AND (previous_global_genesis_configuration_epoch IS NULL) AND (previous_global_genesis_configuration_digest IS NULL) AND (previous_global_genesis_receipt_digest IS NULL) AND (previous_global_event_receipt_digest IS NOT NULL)))';
    case 'semantic_events_previous_run_check_v1': return '((((run_sequence)::numeric = (0)::numeric) AND (previous_run_kind = \'run-genesis\'::text) AND (previous_run_sequence IS NULL) AND (previous_run_global_sequence IS NULL) AND (previous_run_event_digest IS NULL)) OR (((run_sequence)::numeric = (1)::numeric) AND (previous_run_kind = \'run-event\'::text) AND ((previous_run_sequence)::numeric = (0)::numeric) AND (previous_run_global_sequence IS NOT NULL) AND ((previous_run_global_sequence)::numeric < (global_sequence)::numeric) AND (previous_run_event_digest IS NOT NULL)))';
    default: throw invalid();
  }
}

export interface ConstraintDefinitionInputV1 {
  readonly kind: string;
  readonly columns: readonly string[];
  readonly referencedSchema: string | null;
  readonly referencedRelation: string | null;
  readonly referencedColumns: readonly string[] | null;
  readonly updateAction: string | null;
  readonly deleteAction: string | null;
  readonly deferrable: boolean;
  readonly initiallyDeferred: boolean;
  readonly expression: string | null;
}

export function constraintDefinitionV1(value: ConstraintDefinitionInputV1): string {
  if (value.kind === 'primary-key') return `PRIMARY KEY (${value.columns.join(', ')})`;
  if (value.kind === 'unique') return `UNIQUE (${value.columns.join(', ')})`;
  if (value.kind === 'check' && value.expression !== null) return `CHECK (${value.expression})`;
  if (value.kind !== 'foreign-key' || value.referencedSchema === null
    || value.referencedRelation === null || value.referencedColumns === null
    || value.updateAction !== 'restrict' || value.deleteAction !== 'restrict') throw invalid();
  const timing = value.deferrable
    ? ` DEFERRABLE${value.initiallyDeferred ? ' INITIALLY DEFERRED' : ''}` : '';
  return `FOREIGN KEY (${value.columns.join(', ')}) REFERENCES ${value.referencedSchema}.${value.referencedRelation}(${value.referencedColumns.join(', ')}) ON UPDATE RESTRICT ON DELETE RESTRICT${timing}`;
}

export interface PolicyArgumentsV1 {
  readonly scopeRole: string | null;
  readonly capabilityRole: string | null;
  readonly sessionLogin: string | null;
  readonly ownerRole: string | null;
}

export function policyExpressionV1(template: string, value: PolicyArgumentsV1): string {
  const scope = value.scopeRole;
  if (scope === null) throw invalid();
  if (template === 'scope-equality-v1' && value.capabilityRole === null
    && value.sessionLogin === null && value.ownerRole === null) {
    return `((project_scope_role)::text = '${scope}'::text)`;
  }
  if (template === 'scope-capability-v1' && value.capabilityRole !== null
    && value.sessionLogin === null && value.ownerRole === null) {
    return `(((project_scope_role)::text = '${scope}'::text) AND pg_has_role(SESSION_USER, '${value.capabilityRole}'::name, 'MEMBER'::text) AND pg_has_role(SESSION_USER, (project_scope_role)::name, 'MEMBER'::text))`;
  }
  if (template === 'migration-session-owner-v1' && value.capabilityRole === null
    && value.sessionLogin !== null && value.ownerRole !== null) {
    return `((CURRENT_USER = '${value.ownerRole}'::name) AND (SESSION_USER = '${value.sessionLogin}'::name) AND ((project_scope_role)::text = '${scope}'::text))`;
  }
  throw invalid();
}

function invalid(): TypeError {
  return new TypeError('PostgreSQL catalogue contract is invalid');
}

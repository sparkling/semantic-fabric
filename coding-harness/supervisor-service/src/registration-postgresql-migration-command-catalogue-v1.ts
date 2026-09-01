// SPDX-License-Identifier: MIT

import { copyPostgresMigrationLifecycleContractV1 }
  from './registration-postgresql-migration-lifecycle-v1.js';
import { postgresMigrationPlanPreflightReceiptV1 }
  from './registration-postgresql-migration-plan-v1.js';

const ARRAY = Array;
const CREATE = Object.create;
const DEFINE_PROPERTY = Object.defineProperty;
const FREEZE = Object.freeze;
const PLAIN_OBJECT_PROTOTYPE = Object.prototype;

const CONTROL_OPERATIONS = FREEZE([
  'begin',
  'set-search-path',
  'set-row-security',
  'set-lock-timeout',
  'set-statement-timeout',
  'set-idle-in-transaction-session-timeout',
  'set-synchronous-commit',
  'set-local-role',
  'commit',
  'rollback',
] as const);

const AUTHORITY_CONFIGURATION_INSERT = `INSERT INTO sf_supervisor_v1.authority_configurations (
  project_authority_digest,
  project_scope_role,
  configuration_epoch,
  configuration_digest,
  genesis_authority_head_digest,
  serialized_configuration,
  serialized_configuration_sha256,
  project_principal_id,
  project_authentication_policy_digest,
  service_principal_id,
  service_key_epoch,
  service_key_fingerprint,
  service_signing_spki_der,
  genesis_semantic_receipt_digest
) VALUES (
  $1::pg_catalog.bytea,
  $2::pg_catalog.text,
  $3::pg_catalog.numeric,
  $4::pg_catalog.bytea,
  $5::pg_catalog.bytea,
  $6::pg_catalog.bytea,
  $7::pg_catalog.bytea,
  $8::pg_catalog.text,
  $9::pg_catalog.bytea,
  $10::pg_catalog.text,
  $11::pg_catalog.numeric,
  $12::pg_catalog.bytea,
  $13::pg_catalog.bytea,
  $14::pg_catalog.bytea
)
`;

const AUTHORITY_STATE_INSERT = `INSERT INTO sf_supervisor_v1.authority_state (
  project_authority_digest,
  project_scope_role,
  singleton_key,
  active_configuration_epoch,
  active_configuration_digest,
  authority_head_digest,
  last_global_sequence,
  next_global_sequence,
  last_event_digest
) VALUES (
  $1::pg_catalog.bytea,
  $2::pg_catalog.text,
  $3::pg_catalog.bool,
  $4::pg_catalog.numeric,
  $5::pg_catalog.bytea,
  $6::pg_catalog.bytea,
  $7::pg_catalog.numeric,
  $8::pg_catalog.numeric,
  $9::pg_catalog.bytea
)
`;

const LEDGER_INSERT = `INSERT INTO sf_supervisor_v1.schema_migrations (
  migration_version,
  script_sha256,
  catalog_contract_sha256,
  authority_seed_sha256
) VALUES (
  $1::pg_catalog.int4,
  $2::pg_catalog.bytea,
  $3::pg_catalog.bytea,
  $4::pg_catalog.bytea
)
`;

export type PostgresMigrationCommandOperationV1 =
  | typeof CONTROL_OPERATIONS[number]
  | 'seed-authority-configuration-insert'
  | 'seed-authority-state-insert'
  | 'ledger-insert-version-1'
  | 'ledger-insert-version-2';

export type PostgresMigrationParameterBaseTypeV1 =
  'bytea' | 'text' | 'numeric' | 'bool' | 'int4';

export type PostgresMigrationParameterRepresentationV1 =
  | 'sha256-hex'
  | 'utf8-text'
  | 'canonical-uint64-decimal'
  | 'base64url'
  | 'boolean'
  | 'null'
  | 'safe-integer';

export interface PostgresMigrationCommandParameterV1 {
  readonly position: number;
  readonly baseType: PostgresMigrationParameterBaseTypeV1;
  readonly source: string;
  readonly representation: PostgresMigrationParameterRepresentationV1;
}

export interface PostgresMigrationCommandDescriptorV1 {
  readonly descriptorKind: 'postgresql-migration-command-descriptor-v1';
  readonly operation: PostgresMigrationCommandOperationV1;
  readonly text: string;
  readonly parameters: readonly PostgresMigrationCommandParameterV1[];
}

export interface PostgresMigrationCommandCatalogueV1 {
  readonly catalogueKind: 'postgresql-migration-command-catalogue-v1';
  readonly authority: 'none';
  readonly readinessAuthorized: false;
  readonly databaseAccessAuthorized: false;
  readonly migrationApplyAuthorized: false;
  readonly executableAuthority: false;
  readonly statementCatalogueComplete: false;
  readonly resultContractsSealed: false;
  readonly sourcePlanReceiptSha256: string;
  readonly sourcePins: Readonly<Record<
    'authoritySeed' | 'catalogueContract' | 'migration0001' | 'migration0002',
    Readonly<{ bytes: number; sha256: string }>
  >>;
  readonly controlCommands: readonly PostgresMigrationCommandDescriptorV1[];
  readonly insertCommands: readonly PostgresMigrationCommandDescriptorV1[];
}

type ParameterSpecV1 = readonly [
  PostgresMigrationParameterBaseTypeV1,
  string,
  PostgresMigrationParameterRepresentationV1,
];
type InsertSpecV1 = Readonly<{
  operation: PostgresMigrationCommandOperationV1;
  text: string;
  parameters: readonly ParameterSpecV1[];
}>;

const CONFIGURATION_PARAMETERS = FREEZE([
  ['bytea', 'authoritySeed.authorityConfiguration.projectAuthorityDigest', 'sha256-hex'],
  ['text', 'authoritySeed.authorityConfiguration.projectScopeRole', 'utf8-text'],
  ['numeric', 'authoritySeed.authorityConfiguration.configurationEpoch', 'canonical-uint64-decimal'],
  ['bytea', 'authoritySeed.authorityConfiguration.configurationDigest', 'sha256-hex'],
  ['bytea', 'authoritySeed.authorityConfiguration.genesisAuthorityHeadDigest', 'sha256-hex'],
  ['bytea', 'authoritySeed.authorityConfiguration.serializedConfiguration', 'base64url'],
  ['bytea', 'authoritySeed.authorityConfiguration.serializedConfigurationSha256', 'sha256-hex'],
  ['text', 'authoritySeed.authorityConfiguration.projectPrincipalId', 'utf8-text'],
  ['bytea', 'authoritySeed.authorityConfiguration.projectAuthenticationPolicyDigest', 'sha256-hex'],
  ['text', 'authoritySeed.authorityConfiguration.servicePrincipalId', 'utf8-text'],
  ['numeric', 'authoritySeed.authorityConfiguration.serviceKeyEpoch', 'canonical-uint64-decimal'],
  ['bytea', 'authoritySeed.authorityConfiguration.serviceKeyFingerprint', 'sha256-hex'],
  ['bytea', 'authoritySeed.authorityConfiguration.serviceSigningSpkiDer', 'base64url'],
  ['bytea', 'authoritySeed.authorityConfiguration.genesisSemanticReceiptDigest', 'sha256-hex'],
] as const);
const STATE_PARAMETERS = FREEZE([
  ['bytea', 'authoritySeed.authorityStateIdentity.projectAuthorityDigest', 'sha256-hex'],
  ['text', 'authoritySeed.authorityStateIdentity.projectScopeRole', 'utf8-text'],
  ['bool', 'authoritySeed.authorityStateIdentity.singletonKey', 'boolean'],
  ['numeric', 'authoritySeed.authorityStateIdentity.activeConfigurationEpoch', 'canonical-uint64-decimal'],
  ['bytea', 'authoritySeed.authorityStateIdentity.activeConfigurationDigest', 'sha256-hex'],
  ['bytea', 'authoritySeed.authorityStateIdentity.authorityHeadDigest', 'sha256-hex'],
  ['numeric', 'compiledGenesis.lastGlobalSequence', 'canonical-uint64-decimal'],
  ['numeric', 'compiledGenesis.nextGlobalSequence', 'canonical-uint64-decimal'],
  ['bytea', 'compiledGenesis.lastEventDigest', 'null'],
] as const);
const LEDGER_V1_PARAMETERS = FREEZE([
  ['int4', 'migration0001.migrationVersion', 'safe-integer'],
  ['bytea', 'migration0001.rawSha256', 'sha256-hex'],
  ['bytea', 'catalogueContract.rawSha256', 'sha256-hex'],
  ['bytea', 'authoritySeed.rawSha256', 'sha256-hex'],
] as const);
const LEDGER_V2_PARAMETERS = FREEZE([
  ['int4', 'migration0002.migrationVersion', 'safe-integer'],
  ['bytea', 'migration0002.rawSha256', 'sha256-hex'],
  ['bytea', 'catalogueContract.rawSha256', 'sha256-hex'],
  ['bytea', 'authoritySeed.rawSha256', 'sha256-hex'],
] as const);
const INSERT_SPECS = FREEZE([
  FREEZE({
    operation: 'seed-authority-configuration-insert',
    text: AUTHORITY_CONFIGURATION_INSERT,
    parameters: CONFIGURATION_PARAMETERS,
  }),
  FREEZE({
    operation: 'seed-authority-state-insert',
    text: AUTHORITY_STATE_INSERT,
    parameters: STATE_PARAMETERS,
  }),
  FREEZE({
    operation: 'ledger-insert-version-1',
    text: LEDGER_INSERT,
    parameters: LEDGER_V1_PARAMETERS,
  }),
  FREEZE({
    operation: 'ledger-insert-version-2',
    text: LEDGER_INSERT,
    parameters: LEDGER_V2_PARAMETERS,
  }),
] as const satisfies readonly InsertSpecV1[]);

/** Copy exact but incomplete command metadata. This grants no execution authority. */
export function copyPostgresMigrationCommandCatalogueV1(
  plan: unknown,
): PostgresMigrationCommandCatalogueV1 {
  const receipt = postgresMigrationPlanPreflightReceiptV1(plan);
  const lifecycle = copyPostgresMigrationLifecycleContractV1();
  const controls = new ARRAY<PostgresMigrationCommandDescriptorV1>(
    CONTROL_OPERATIONS.length,
  );
  for (let index = 0; index < CONTROL_OPERATIONS.length; index += 1) {
    const operation = CONTROL_OPERATIONS[index]!;
    DEFINE_PROPERTY(controls, index, {
      value: descriptorV1(operation, lifecycle.controlSql[operation], FREEZE([])),
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  const inserts = new ARRAY<PostgresMigrationCommandDescriptorV1>(INSERT_SPECS.length);
  for (let index = 0; index < INSERT_SPECS.length; index += 1) {
    const spec = INSERT_SPECS[index]!;
    DEFINE_PROPERTY(inserts, index, {
      value: descriptorV1(spec.operation, spec.text, copyParametersV1(spec.parameters)),
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return recordV1<PostgresMigrationCommandCatalogueV1>([
    ['catalogueKind', 'postgresql-migration-command-catalogue-v1'],
    ['authority', 'none'],
    ['readinessAuthorized', false],
    ['databaseAccessAuthorized', false],
    ['migrationApplyAuthorized', false],
    ['executableAuthority', false],
    ['statementCatalogueComplete', false],
    ['resultContractsSealed', false],
    ['sourcePlanReceiptSha256', receipt.receiptSha256],
    ['sourcePins', recordV1([
      ['authoritySeed', copyPinV1(receipt.artifacts.authoritySeed)],
      ['catalogueContract', copyPinV1(receipt.artifacts.catalogueContract)],
      ['migration0001', copyPinV1(receipt.artifacts.migration0001)],
      ['migration0002', copyPinV1(receipt.artifacts.migration0002)],
    ])],
    ['controlCommands', FREEZE(controls)],
    ['insertCommands', FREEZE(inserts)],
  ]);
}

function descriptorV1(
  operation: PostgresMigrationCommandOperationV1,
  text: string,
  parameters: readonly PostgresMigrationCommandParameterV1[],
): PostgresMigrationCommandDescriptorV1 {
  return recordV1([
    ['descriptorKind', 'postgresql-migration-command-descriptor-v1'],
    ['operation', operation],
    ['text', text],
    ['parameters', parameters],
  ]);
}

function copyParametersV1(
  specs: readonly ParameterSpecV1[],
): readonly PostgresMigrationCommandParameterV1[] {
  const parameters = new ARRAY<PostgresMigrationCommandParameterV1>(specs.length);
  for (let index = 0; index < specs.length; index += 1) {
    const spec = specs[index]!;
    DEFINE_PROPERTY(parameters, index, {
      value: recordV1([
        ['position', index + 1],
        ['baseType', spec[0]],
        ['source', spec[1]],
        ['representation', spec[2]],
      ]),
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return FREEZE(parameters);
}

function copyPinV1(pin: Readonly<{ bytes: number; sha256: string }>) {
  return recordV1<Readonly<{ bytes: number; sha256: string }>>([
    ['bytes', pin.bytes],
    ['sha256', pin.sha256],
  ]);
}

function recordV1<T>(entries: readonly (readonly [string, unknown])[]): T {
  const record = CREATE(PLAIN_OBJECT_PROTOTYPE) as Record<string, unknown>;
  for (let index = 0; index < entries.length; index += 1) {
    const entry = entries[index]!;
    DEFINE_PROPERTY(record, entry[0], {
      value: entry[1],
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return FREEZE(record) as T;
}

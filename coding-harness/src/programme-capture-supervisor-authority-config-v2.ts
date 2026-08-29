// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  assertExactKeys,
  deepFreeze,
  normalizePublicHttpsOrigin,
} from './contracts.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_MAX_BYTES_V2 = 131_072;
export const PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-authority-configuration-digest-v2';

const UINT64_DECIMAL_PATTERN = /^(?:0|[1-9][0-9]{0,19})$/;
const MAX_UINT64 = 18_446_744_073_709_551_615n;
const MINIMUM_WITNESS_MEMBERS = 4;
const MAXIMUM_WITNESS_MEMBERS = 64;

export interface ProgrammeCaptureSupervisorPrincipalV2 {
  readonly principalId: string;
  readonly keyEpoch: string;
  readonly keyFingerprint: string;
  readonly policyDigest: string;
  readonly administrationDigest: string;
}

export interface ProgrammeCaptureSupervisorWitnessPolicyV2 {
  readonly policyId: string;
  readonly faultThreshold: string;
  readonly quorumThreshold: string;
  readonly members: readonly ProgrammeCaptureSupervisorPrincipalV2[];
}

type ConfigurationPredecessorV2 = Readonly<
  | { kind: 'genesis'; configurationDigest: null; transitionDigest: null }
  | { kind: 'transition'; configurationDigest: string; transitionDigest: string }
>;

export interface ProgrammeCaptureSupervisorAuthorityConfigurationV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly recordKind: 'supervisor-authority-configuration-v2';
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly configurationEpoch: string;
  readonly predecessor: ConfigurationPredecessorV2;
  readonly project: Readonly<{
    projectAuthorityDigest: string;
    principal: ProgrammeCaptureSupervisorPrincipalV2;
    authenticationPolicyDigest: string;
  }>;
  readonly service: Readonly<{
    principal: ProgrammeCaptureSupervisorPrincipalV2;
    endpointOrigin: string;
    tlsSpkiFingerprint: string;
    clientPolicyDigest: string;
  }>;
  readonly transparencyLog: Readonly<{
    principal: ProgrammeCaptureSupervisorPrincipalV2;
    endpointOrigin: string;
    tlsSpkiFingerprint: string;
    publicCommitmentPolicyDigest: string;
  }>;
  readonly checkpointWitnesses: ProgrammeCaptureSupervisorWitnessPolicyV2;
  readonly semanticWitnesses: ProgrammeCaptureSupervisorWitnessPolicyV2;
  readonly initializationAnchor: ProgrammeCaptureSupervisorPrincipalV2;
  readonly runnerEnrollment: ProgrammeCaptureSupervisorPrincipalV2;
  readonly deploymentAttestor: ProgrammeCaptureSupervisorPrincipalV2;
  readonly readinessPolicyDigest: string;
  readonly verificationScope: 'trust-pins-and-quorum-math-only';
  readonly externalAdministrationVerified: false;
  readonly deploymentAttestationVerified: false;
  readonly checkpointWitnessQuorumVerified: false;
  readonly semanticWitnessQuorumVerified: false;
  readonly stateTransitionAuthorized: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
  readonly configurationDigest: string;
}

const NON_AUTHORITY = Object.freeze({
  externalAdministrationVerified: false as const,
  deploymentAttestationVerified: false as const,
  checkpointWitnessQuorumVerified: false as const,
  semanticWitnessQuorumVerified: false as const,
  stateTransitionAuthorized: false as const,
  attemptStartAuthorized: false as const,
  captureAuthorized: false as const,
});
const NON_AUTHORITY_KEYS = Object.freeze(Object.keys(NON_AUTHORITY));

export function parseProgrammeCaptureSupervisorAuthorityConfigurationV2(
  value: unknown,
): ProgrammeCaptureSupervisorAuthorityConfigurationV2 {
  const input = closedRecord(value, 'programme capture supervisor authority configuration');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'recordKind', 'authority', 'configurationEpoch',
    'predecessor', 'project', 'service', 'transparencyLog', 'checkpointWitnesses',
    'semanticWitnesses', 'initializationAnchor', 'runnerEnrollment', 'deploymentAttestor',
    'readinessPolicyDigest', 'verificationScope', ...NON_AUTHORITY_KEYS, 'configurationDigest',
  ], 'programme capture supervisor authority configuration');
  assertIdentityAndScope(input);
  assertNonAuthority(input);

  const configurationEpoch = parseUint64(
    input.configurationEpoch, 'supervisor authority configuration epoch', 0n,
  );
  const predecessor = parsePredecessor(input.predecessor, configurationEpoch);
  const project = parseProject(input.project);
  const service = parseService(input.service);
  const transparencyLog = parseTransparencyLog(input.transparencyLog);
  const checkpointWitnesses = parseWitnessPolicy(
    input.checkpointWitnesses, 'checkpoint witness policy',
  );
  const semanticWitnesses = parseWitnessPolicy(
    input.semanticWitnesses, 'semantic witness policy',
  );
  if (checkpointWitnesses.policyId === semanticWitnesses.policyId) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_WITNESS_POLICY_REUSE');
  }
  const initializationAnchor = parsePrincipal(
    input.initializationAnchor, 'supervisor witness initialization anchor',
  );
  const runnerEnrollment = parsePrincipal(
    input.runnerEnrollment, 'supervisor runner enrollment authority',
  );
  const deploymentAttestor = parsePrincipal(
    input.deploymentAttestor, 'supervisor deployment attestor',
  );
  assertRoleSeparation({
    project, service, transparencyLog, checkpointWitnesses, semanticWitnesses,
    initializationAnchor, runnerEnrollment, deploymentAttestor,
  });

  const body = {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    recordKind: 'supervisor-authority-configuration-v2' as const,
    authority: DEVELOPMENT_AUTHORITY,
    configurationEpoch,
    predecessor,
    project,
    service,
    transparencyLog,
    checkpointWitnesses,
    semanticWitnesses,
    initializationAnchor,
    runnerEnrollment,
    deploymentAttestor,
    readinessPolicyDigest: parseDigest(
      input.readinessPolicyDigest, 'supervisor readiness policy digest',
    ),
    verificationScope: 'trust-pins-and-quorum-math-only' as const,
    ...NON_AUTHORITY,
  };
  const configurationDigest = parseDigest(
    input.configurationDigest, 'supervisor authority configuration digest',
  );
  if (configurationDigest !== digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
    configuration: body,
  })) throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_MISMATCH');
  return deepFreeze({ ...body, configurationDigest });
}

export function serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(
  value: unknown,
): string {
  return `${JSON.stringify(
    parseProgrammeCaptureSupervisorAuthorityConfigurationV2(value), null, 2,
  )}\n`;
}

export function parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2(
  serialized: string,
): ProgrammeCaptureSupervisorAuthorityConfigurationV2 {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8')
      > PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_MAX_BYTES_V2
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError('supervisor authority configuration must be bounded canonical UTF-8 JSON');
  }
  const parsed = parseProgrammeCaptureSupervisorAuthorityConfigurationV2(
    parseJsonWithoutDuplicateKeys(serialized, 'supervisor authority configuration'),
  );
  if (serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(parsed) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_CANONICAL_REQUIRED');
  }
  return parsed;
}

function parseProject(value: unknown) {
  const input = closedRecord(value, 'supervisor project authority');
  assertExactKeys(input, [
    'projectAuthorityDigest', 'principal', 'authenticationPolicyDigest',
  ], 'supervisor project authority');
  return Object.freeze({
    projectAuthorityDigest: parseDigest(
      input.projectAuthorityDigest, 'supervisor project authority digest',
    ),
    principal: parsePrincipal(input.principal, 'supervisor project principal'),
    authenticationPolicyDigest: parseDigest(
      input.authenticationPolicyDigest, 'supervisor project authentication policy digest',
    ),
  });
}

function parseService(value: unknown) {
  const input = closedRecord(value, 'supervisor service');
  assertExactKeys(input, [
    'principal', 'endpointOrigin', 'tlsSpkiFingerprint', 'clientPolicyDigest',
  ], 'supervisor service');
  return Object.freeze({
    principal: parsePrincipal(input.principal, 'supervisor service principal'),
    endpointOrigin: normalizePublicHttpsOrigin(input.endpointOrigin, 'supervisor service origin'),
    tlsSpkiFingerprint: parseDigest(
      input.tlsSpkiFingerprint, 'supervisor service TLS SPKI fingerprint',
    ),
    clientPolicyDigest: parseDigest(
      input.clientPolicyDigest, 'supervisor service client policy digest',
    ),
  });
}

function parseTransparencyLog(value: unknown) {
  const input = closedRecord(value, 'supervisor transparency log');
  assertExactKeys(input, [
    'principal', 'endpointOrigin', 'tlsSpkiFingerprint', 'publicCommitmentPolicyDigest',
  ], 'supervisor transparency log');
  return Object.freeze({
    principal: parsePrincipal(input.principal, 'supervisor transparency log principal'),
    endpointOrigin: normalizePublicHttpsOrigin(
      input.endpointOrigin, 'supervisor transparency log origin',
    ),
    tlsSpkiFingerprint: parseDigest(
      input.tlsSpkiFingerprint, 'supervisor transparency log TLS SPKI fingerprint',
    ),
    publicCommitmentPolicyDigest: parseDigest(
      input.publicCommitmentPolicyDigest, 'supervisor public commitment policy digest',
    ),
  });
}

function parseWitnessPolicy(
  value: unknown,
  label: string,
): ProgrammeCaptureSupervisorWitnessPolicyV2 {
  const input = closedRecord(value, label);
  assertExactKeys(
    input, ['policyId', 'faultThreshold', 'quorumThreshold', 'members'], label,
  );
  const entries = denseArray(input.members, `${label} members`);
  if (entries.length > MAXIMUM_WITNESS_MEMBERS) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_WITNESS_ROSTER_INVALID');
  }
  const members = entries.map((entry, index) =>
    parsePrincipal(entry, `${label} member[${index}]`));
  const memberIds = members.map(({ principalId }) => principalId);
  if (memberIds.some((id, index) => index > 0 && memberIds[index - 1] >= id)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_WITNESS_ROSTER_ORDER_INVALID');
  }
  const faultThreshold = parseUint64(input.faultThreshold, `${label} fault threshold`, 1n);
  const quorumThreshold = parseUint64(input.quorumThreshold, `${label} quorum threshold`, 1n);
  const n = BigInt(members.length);
  const f = BigInt(faultThreshold);
  const q = BigInt(quorumThreshold);
  if (n < BigInt(MINIMUM_WITNESS_MEMBERS) || n < (3n * f) + 1n
    || q < (2n * f) + 1n || q > n || (2n * q) <= n + f) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_QUORUM_POLICY_INVALID');
  }
  return deepFreeze({
    policyId: parseTaskOpaqueId(input.policyId, `${label} ID`),
    faultThreshold,
    quorumThreshold,
    members,
  });
}

function parsePrincipal(
  value: unknown,
  label: string,
): ProgrammeCaptureSupervisorPrincipalV2 {
  const input = closedRecord(value, label);
  assertExactKeys(input, [
    'principalId', 'keyEpoch', 'keyFingerprint', 'policyDigest', 'administrationDigest',
  ], label);
  return Object.freeze({
    principalId: parseTaskOpaqueId(input.principalId, `${label} ID`),
    keyEpoch: parseUint64(input.keyEpoch, `${label} key epoch`, 1n),
    keyFingerprint: parseDigest(input.keyFingerprint, `${label} key fingerprint`),
    policyDigest: parseDigest(input.policyDigest, `${label} policy digest`),
    administrationDigest: parseDigest(
      input.administrationDigest, `${label} administration digest`,
    ),
  });
}

function parsePredecessor(value: unknown, configurationEpoch: string): ConfigurationPredecessorV2 {
  const input = closedRecord(value, 'supervisor authority configuration predecessor');
  assertExactKeys(input, [
    'kind', 'configurationDigest', 'transitionDigest',
  ], 'supervisor authority configuration predecessor');
  if (configurationEpoch === '0') {
    if (input.kind !== 'genesis' || input.configurationDigest !== null
      || input.transitionDigest !== null) {
      throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PREDECESSOR_INVALID');
    }
    return Object.freeze({ kind: 'genesis', configurationDigest: null, transitionDigest: null });
  }
  if (input.kind !== 'transition') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PREDECESSOR_INVALID');
  }
  return Object.freeze({
    kind: 'transition',
    configurationDigest: parseDigest(
      input.configurationDigest, 'predecessor configuration digest',
    ),
    transitionDigest: parseDigest(input.transitionDigest, 'predecessor transition digest'),
  });
}

function assertRoleSeparation(value: Readonly<{
  project: ReturnType<typeof parseProject>;
  service: ReturnType<typeof parseService>;
  transparencyLog: ReturnType<typeof parseTransparencyLog>;
  checkpointWitnesses: ProgrammeCaptureSupervisorWitnessPolicyV2;
  semanticWitnesses: ProgrammeCaptureSupervisorWitnessPolicyV2;
  initializationAnchor: ProgrammeCaptureSupervisorPrincipalV2;
  runnerEnrollment: ProgrammeCaptureSupervisorPrincipalV2;
  deploymentAttestor: ProgrammeCaptureSupervisorPrincipalV2;
}>): void {
  if (value.service.endpointOrigin === value.transparencyLog.endpointOrigin) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_DUPLICATE_SERVICE_ORIGIN');
  }
  const principals = [
    value.project.principal, value.service.principal, value.transparencyLog.principal,
    ...value.checkpointWitnesses.members, ...value.semanticWitnesses.members,
    value.initializationAnchor, value.runnerEnrollment, value.deploymentAttestor,
  ];
  assertUnique(principals.map(({ principalId }) => principalId), 'principal identity');
  assertUnique([
    ...principals.map(({ keyFingerprint }) => keyFingerprint),
    value.service.tlsSpkiFingerprint, value.transparencyLog.tlsSpkiFingerprint,
  ], 'role key fingerprint');
  assertUnique(
    principals.map(({ administrationDigest }) => administrationDigest),
    'principal administration',
  );
}

function assertUnique(values: readonly string[], label: string): void {
  if (new Set(values).size !== values.length) {
    throw new TypeError(`HARNESS_CAPTURE_SUPERVISOR_DUPLICATE_${label.toUpperCase().replaceAll(' ', '_')}`);
  }
}

function assertIdentityAndScope(input: Record<string, unknown>): void {
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.recordKind !== 'supervisor-authority-configuration-v2'
    || input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_IDENTITY_INVALID');
  }
  if (input.verificationScope !== 'trust-pins-and-quorum-math-only') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_SCOPE_INVALID');
  }
}

function assertNonAuthority(input: Record<string, unknown>): void {
  if (NON_AUTHORITY_KEYS.some(
    (key) => input[key] !== NON_AUTHORITY[key as keyof typeof NON_AUTHORITY],
  )) throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_AUTHORITY_ESCALATION');
}

function parseUint64(value: unknown, label: string, minimum: bigint): string {
  if (typeof value !== 'string' || !UINT64_DECIMAL_PATTERN.test(value)
    || BigInt(value) < minimum || BigInt(value) > MAX_UINT64) {
    throw new TypeError(`${label} must be a canonical uint64 decimal string >= ${minimum}`);
  }
  return value;
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

function denseArray(value: unknown, label: string): unknown[] {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asDenseArray(value, label);
}

function closedRecord(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}

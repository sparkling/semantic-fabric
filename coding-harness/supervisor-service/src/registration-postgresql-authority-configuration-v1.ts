// SPDX-License-Identifier: MIT

import { isIP } from 'node:net';
import {
  canonicalDigestHexV1,
  closedRecordV1,
  deepFreezeV1,
  denseArrayV1,
  exactKeysV1,
  parseCanonicalPrettyJsonBytesV1,
  parseDigestV1,
  parseOpaqueIdV1,
  parseUint64V1,
  snapshotBytesV1,
  utf8TextV1,
} from './registration-postgresql-canonical-v1.js';

export const POSTGRES_AUTHORITY_CONFIG_MAX_BYTES_V2 = 131_072;
export const POSTGRES_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-authority-configuration-digest-v2';
export const POSTGRES_AUTHORITY_GENESIS_HEAD_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-authority-genesis-head-v2';
export const POSTGRES_TRANSPARENCY_LOG_IDENTITY_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-transparency-log-identity-digest-v2';

const MINIMUM_WITNESS_MEMBERS = 4;
const MAXIMUM_WITNESS_MEMBERS = 64;
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

export interface PostgresAuthorityPrincipalV2 {
  readonly principalId: string;
  readonly keyEpoch: string;
  readonly keyFingerprint: string;
  readonly policyDigest: string;
  readonly administrationDigest: string;
}

export interface PostgresAuthorityWitnessPolicyV2 {
  readonly policyId: string;
  readonly faultThreshold: string;
  readonly quorumThreshold: string;
  readonly members: readonly PostgresAuthorityPrincipalV2[];
}

export interface PostgresAuthorityConfigurationV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly recordKind: 'supervisor-authority-configuration-v2';
  readonly authority: 'development-only-no-promotion';
  readonly configurationEpoch: string;
  readonly predecessor: Readonly<
    | { kind: 'genesis'; configurationDigest: null; headDigest: null }
    | { kind: 'configuration-head'; configurationDigest: string; headDigest: string }
  >;
  readonly project: Readonly<{
    projectAuthorityDigest: string;
    principal: PostgresAuthorityPrincipalV2;
    authenticationPolicyDigest: string;
  }>;
  readonly service: Readonly<{
    principal: PostgresAuthorityPrincipalV2;
    endpointOrigin: string;
    tlsSpkiFingerprint: string;
    clientPolicyDigest: string;
  }>;
  readonly transparencyLog: Readonly<{
    principal: PostgresAuthorityPrincipalV2;
    endpointOrigin: string;
    tlsSpkiFingerprint: string;
    publicCommitmentPolicyDigest: string;
  }>;
  readonly checkpointWitnesses: PostgresAuthorityWitnessPolicyV2;
  readonly semanticWitnesses: PostgresAuthorityWitnessPolicyV2;
  readonly initializationAnchor: PostgresAuthorityPrincipalV2;
  readonly runnerEnrollment: PostgresAuthorityPrincipalV2;
  readonly deploymentAttestor: PostgresAuthorityPrincipalV2;
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

export function parsePostgresAuthorityConfigurationBytesV1(
  value: unknown,
): PostgresAuthorityConfigurationV2 {
  const bytes = snapshotBytesV1(
    value, 'locked authority configuration bytes', POSTGRES_AUTHORITY_CONFIG_MAX_BYTES_V2,
  );
  const text = utf8TextV1(
    bytes, 'locked authority configuration bytes', POSTGRES_AUTHORITY_CONFIG_MAX_BYTES_V2,
  );
  const parsed = parseConfigurationRecord(parseCanonicalPrettyJsonBytesV1(
    bytes, 'locked authority configuration', POSTGRES_AUTHORITY_CONFIG_MAX_BYTES_V2,
  ));
  if (`${JSON.stringify(parsed, null, 2)}\n` !== text) {
    throw new TypeError('locked authority configuration member order is noncanonical');
  }
  return parsed;
}

export function postgresAuthorityGenesisHeadDigestV1(
  configuration: PostgresAuthorityConfigurationV2,
): string {
  if (configuration.configurationEpoch !== '0' || configuration.predecessor.kind !== 'genesis') {
    throw new TypeError('M0 authority configuration must be genesis epoch zero');
  }
  return canonicalDigestHexV1({
    domain: POSTGRES_AUTHORITY_GENESIS_HEAD_DOMAIN_V2,
    configurationEpoch: configuration.configurationEpoch,
    configurationDigest: configuration.configurationDigest,
  });
}

export function postgresTransparencyLogIdentityDigestV1(
  configuration: PostgresAuthorityConfigurationV2,
): string {
  return canonicalDigestHexV1({
    domain: POSTGRES_TRANSPARENCY_LOG_IDENTITY_DOMAIN_V2,
    transparencyLog: configuration.transparencyLog,
  });
}

function parseConfigurationRecord(
  input: Record<string, unknown>,
): PostgresAuthorityConfigurationV2 {
  exactKeysV1(input, [
    'schemaVersion', 'transactionKind', 'recordKind', 'authority', 'configurationEpoch',
    'predecessor', 'project', 'service', 'transparencyLog', 'checkpointWitnesses',
    'semanticWitnesses', 'initializationAnchor', 'runnerEnrollment', 'deploymentAttestor',
    'readinessPolicyDigest', 'verificationScope', ...NON_AUTHORITY_KEYS, 'configurationDigest',
  ], 'locked authority configuration');
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.recordKind !== 'supervisor-authority-configuration-v2'
    || input.authority !== 'development-only-no-promotion'
    || input.verificationScope !== 'trust-pins-and-quorum-math-only'
    || NON_AUTHORITY_KEYS.some((key) => input[key] !== false)) {
    throw new TypeError('locked authority configuration identity is invalid');
  }
  const configurationEpoch = parseUint64V1(
    input.configurationEpoch, 'authority configuration epoch', 0n,
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
    throw new TypeError('authority witness policy IDs must differ');
  }
  const initializationAnchor = parsePrincipal(
    input.initializationAnchor, 'initialization anchor',
  );
  const runnerEnrollment = parsePrincipal(input.runnerEnrollment, 'runner enrollment');
  const deploymentAttestor = parsePrincipal(input.deploymentAttestor, 'deployment attestor');
  assertRoleSeparation({
    project, service, transparencyLog, checkpointWitnesses, semanticWitnesses,
    initializationAnchor, runnerEnrollment, deploymentAttestor,
  });
  const body = {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    recordKind: 'supervisor-authority-configuration-v2' as const,
    authority: 'development-only-no-promotion' as const,
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
    readinessPolicyDigest: parseDigestV1(
      input.readinessPolicyDigest, 'readiness policy digest',
    ),
    verificationScope: 'trust-pins-and-quorum-math-only' as const,
    ...NON_AUTHORITY,
  };
  const configurationDigest = parseDigestV1(
    input.configurationDigest, 'authority configuration digest',
  );
  if (configurationDigest !== canonicalDigestHexV1({
    domain: POSTGRES_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
    configuration: body,
  })) throw new TypeError('authority configuration digest mismatch');
  return deepFreezeV1({ ...body, configurationDigest });
}

function parseProject(value: unknown) {
  const input = closedRecordV1(value, 'authority project');
  exactKeysV1(
    input, ['projectAuthorityDigest', 'principal', 'authenticationPolicyDigest'],
    'authority project',
  );
  return deepFreezeV1({
    projectAuthorityDigest: parseDigestV1(
      input.projectAuthorityDigest, 'project authority digest',
    ),
    principal: parsePrincipal(input.principal, 'project principal'),
    authenticationPolicyDigest: parseDigestV1(
      input.authenticationPolicyDigest, 'project authentication policy digest',
    ),
  });
}

function parseService(value: unknown) {
  const input = closedRecordV1(value, 'authority service');
  exactKeysV1(
    input, ['principal', 'endpointOrigin', 'tlsSpkiFingerprint', 'clientPolicyDigest'],
    'authority service',
  );
  return deepFreezeV1({
    principal: parsePrincipal(input.principal, 'service principal'),
    endpointOrigin: normalizePublicHttpsOrigin(input.endpointOrigin, 'service origin'),
    tlsSpkiFingerprint: parseDigestV1(input.tlsSpkiFingerprint, 'service TLS fingerprint'),
    clientPolicyDigest: parseDigestV1(input.clientPolicyDigest, 'service client policy digest'),
  });
}

function parseTransparencyLog(value: unknown) {
  const input = closedRecordV1(value, 'authority transparency log');
  exactKeysV1(input, [
    'principal', 'endpointOrigin', 'tlsSpkiFingerprint', 'publicCommitmentPolicyDigest',
  ], 'authority transparency log');
  return deepFreezeV1({
    principal: parsePrincipal(input.principal, 'transparency log principal'),
    endpointOrigin: normalizePublicHttpsOrigin(input.endpointOrigin, 'transparency log origin'),
    tlsSpkiFingerprint: parseDigestV1(
      input.tlsSpkiFingerprint, 'transparency log TLS fingerprint',
    ),
    publicCommitmentPolicyDigest: parseDigestV1(
      input.publicCommitmentPolicyDigest, 'public commitment policy digest',
    ),
  });
}

function parseWitnessPolicy(value: unknown, label: string): PostgresAuthorityWitnessPolicyV2 {
  const input = closedRecordV1(value, label);
  exactKeysV1(input, ['policyId', 'faultThreshold', 'quorumThreshold', 'members'], label);
  const entries = denseArrayV1(input.members, `${label} members`);
  if (entries.length > MAXIMUM_WITNESS_MEMBERS) throw new TypeError(`${label} is too large`);
  const members = entries.map((entry, index) =>
    parsePrincipal(entry, `${label} member[${index}]`));
  const ids = members.map(({ principalId }) => principalId);
  if (ids.some((id, index) => index > 0 && ids[index - 1]! >= id)) {
    throw new TypeError(`${label} roster order is invalid`);
  }
  const faultThreshold = parseUint64V1(input.faultThreshold, `${label} fault threshold`, 1n);
  const quorumThreshold = parseUint64V1(input.quorumThreshold, `${label} quorum threshold`, 1n);
  const n = BigInt(members.length);
  const f = BigInt(faultThreshold);
  const q = BigInt(quorumThreshold);
  if (n < BigInt(MINIMUM_WITNESS_MEMBERS) || n < (3n * f) + 1n
    || q < (2n * f) + 1n || q > n || (2n * q) <= n + f) {
    throw new TypeError(`${label} quorum math is invalid`);
  }
  return deepFreezeV1({
    policyId: parseOpaqueIdV1(input.policyId, `${label} ID`),
    faultThreshold,
    quorumThreshold,
    members,
  });
}

function parsePrincipal(value: unknown, label: string): PostgresAuthorityPrincipalV2 {
  const input = closedRecordV1(value, label);
  exactKeysV1(input, [
    'principalId', 'keyEpoch', 'keyFingerprint', 'policyDigest', 'administrationDigest',
  ], label);
  return deepFreezeV1({
    principalId: parseOpaqueIdV1(input.principalId, `${label} ID`),
    keyEpoch: parseUint64V1(input.keyEpoch, `${label} key epoch`, 1n),
    keyFingerprint: parseDigestV1(input.keyFingerprint, `${label} key fingerprint`),
    policyDigest: parseDigestV1(input.policyDigest, `${label} policy digest`),
    administrationDigest: parseDigestV1(
      input.administrationDigest, `${label} administration digest`,
    ),
  });
}

function parsePredecessor(value: unknown, epoch: string) {
  const input = closedRecordV1(value, 'authority configuration predecessor');
  exactKeysV1(input, ['kind', 'configurationDigest', 'headDigest'], 'configuration predecessor');
  if (epoch === '0') {
    if (input.kind !== 'genesis' || input.configurationDigest !== null
      || input.headDigest !== null) throw new TypeError('configuration predecessor is invalid');
    return Object.freeze({ kind: 'genesis' as const, configurationDigest: null, headDigest: null });
  }
  if (input.kind !== 'configuration-head') throw new TypeError('configuration predecessor invalid');
  return Object.freeze({
    kind: 'configuration-head' as const,
    configurationDigest: parseDigestV1(input.configurationDigest, 'predecessor config digest'),
    headDigest: parseDigestV1(input.headDigest, 'predecessor head digest'),
  });
}

function assertRoleSeparation(value: Readonly<{
  project: ReturnType<typeof parseProject>;
  service: ReturnType<typeof parseService>;
  transparencyLog: ReturnType<typeof parseTransparencyLog>;
  checkpointWitnesses: PostgresAuthorityWitnessPolicyV2;
  semanticWitnesses: PostgresAuthorityWitnessPolicyV2;
  initializationAnchor: PostgresAuthorityPrincipalV2;
  runnerEnrollment: PostgresAuthorityPrincipalV2;
  deploymentAttestor: PostgresAuthorityPrincipalV2;
}>): void {
  if (value.service.endpointOrigin === value.transparencyLog.endpointOrigin) {
    throw new TypeError('service and transparency-log origins must differ');
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
  if (new Set(values).size !== values.length) throw new TypeError(`${label} must be unique`);
}

function normalizePublicHttpsOrigin(value: unknown, label: string): string {
  if (typeof value !== 'string' || value.trim() === '') throw new TypeError(`${label} invalid`);
  let url: URL;
  try { url = new URL(value); }
  catch { throw new TypeError(`${label} must be an absolute URL`); }
  if (url.protocol !== 'https:' || url.username !== '' || url.password !== '' || url.port !== ''
    || (url.pathname !== '' && url.pathname !== '/') || url.search !== '' || url.hash !== ''
    || isNonPublicHostname(url.hostname)) {
    throw new TypeError(`${label} must be a public credential-free HTTPS origin`);
  }
  return url.origin;
}

function isNonPublicHostname(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, '');
  if (host === 'localhost' || host.endsWith('.localhost') || host.endsWith('.local')
    || host.endsWith('.internal') || host.endsWith('.home.arpa')
    || host.endsWith('.invalid') || host.endsWith('.test') || host.endsWith('.example')) {
    return true;
  }
  const family = isIP(host);
  if (family === 6) return true;
  if (family !== 4) return false;
  const [a = 0, b = 0] = host.split('.').map(Number);
  return a === 0 || a === 10 || a === 127 || (a === 100 && b >= 64 && b <= 127)
    || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && (b === 0 || b === 168)) || (a === 198 && b >= 18 && b <= 19)
    || (a === 198 && b === 51) || (a === 203 && b === 0) || a >= 224;
}

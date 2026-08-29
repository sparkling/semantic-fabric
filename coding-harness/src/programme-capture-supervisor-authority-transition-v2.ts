// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asClosedRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  parseProgrammeCaptureSupervisorAuthorityConfigurationV2,
  programmeCaptureSupervisorAuthorityGenesisHeadDigestV2,
} from './programme-capture-supervisor-authority-config-v2.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_MAX_BYTES_V2 = 16_384;
export const PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-authority-transition-digest-v2';

const UINT64_DECIMAL_PATTERN = /^(?:0|[1-9][0-9]{0,19})$/;
const MAX_UINT64 = 18_446_744_073_709_551_615n;

interface AuthorityTransitionNonAuthorityV2 {
  readonly externalAdministrationVerified: false;
  readonly deploymentAttestationVerified: false;
  readonly checkpointWitnessQuorumVerified: false;
  readonly semanticWitnessQuorumVerified: false;
  readonly stateTransitionAuthorized: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
}

export interface ProgrammeCaptureSupervisorAuthorityTransitionV2
extends AuthorityTransitionNonAuthorityV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly recordKind: 'supervisor-authority-transition-v2';
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly globalSequence: string;
  readonly predecessorHead: Readonly<{
    configurationEpoch: string;
    configurationDigest: string;
    headDigest: string;
  }>;
  readonly successorConfiguration: Readonly<{
    configurationEpoch: string;
    configurationDigest: string;
  }>;
  readonly verificationScope: 'configuration-adjacency-only';
  readonly transitionDigest: string;
}

export interface ProgrammeCaptureSupervisorAuthorityTransitionContextV2 {
  readonly predecessorConfiguration: unknown;
  readonly expectedPredecessorHeadDigest: string;
  readonly expectedGlobalSequence: string;
  readonly successorConfiguration: unknown;
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

export function parseProgrammeCaptureSupervisorAuthorityTransitionV2(
  value: unknown,
): ProgrammeCaptureSupervisorAuthorityTransitionV2 {
  const input = closedRecord(value, 'programme capture supervisor authority transition');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'recordKind', 'authority', 'globalSequence',
    'predecessorHead', 'successorConfiguration', 'verificationScope',
    ...NON_AUTHORITY_KEYS, 'transitionDigest',
  ], 'programme capture supervisor authority transition');
  assertIdentityAndScope(input);
  assertNonAuthority(input);

  const body = {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    recordKind: 'supervisor-authority-transition-v2' as const,
    authority: DEVELOPMENT_AUTHORITY,
    globalSequence: parseUint64(
      input.globalSequence, 'supervisor authority transition global sequence', 1n,
    ),
    predecessorHead: parsePredecessorHead(input.predecessorHead),
    successorConfiguration: parseSuccessorConfiguration(input.successorConfiguration),
    verificationScope: 'configuration-adjacency-only' as const,
    ...NON_AUTHORITY,
  };
  const transitionDigest = parseDigest(
    input.transitionDigest, 'supervisor authority transition digest',
  );
  if (transitionDigest !== digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_DIGEST_DOMAIN_V2,
    transition: body,
  })) throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_DIGEST_MISMATCH');
  return deepFreeze({ ...body, transitionDigest });
}

export function serializeProgrammeCaptureSupervisorAuthorityTransitionV2(
  value: unknown,
): string {
  return `${JSON.stringify(
    parseProgrammeCaptureSupervisorAuthorityTransitionV2(value), null, 2,
  )}\n`;
}

export function parseProgrammeCaptureSupervisorAuthorityTransitionBlobV2(
  serialized: string,
): ProgrammeCaptureSupervisorAuthorityTransitionV2 {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8')
      > PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_MAX_BYTES_V2
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError('supervisor authority transition must be bounded canonical UTF-8 JSON');
  }
  const parsed = parseProgrammeCaptureSupervisorAuthorityTransitionV2(
    parseJsonWithoutDuplicateKeys(serialized, 'supervisor authority transition'),
  );
  if (serializeProgrammeCaptureSupervisorAuthorityTransitionV2(parsed) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_CANONICAL_REQUIRED');
  }
  return parsed;
}

export function verifyProgrammeCaptureSupervisorAuthorityTransitionV2(
  value: unknown,
  contextValue: unknown,
): ProgrammeCaptureSupervisorAuthorityTransitionV2 {
  const transition = parseProgrammeCaptureSupervisorAuthorityTransitionV2(value);
  const context = closedRecord(
    contextValue, 'programme capture supervisor authority transition context',
  );
  assertExactKeys(context, [
    'predecessorConfiguration', 'expectedPredecessorHeadDigest',
    'expectedGlobalSequence', 'successorConfiguration',
  ], 'programme capture supervisor authority transition context');
  const predecessor = parseProgrammeCaptureSupervisorAuthorityConfigurationV2(
    context.predecessorConfiguration,
  );
  const successor = parseProgrammeCaptureSupervisorAuthorityConfigurationV2(
    context.successorConfiguration,
  );
  const expectedHeadDigest = parseDigest(
    context.expectedPredecessorHeadDigest, 'expected predecessor authority head digest',
  );
  const expectedGlobalSequence = parseUint64(
    context.expectedGlobalSequence, 'expected authority transition global sequence', 1n,
  );
  if (transition.globalSequence !== expectedGlobalSequence) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_GLOBAL_SEQUENCE_MISMATCH');
  }

  const predecessorEpoch = BigInt(predecessor.configurationEpoch);
  if (predecessorEpoch === MAX_UINT64) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_EPOCH_OVERFLOW');
  }
  const successorEpoch = (predecessorEpoch + 1n).toString();
  if (successor.configurationEpoch !== successorEpoch
    || transition.successorConfiguration.configurationEpoch !== successorEpoch) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_EPOCH_SEQUENCE_INVALID');
  }

  if (transition.predecessorHead.configurationEpoch !== predecessor.configurationEpoch
    || transition.predecessorHead.configurationDigest !== predecessor.configurationDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_PREDECESSOR_CONFIGURATION_MISMATCH');
  }
  if (transition.predecessorHead.headDigest !== expectedHeadDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_PREDECESSOR_HEAD_MISMATCH');
  }
  if (predecessorEpoch === 0n
    && expectedHeadDigest
      !== programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(predecessor)) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_GENESIS_HEAD_MISMATCH');
  }

  if (successor.predecessor.kind !== 'configuration-head'
    || successor.predecessor.configurationDigest !== predecessor.configurationDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_PREDECESSOR_CONFIGURATION_MISMATCH');
  }
  if (successor.predecessor.headDigest !== expectedHeadDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_PREDECESSOR_HEAD_MISMATCH');
  }
  if (transition.successorConfiguration.configurationDigest
    !== successor.configurationDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_SUCCESSOR_CONFIGURATION_MISMATCH');
  }
  return transition;
}

function parsePredecessorHead(value: unknown) {
  const input = closedRecord(value, 'supervisor authority transition predecessor head');
  assertExactKeys(input, [
    'configurationEpoch', 'configurationDigest', 'headDigest',
  ], 'supervisor authority transition predecessor head');
  return Object.freeze({
    configurationEpoch: parseUint64(
      input.configurationEpoch, 'transition predecessor configuration epoch', 0n,
    ),
    configurationDigest: parseDigest(
      input.configurationDigest, 'transition predecessor configuration digest',
    ),
    headDigest: parseDigest(input.headDigest, 'transition predecessor authority head digest'),
  });
}

function parseSuccessorConfiguration(value: unknown) {
  const input = closedRecord(value, 'supervisor authority transition successor configuration');
  assertExactKeys(input, [
    'configurationEpoch', 'configurationDigest',
  ], 'supervisor authority transition successor configuration');
  return Object.freeze({
    configurationEpoch: parseUint64(
      input.configurationEpoch, 'transition successor configuration epoch', 1n,
    ),
    configurationDigest: parseDigest(
      input.configurationDigest, 'transition successor configuration digest',
    ),
  });
}

function assertIdentityAndScope(input: Record<string, unknown>): void {
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.recordKind !== 'supervisor-authority-transition-v2'
    || input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_IDENTITY_INVALID');
  }
  if (input.verificationScope !== 'configuration-adjacency-only') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_SCOPE_INVALID');
  }
}

function assertNonAuthority(input: Record<string, unknown>): void {
  if (NON_AUTHORITY_KEYS.some(
    (key) => input[key] !== NON_AUTHORITY[key as keyof typeof NON_AUTHORITY],
  )) throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_AUTHORITY_ESCALATION');
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

function closedRecord(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}

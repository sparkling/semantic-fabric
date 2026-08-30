// SPDX-License-Identifier: MIT

import { canonical } from '@metaharness/harness';
import { isProxy } from 'node:util/types';
import {
  SHA256_PATTERN,
  asClosedRecord,
  assertExactKeys,
  deepFreeze,
  snapshotUint8Array,
} from './contracts.js';
import {
  parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2,
  parseProgrammeCaptureSupervisorAuthorityConfigurationV2,
} from './programme-capture-supervisor-authority-config-v2.js';
import {
  parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2,
} from './programme-capture-supervisor-run-event-codec-v2.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_MAX_BYTES_V2 = 1_024;
export const PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-public-event-commitment-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_TRANSPARENCY_LOG_IDENTITY_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-transparency-log-identity-digest-v2';

export interface ProgrammeCaptureSupervisorPublicCommitmentV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly leafKind: 'programme-capture-event-commitment-v2';
  readonly logIdentityDigest: string;
  readonly eventDigest: string;
}

export function buildProgrammeCaptureSupervisorPublicCommitmentV2(
  value: unknown,
): ProgrammeCaptureSupervisorPublicCommitmentV2 {
  const input = closed(value, 'supervisor public commitment input');
  assertExactKeys(
    input, ['serializedAuthorityConfiguration', 'serializedEventEnvelope'],
    'supervisor public commitment input',
  );
  if (typeof input.serializedAuthorityConfiguration !== 'string'
    || typeof input.serializedEventEnvelope !== 'string') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_INPUT_INVALID');
  }
  const configuration = parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2(
    input.serializedAuthorityConfiguration,
  );
  const envelope = parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2(
    input.serializedEventEnvelope,
  );
  if (envelope.event.authorityHead.configurationEpoch !== configuration.configurationEpoch
    || envelope.event.authorityHead.configurationDigest !== configuration.configurationDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_AUTHORITY_MISMATCH');
  }
  return normalizedCommitment({
    logIdentityDigest: programmeCaptureSupervisorTransparencyLogIdentityDigestV2(configuration),
    eventDigest: envelope.event.eventDigest,
  });
}

export function parseProgrammeCaptureSupervisorPublicCommitmentV2(
  value: unknown,
): ProgrammeCaptureSupervisorPublicCommitmentV2 {
  const input = closed(value, 'supervisor public commitment');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'leafKind', 'logIdentityDigest', 'eventDigest',
  ], 'supervisor public commitment');
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.leafKind !== 'programme-capture-event-commitment-v2') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_IDENTITY_INVALID');
  }
  return normalizedCommitment(input);
}

export function serializeProgrammeCaptureSupervisorPublicCommitmentV2(
  value: unknown,
): string {
  return `${JSON.stringify(
    parseProgrammeCaptureSupervisorPublicCommitmentV2(value), null, 2,
  )}\n`;
}

export function parseProgrammeCaptureSupervisorPublicCommitmentBlobV2(
  serialized: string,
): ProgrammeCaptureSupervisorPublicCommitmentV2 {
  assertCanonicalBlob(serialized);
  const commitment = parseProgrammeCaptureSupervisorPublicCommitmentV2(
    parseJsonWithoutDuplicateKeys(serialized, 'supervisor public commitment'),
  );
  if (serializeProgrammeCaptureSupervisorPublicCommitmentV2(commitment) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_CANONICAL_REQUIRED');
  }
  return commitment;
}

export function programmeCaptureSupervisorPublicCommitmentDigestV2(
  value: unknown,
): string {
  return digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2,
    commitment: parseProgrammeCaptureSupervisorPublicCommitmentV2(value),
  });
}

export function programmeCaptureSupervisorPublicCommitmentLeafBytesV2(
  value: unknown,
): Buffer {
  return Buffer.from(canonical({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2,
    commitment: parseProgrammeCaptureSupervisorPublicCommitmentV2(value),
  }), 'utf8');
}

export function parseProgrammeCaptureSupervisorPublicCommitmentLeafBytesV2(
  value: unknown,
): ProgrammeCaptureSupervisorPublicCommitmentV2 {
  const snapshot = snapshotUint8Array(
    value, 'supervisor public commitment leaf bytes',
    PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_MAX_BYTES_V2,
  );
  const serialized = decodeLeafBytes(snapshot);
  const outer = closed(
    parseJsonWithoutDuplicateKeys(serialized, 'supervisor public commitment leaf'),
    'supervisor public commitment leaf',
  );
  assertExactKeys(outer, ['domain', 'commitment'], 'supervisor public commitment leaf');
  if (outer.domain !== PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_INVALID');
  }
  const commitment = parseProgrammeCaptureSupervisorPublicCommitmentV2(outer.commitment);
  if (!Buffer.from(snapshot).equals(
    programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment),
  )) throw new Error('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_LEAF_CANONICAL_REQUIRED');
  return commitment;
}

export function programmeCaptureSupervisorTransparencyLogIdentityDigestV2(
  authorityConfiguration: unknown,
): string {
  const configuration = parseProgrammeCaptureSupervisorAuthorityConfigurationV2(
    authorityConfiguration,
  );
  return digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_TRANSPARENCY_LOG_IDENTITY_DIGEST_DOMAIN_V2,
    transparencyLog: configuration.transparencyLog,
  });
}

function normalizedCommitment(
  input: Record<string, unknown>,
): ProgrammeCaptureSupervisorPublicCommitmentV2 {
  return deepFreeze({
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    leafKind: 'programme-capture-event-commitment-v2' as const,
    logIdentityDigest: parseDigest(
      input.logIdentityDigest, 'supervisor public commitment log identity digest',
    ),
    eventDigest: parseDigest(
      input.eventDigest, 'supervisor public commitment event digest',
    ),
  });
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

function closed(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}

function assertCanonicalBlob(serialized: string): void {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8')
      > PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_MAX_BYTES_V2
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_CANONICAL_INVALID');
  }
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}

function decodeLeafBytes(value: Uint8Array): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(value); }
  catch {
    throw new TypeError('supervisor public commitment leaf must be canonical UTF-8');
  }
}

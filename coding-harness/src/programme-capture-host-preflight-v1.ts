// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  asInteger,
  assertExactKeys,
  deepFreeze,
  snapshotUint8Array,
} from './contracts.js';
import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import {
  PROGRAMME_CAPTURE_HOST_FIELD_NAMES_V1,
  isFixedProgrammeCaptureHostObservationV1,
  parseProgrammeCaptureCpuListV1,
  parseProgrammeCaptureHostObservationV1,
  programmeCaptureHostObservedValuesV1,
  type ProgrammeCaptureHostObservationV1,
  type ProgrammeCaptureHostSnapshotV1,
} from './programme-capture-host-observation-v1.js';
import {
  parseProgrammeCaptureInputAttestationV1,
  type ProgrammeCaptureInputAttestationV1,
} from './programme-capture-input-attestation-record-v1.js';
import {
  PROGRAMME_CAPTURE_RUNNER_PROFILE_MAX_BYTES_V1,
  parseProgrammeCaptureRunnerProfileV1,
  type ProgrammeCaptureRunnerProfileV1,
} from './programme-capture-runner-profile-v1.js';
import {
  parseProgrammeCaptureStateV1,
  transitionProgrammeCaptureStateV1,
  type ProgrammeCaptureStateV1,
} from './programme-capture-state-v1.js';
import { PROGRAMME_CAPTURE_PROFILE_PATH } from './programme-capture-task-v1.js';
import { digestValue } from './receipts.js';
export {
  collectProgrammeCaptureHostDiagnosticObservationV1,
  collectProgrammeCaptureHostObservationV1,
  parseProgrammeCaptureHostObservationV1,
  rustArchitecture,
  type ProgrammeCaptureHostObservationV1,
  type ProgrammeCaptureHostSurfaceSourceV1,
} from './programme-capture-host-observation-v1.js';
const REASON_ORDER = [
  'profile-authority-bytes-unavailable',
  'profile-authority-mismatch',
  'profile-invalid',
  'profile-explicitly-uncontrolled',
  'profile-control-contract-invalid',
  'required-observation-unavailable',
  'observation-changed',
  'profile-static-mismatch',
  'cpu-list-invalid',
  'allowed-cpus-empty',
  'allowed-cpus-offline',
  'allowed-cpus-not-isolated',
  'governor-not-performance',
  'turbo-not-disabled',
  'swap-enabled',
  'load-exceeds-profile',
  'positive-control-closure-incomplete',
] as const;
export type ProgrammeCaptureHostNonAdmissionReasonV1 = typeof REASON_ORDER[number];
const INELIGIBLE_REASONS = new Set<ProgrammeCaptureHostNonAdmissionReasonV1>([
  'profile-explicitly-uncontrolled', 'profile-control-contract-invalid',
  'profile-static-mismatch', 'allowed-cpus-empty', 'allowed-cpus-offline',
  'allowed-cpus-not-isolated', 'governor-not-performance', 'turbo-not-disabled',
  'swap-enabled', 'load-exceeds-profile',
]);

export interface ProgrammeCaptureHostNonAdmissionV1 {
  readonly schemaVersion: 1;
  readonly transactionKind: 'programme-capture-v1';
  readonly evidenceKind: 'host-preflight-non-admission-v1';
  readonly authority: 'controller-classified-non-admission'
    | 'diagnostic-classified-non-admission';
  readonly runId: string;
  readonly taskDigest: string;
  readonly claimDigest: string;
  readonly controller: Readonly<{ commit: string; tree: string }>;
  readonly inputAttestationDigest: string;
  readonly beforeStateHead: string;
  readonly profileAuthority: Readonly<{
    path: typeof PROGRAMME_CAPTURE_PROFILE_PATH;
    gitBlobId: string;
    sha256: string;
    byteLength: number;
  }>;
  readonly profileObservation: Readonly<{
    status: 'verified' | 'unavailable' | 'mismatch' | 'invalid';
    sha256: string | null;
    byteLength: number | null;
  }>;
  readonly hostObservation: ProgrammeCaptureHostObservationV1;
  readonly outcome: 'ineligible' | 'unproven';
  readonly reasons: readonly ProgrammeCaptureHostNonAdmissionReasonV1[];
  readonly captureAuthorized: false;
  readonly recordDigest: string;
}

export function diagnoseProgrammeCaptureHostObservationV1(
  value: unknown,
): Readonly<{
  authority: 'diagnostic-only-no-admission';
  observationDigest: string;
  outcome: 'ineligible' | 'unproven';
  reasons: readonly ProgrammeCaptureHostNonAdmissionReasonV1[];
  captureAuthorized: false;
}> {
  const observation = parseProgrammeCaptureHostObservationV1(value);
  const reasons = assess(undefined, null, observation);
  const outcome = reasons.some((reason) => INELIGIBLE_REASONS.has(reason))
    ? 'ineligible' as const : 'unproven' as const;
  return deepFreeze({
    authority: 'diagnostic-only-no-admission',
    observationDigest: observation.observationDigest,
    outcome,
    reasons,
    captureAuthorized: false,
  });
}

export function rejectProgrammeCaptureHostPreflightV1(value: Readonly<{
  state: ProgrammeCaptureStateV1;
  inputAttestation: ProgrammeCaptureInputAttestationV1;
  profileBytes: Uint8Array | undefined;
  observation: ProgrammeCaptureHostObservationV1;
}>): Readonly<{
  record: ProgrammeCaptureHostNonAdmissionV1;
  state: ProgrammeCaptureStateV1;
}> {
  const input = asClosedRecord(value, 'programme capture host rejection input');
  assertExactKeys(
    input, ['state', 'inputAttestation', 'profileBytes', 'observation'],
    'programme capture host rejection input',
  );
  const state = parseProgrammeCaptureStateV1(input.state);
  const attestation = parseProgrammeCaptureInputAttestationV1(input.inputAttestation);
  const fixedObservation = isFixedProgrammeCaptureHostObservationV1(input.observation);
  const observation = parseProgrammeCaptureHostObservationV1(input.observation);
  const profileBytes = copyProfileBytes(input.profileBytes);
  if (state.phase !== 'inputs-attested' || state.captureAttempts !== 0) {
    throw new Error('HARNESS_CAPTURE_HOST_INPUTS_ATTESTED_REQUIRED');
  }
  const head = state.events.at(-1);
  if (head?.kind !== 'attest-inputs'
    || head.evidenceDigest !== attestation.attestationDigest
    || state.taskDigest !== attestation.task.valueDigest
    || state.controller.commit !== attestation.controller.commit
    || state.controller.tree !== attestation.controller.tree) {
    throw new Error('HARNESS_CAPTURE_HOST_INPUT_ATTESTATION_MISMATCH');
  }
  const profileAuthority = attestation.protectedInputs[0];
  if (profileAuthority?.path !== PROGRAMME_CAPTURE_PROFILE_PATH) {
    throw new Error('HARNESS_CAPTURE_HOST_PROFILE_AUTHORITY_MISSING');
  }
  const profileResult = inspectProfile(profileBytes, profileAuthority);
  const reasons = assess(profileResult.profile, profileResult.reason, observation);
  const outcome = reasons.some((reason) => INELIGIBLE_REASONS.has(reason))
    ? 'ineligible' as const : 'unproven' as const;
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    evidenceKind: 'host-preflight-non-admission-v1' as const,
    authority: fixedObservation ? 'controller-classified-non-admission' as const
      : 'diagnostic-classified-non-admission' as const,
    runId: state.runId,
    taskDigest: state.taskDigest,
    claimDigest: state.claimDigest,
    controller: state.controller,
    inputAttestationDigest: attestation.attestationDigest,
    beforeStateHead: head.digest,
    profileAuthority,
    profileObservation: profileResult.observation,
    hostObservation: observation,
    outcome,
    reasons,
    captureAuthorized: false as const,
  };
  const record = parseProgrammeCaptureHostNonAdmissionV1({
    ...body,
    recordDigest: digestValue(body),
  });
  const failed = transitionProgrammeCaptureStateV1(state, {
    kind: 'fail',
    evidenceDigest: record.recordDigest,
    reasonDigest: digestValue({ outcome, reasons }),
    processDispositionDigest: digestValue({
      measurementProcess: 'not-authorized', modelProcess: 'not-authorized',
    }),
    egressDispositionDigest: digestValue({ providerEgress: 'not-authorized' }),
    leaseDispositionDigest: digestValue({ runnerLease: 'not-acquired' }),
  });
  verifyProgrammeCaptureHostNonAdmissionV1({
    record,
    beforeState: state,
    afterState: failed,
    inputAttestation: attestation,
    profileBytes,
  });
  return deepFreeze({ record, state: failed });
}

export function verifyProgrammeCaptureHostNonAdmissionV1(value: Readonly<{
  record: ProgrammeCaptureHostNonAdmissionV1;
  beforeState: ProgrammeCaptureStateV1;
  afterState: ProgrammeCaptureStateV1;
  inputAttestation: ProgrammeCaptureInputAttestationV1;
  profileBytes: Uint8Array | undefined;
}>): ProgrammeCaptureHostNonAdmissionV1 {
  const input = asClosedRecord(value, 'programme capture host non-admission verification input');
  assertExactKeys(
    input, ['record', 'beforeState', 'afterState', 'inputAttestation', 'profileBytes'],
    'programme capture host non-admission verification input',
  );
  const record = parseProgrammeCaptureHostNonAdmissionV1(input.record);
  const before = parseProgrammeCaptureStateV1(input.beforeState);
  const after = parseProgrammeCaptureStateV1(input.afterState);
  const attestation = parseProgrammeCaptureInputAttestationV1(input.inputAttestation);
  const profileBytes = copyProfileBytes(input.profileBytes);
  const head = before.events.at(-1), profile = attestation.protectedInputs[0];
  if (before.phase !== 'inputs-attested' || before.captureAttempts !== 0
    || head?.kind !== 'attest-inputs'
    || record.runId !== before.runId || record.taskDigest !== before.taskDigest
    || record.claimDigest !== before.claimDigest || record.beforeStateHead !== head.digest
    || record.controller.commit !== before.controller.commit
    || record.controller.tree !== before.controller.tree
    || record.inputAttestationDigest !== attestation.attestationDigest
    || head.evidenceDigest !== attestation.attestationDigest
    || before.taskDigest !== attestation.task.valueDigest
    || JSON.stringify(record.controller) !== JSON.stringify(attestation.controller)
    || JSON.stringify(record.profileAuthority) !== JSON.stringify(profile)) {
    throw new Error('HARNESS_CAPTURE_HOST_NON_ADMISSION_AUTHORITY_MISMATCH');
  }
  const inspected = inspectProfile(profileBytes, record.profileAuthority);
  const reasons = assess(inspected.profile, inspected.reason, record.hostObservation);
  if (JSON.stringify(inspected.observation) !== JSON.stringify(record.profileObservation)
    || JSON.stringify(reasons) !== JSON.stringify(record.reasons)) {
    throw new Error('HARNESS_CAPTURE_HOST_NON_ADMISSION_SEMANTICS_MISMATCH');
  }
  const terminal = after.events.at(-1);
  if (after.phase !== 'failed' || after.captureAttempts !== 0
    || after.runId !== before.runId || after.taskDigest !== before.taskDigest
    || after.claimDigest !== before.claimDigest
    || JSON.stringify(after.controller) !== JSON.stringify(before.controller)
    || after.events.length !== before.events.length + 1
    || JSON.stringify(after.events.slice(0, -1)) !== JSON.stringify(before.events)
    || terminal?.kind !== 'fail' || terminal.evidenceDigest !== record.recordDigest
    || terminal.reasonDigest !== digestValue({ outcome: record.outcome, reasons: record.reasons })
    || terminal.processDispositionDigest !== digestValue({
      measurementProcess: 'not-authorized', modelProcess: 'not-authorized',
    })
    || terminal.egressDispositionDigest !== digestValue({ providerEgress: 'not-authorized' })
    || terminal.leaseDispositionDigest !== digestValue({ runnerLease: 'not-acquired' })) {
    throw new Error('HARNESS_CAPTURE_HOST_NON_ADMISSION_TERMINAL_MISMATCH');
  }
  return record;
}

export function parseProgrammeCaptureHostNonAdmissionV1(
  value: unknown,
): ProgrammeCaptureHostNonAdmissionV1 {
  const input = asClosedRecord(value, 'programme capture host non-admission');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'evidenceKind', 'authority', 'runId',
    'taskDigest', 'claimDigest', 'controller', 'inputAttestationDigest',
    'beforeStateHead', 'profileAuthority', 'profileObservation', 'hostObservation',
    'outcome', 'reasons', 'captureAuthorized', 'recordDigest',
  ], 'programme capture host non-admission');
  if (input.schemaVersion !== 1 || input.transactionKind !== 'programme-capture-v1'
    || input.evidenceKind !== 'host-preflight-non-admission-v1'
    || (input.authority !== 'controller-classified-non-admission'
      && input.authority !== 'diagnostic-classified-non-admission')
    || (input.outcome !== 'ineligible' && input.outcome !== 'unproven')
    || input.captureAuthorized !== false) {
    throw new TypeError('HARNESS_CAPTURE_HOST_NON_ADMISSION_IDENTITY_INVALID');
  }
  const controller = gitIdentity(input.controller);
  const profileAuthority = profileIdentity(input.profileAuthority, controller.commit.length);
  const profileObservation = parseProfileObservation(input.profileObservation);
  assertProfileObservationConsistency(profileAuthority, profileObservation);
  const reasons = parseReasons(input.reasons);
  const expectedOutcome = reasons.some((reason) => INELIGIBLE_REASONS.has(reason))
    ? 'ineligible' : 'unproven';
  if (input.outcome !== expectedOutcome) {
    throw new Error('HARNESS_CAPTURE_HOST_NON_ADMISSION_OUTCOME_INVALID');
  }
  const hostObservation = parseProgrammeCaptureHostObservationV1(input.hostObservation);
  if ((input.authority === 'controller-classified-non-admission') !==
    (hostObservation.observerKind === 'controller-read-only-proc-sysfs-v1')) {
    throw new TypeError('HARNESS_CAPTURE_HOST_NON_ADMISSION_OBSERVER_AUTHORITY_INVALID');
  }
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    evidenceKind: 'host-preflight-non-admission-v1' as const,
    authority: input.authority,
    runId: parseTaskOpaqueId(input.runId, 'programme capture host non-admission.runId'),
    taskDigest: nonzeroDigest(input.taskDigest, 'task'),
    claimDigest: nonzeroDigest(input.claimDigest, 'claim'),
    controller,
    inputAttestationDigest: nonzeroDigest(input.inputAttestationDigest, 'input attestation'),
    beforeStateHead: nonzeroDigest(input.beforeStateHead, 'before state head'),
    profileAuthority,
    profileObservation,
    hostObservation,
    outcome: input.outcome,
    reasons,
    captureAuthorized: false as const,
  };
  const recordDigest = nonzeroDigest(input.recordDigest, 'host non-admission record');
  if (recordDigest !== digestValue(body)) {
    throw new Error('HARNESS_CAPTURE_HOST_NON_ADMISSION_DIGEST_MISMATCH');
  }
  return deepFreeze({ ...body, recordDigest }) as ProgrammeCaptureHostNonAdmissionV1;
}

interface ProfileInspection {
  readonly profile?: ProgrammeCaptureRunnerProfileV1;
  readonly reason: ProgrammeCaptureHostNonAdmissionReasonV1 | null;
  readonly observation: ProgrammeCaptureHostNonAdmissionV1['profileObservation'];
}

function inspectProfile(
  value: unknown,
  authority: ProgrammeCaptureInputAttestationV1['protectedInputs'][number],
): ProfileInspection {
  if (value === undefined) return {
    reason: 'profile-authority-bytes-unavailable',
    observation: { status: 'unavailable', sha256: null, byteLength: null },
  };
  if (!(value instanceof Uint8Array)) throw new TypeError('HARNESS_CAPTURE_HOST_PROFILE_BYTES_INVALID');
  const sha256 = createHash('sha256').update(value).digest('hex');
  const observed = { sha256, byteLength: value.byteLength };
  if (sha256 !== authority.sha256 || value.byteLength !== authority.byteLength) return {
    reason: 'profile-authority-mismatch', observation: { status: 'mismatch', ...observed },
  };
  try {
    return { profile: parseProgrammeCaptureRunnerProfileV1(value), reason: null,
      observation: { status: 'verified', ...observed } };
  } catch {
    return { reason: 'profile-invalid', observation: { status: 'invalid', ...observed } };
  }
}

function assess(
  profile: ProgrammeCaptureRunnerProfileV1 | undefined,
  profileReason: ProgrammeCaptureHostNonAdmissionReasonV1 | null,
  observation: ProgrammeCaptureHostObservationV1,
): ProgrammeCaptureHostNonAdmissionReasonV1[] {
  const reasons = new Set<ProgrammeCaptureHostNonAdmissionReasonV1>();
  if (profileReason !== null) reasons.add(profileReason);
  if (profile !== undefined) {
    if (!profile.controlled) reasons.add('profile-explicitly-uncontrolled');
    if (profile.os !== 'linux' || profile.scalingGovernor !== 'performance'
      || profile.turbo !== 'disabled' || profile.swapTotalKib !== 0
      || profile.buildProfile !== 'release') reasons.add('profile-control-contract-invalid');
  }
  const [first, second] = observation.samples;
  if (first.controlDigest !== second.controlDigest) reasons.add('observation-changed');
  for (const snapshot of observation.samples) assessSnapshot(snapshot, observation, profile, reasons);
  if (reasons.size === 0) reasons.add('positive-control-closure-incomplete');
  return REASON_ORDER.filter((reason) => reasons.has(reason));
}

function assessSnapshot(
  snapshot: ProgrammeCaptureHostSnapshotV1,
  observation: ProgrammeCaptureHostObservationV1,
  profile: ProgrammeCaptureRunnerProfileV1 | undefined,
  reasons: Set<ProgrammeCaptureHostNonAdmissionReasonV1>,
): void {
  if (PROGRAMME_CAPTURE_HOST_FIELD_NAMES_V1.some(
    (name) => snapshot.fields[name].status !== 'observed',
  )) reasons.add('required-observation-unavailable');
  const values = programmeCaptureHostObservedValuesV1(snapshot);
  if (observation.architecture === 'unmapped') reasons.add('required-observation-unavailable');
  if (profile !== undefined && [
    [profile.os, observation.platform],
    ...(observation.architecture === 'unmapped'
      ? [] : [[profile.architecture, observation.architecture]]),
    [profile.kernelRelease, values.kernelRelease], [profile.cpuModel, values.cpuModel],
    [profile.onlineCpus, values.onlineCpus], [profile.allowedCpus, values.allowedCpus],
    [profile.isolatedCpus, values.isolatedCpus],
    [profile.scalingGovernor, values.scalingGovernor], [profile.turbo, values.turbo],
    [String(profile.swapTotalKib), values.swapTotalKib],
    [String(profile.memTotalKib), values.memTotalKib],
  ].some(([expected, actual]) => actual !== undefined && expected !== actual)) {
    reasons.add('profile-static-mismatch');
  }
  if (values.scalingGovernor !== undefined && values.scalingGovernor !== 'performance') {
    reasons.add('governor-not-performance');
  }
  if (values.turbo !== undefined && values.turbo !== 'disabled') reasons.add('turbo-not-disabled');
  if (values.swapTotalKib !== undefined && values.swapTotalKib !== '0') reasons.add('swap-enabled');
  if (profile !== undefined && values.load1Milli !== undefined
    && Number(values.load1Milli) > profile.load1LimitMilli) reasons.add('load-exceeds-profile');
  const cpuList = (text: string | undefined, allowEmpty = false): Set<number> | undefined => {
    try { return parseProgrammeCaptureCpuListV1(text, allowEmpty); }
    catch { reasons.add('cpu-list-invalid'); return undefined; }
  };
  const allowed = cpuList(values.allowedCpus, true);
  const online = cpuList(values.onlineCpus);
  const isolated = cpuList(values.isolatedCpus, true);
  if (allowed?.size === 0) reasons.add('allowed-cpus-empty');
  if (allowed && online && ![...allowed].every((value) => online.has(value)))
    reasons.add('allowed-cpus-offline');
  if (allowed && isolated && ![...allowed].every((value) => isolated.has(value)))
    reasons.add('allowed-cpus-not-isolated');
}

function parseReasons(value: unknown): ProgrammeCaptureHostNonAdmissionReasonV1[] {
  const reasons = asDenseArray(value, 'programme capture host non-admission.reasons');
  if (reasons.length === 0 || reasons.length > REASON_ORDER.length
    || reasons.some((reason) => !REASON_ORDER.includes(reason as ProgrammeCaptureHostNonAdmissionReasonV1))) {
    throw new TypeError('HARNESS_CAPTURE_HOST_NON_ADMISSION_REASONS_INVALID');
  }
  const parsed = reasons as ProgrammeCaptureHostNonAdmissionReasonV1[];
  const canonical = REASON_ORDER.filter((reason) => parsed.includes(reason));
  if (new Set(parsed).size !== parsed.length || JSON.stringify(parsed) !== JSON.stringify(canonical)) {
    throw new TypeError('HARNESS_CAPTURE_HOST_NON_ADMISSION_REASONS_NOT_CANONICAL');
  }
  return [...parsed];
}

function parseProfileObservation(value: unknown): ProgrammeCaptureHostNonAdmissionV1['profileObservation'] {
  const input = asClosedRecord(value, 'programme capture profile observation');
  assertExactKeys(input, ['status', 'sha256', 'byteLength'], 'programme capture profile observation');
  if (typeof input.status !== 'string'
    || !['verified', 'unavailable', 'mismatch', 'invalid'].includes(input.status)) {
    throw new TypeError('HARNESS_CAPTURE_HOST_PROFILE_OBSERVATION_INVALID');
  }
  if (input.status === 'unavailable') {
    if (input.sha256 !== null || input.byteLength !== null) {
      throw new TypeError('HARNESS_CAPTURE_HOST_PROFILE_OBSERVATION_INVALID');
    }
    return { status: 'unavailable', sha256: null, byteLength: null };
  }
  return { status: input.status as 'verified' | 'mismatch' | 'invalid',
    sha256: nonzeroDigest(input.sha256, 'observed profile'),
    byteLength: asInteger(input.byteLength, 'observed profile byteLength') };
}

function assertProfileObservationConsistency(
  authority: ProgrammeCaptureHostNonAdmissionV1['profileAuthority'],
  observation: ProgrammeCaptureHostNonAdmissionV1['profileObservation'],
): void {
  if (observation.status === 'unavailable') return;
  const matches = observation.sha256 === authority.sha256
    && observation.byteLength === authority.byteLength;
  if ((observation.status === 'mismatch') === matches
    || ((observation.status === 'verified' || observation.status === 'invalid') && !matches)) {
    throw new Error('HARNESS_CAPTURE_HOST_PROFILE_OBSERVATION_INCONSISTENT');
  }
}

function profileIdentity(value: unknown, objectLength: number) {
  const input = asClosedRecord(value, 'programme capture host profile authority');
  assertExactKeys(input, ['path', 'gitBlobId', 'sha256', 'byteLength'], 'programme capture host profile authority');
  if (input.path !== PROGRAMME_CAPTURE_PROFILE_PATH || typeof input.gitBlobId !== 'string'
    || !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(input.gitBlobId)
    || input.gitBlobId.length !== objectLength) {
    throw new TypeError('HARNESS_CAPTURE_HOST_PROFILE_AUTHORITY_INVALID');
  }
  return { path: PROGRAMME_CAPTURE_PROFILE_PATH, gitBlobId: input.gitBlobId,
    sha256: nonzeroDigest(input.sha256, 'profile authority'),
    byteLength: asInteger(input.byteLength, 'profile authority byteLength') };
}

function copyProfileBytes(value: unknown): Uint8Array | undefined {
  if (value === undefined) return undefined;
  try {
    return snapshotUint8Array(value, 'programme capture host profile bytes',
      PROGRAMME_CAPTURE_RUNNER_PROFILE_MAX_BYTES_V1);
  } catch {
    throw new TypeError('HARNESS_CAPTURE_HOST_PROFILE_BYTES_INVALID');
  }
}

function gitIdentity(value: unknown) {
  const input = asClosedRecord(value, 'programme capture host controller');
  assertExactKeys(input, ['commit', 'tree'], 'programme capture host controller');
  if (typeof input.commit !== 'string' || typeof input.tree !== 'string'
    || !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(input.commit)
    || !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(input.tree)
    || input.commit.length !== input.tree.length) {
    throw new TypeError('HARNESS_CAPTURE_HOST_CONTROLLER_INVALID');
  }
  return { commit: input.commit, tree: input.tree };
}

function nonzeroDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`HARNESS_CAPTURE_HOST_${label.toUpperCase()}_DIGEST_INVALID`);
  }
  return value;
}

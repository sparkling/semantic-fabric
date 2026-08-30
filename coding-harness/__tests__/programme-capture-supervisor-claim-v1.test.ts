// SPDX-License-Identifier: MIT
import { canonical } from '@metaharness/harness';
import { createHash, generateKeyPairSync, sign } from 'node:crypto';
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1 } from '../src/programme-capture-config-v1.js';
import {
  reserveProgrammeCaptureRunClaimV1,
  type ProgrammeCaptureRunClaimAuthorityInputV1,
} from '../src/programme-capture-claim-io-v1.js';
import { programmeCaptureRunClaimKeyDigestV1 }
  from '../src/programme-capture-claim-record-v1.js';
import { parseProgrammeCaptureStateV1 } from '../src/programme-capture-state-v1.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_ACK_DIGEST_DOMAIN_V1,
  PROGRAMME_CAPTURE_SUPERVISOR_ACK_SIGNING_DOMAIN_V1,
  PROGRAMME_CAPTURE_SUPERVISOR_CLAIM_MAX_BYTES_V1,
  PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_DIGEST_DOMAIN_V1,
  createProgrammeCaptureSupervisorClaimAcknowledgementV1,
  deriveProgrammeCaptureSupervisorClaimRequestV1,
  parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1,
  programmeCaptureSupervisorClaimSigningPayloadV1,
  serializeProgrammeCaptureSupervisorClaimEnvelopeV1,
  verifyProgrammeCaptureSupervisorClaimAcknowledgementV1,
  type ProgrammeCaptureSupervisorClaimEnvelopeV1,
  type ProgrammeCaptureSupervisorClaimRequestV1,
} from '../src/programme-capture-supervisor-claim-v1.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_REQUEST_MAX_BYTES_V1,
  PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_MAX_BYTES_V1,
  createProgrammeCaptureSupervisorClaimValidationBlobV1,
  parseProgrammeCaptureSupervisorClaimRequestBlobV1,
  replayProgrammeCaptureSupervisorClaimValidationV1,
  serializeProgrammeCaptureSupervisorClaimRequestV1,
} from '../src/programme-capture-supervisor-codec-v1.js';
import {
  PROGRAMME_CAPTURE_OUTPUT_PATH,
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  PROGRAMME_CAPTURE_SCENARIOS_PATH,
} from '../src/programme-capture-task-v1.js';
import { parseProgrammeEnvelope } from '../src/programme-envelope.js';
import { digestValue } from '../src/receipts.js';
const roots: string[] = [];
const TASK_PATH = 'coding-harness/config/programme-v5-acceptance.json';
const PROJECT_AUTHORITY = '1'.repeat(64);
const RUNNER_IDENTITY = '2'.repeat(64);
const PREVIOUS_CHECKPOINT = '3'.repeat(64);
const SUPERVISOR_ID = 'external_supervisor_20260829';
const LOG_ID = 'capture_claim_log_20260829';
const KEY_EPOCH = 7;
const LOG_SEQUENCE = 41;
const keyPair = generateKeyPairSync('ed25519');
const publicKeySpki = keyPair.publicKey.export({ format: 'der', type: 'spki' }) as Buffer;
const keyFingerprint = sha256(publicKeySpki);
let controller: Readonly<{ root: string; commit: string }>;
beforeAll(() => { controller = controllerRepository(); });
afterAll(() => { rmSync(controller.root, { recursive: true, force: true }); });
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});
describe('programme capture V1 non-authorizing supervisor claim acknowledgement', () => {
  it('verifies a detached signature against a re-attested claim and supplied references', async () => {
    const claimAuthority = authorityInput();
    await reserveProgrammeCaptureRunClaimV1(claimAuthority);
    const request = await deriveProgrammeCaptureSupervisorClaimRequestV1({ claimAuthority });
    const serializedRequest = serializeProgrammeCaptureSupervisorClaimRequestV1(request);
    const envelope = signedEnvelope(request);
    const serialized = serializeProgrammeCaptureSupervisorClaimEnvelopeV1(envelope);
    expect(parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1(serialized)).toEqual(envelope);
    expect(parseProgrammeCaptureSupervisorClaimRequestBlobV1(serializedRequest)).toEqual(request);
    const view = await verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
      verificationInput(claimAuthority, envelope),
    );
    expect(view).toMatchObject({
      verificationScope: 'signature-and-rooted-claim-binding-only',
      signatureVerified: true,
      suppliedCheckpointReferenceMatched: true,
      externalAppendOnlyWitness: false,
      rollbackResistance: 'not-proven',
      hostAdmission: 'not-evaluated',
      runnerLeaseAcquired: false,
      stateTransitionAuthorized: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
    });
    expect(view.requestDigest).toBe(request.requestDigest);
    expect(view.serializedEnvelopeDigest).toBe(sha256(Buffer.from(serialized, 'utf8')));
    const serializedValidation = await createProgrammeCaptureSupervisorClaimValidationBlobV1(
      verificationInput(claimAuthority, envelope),
    );
    expect(await replayProgrammeCaptureSupervisorClaimValidationV1({
      ...verificationInput(claimAuthority, envelope), serializedValidation,
    })).toEqual(view);
    await expect(replayProgrammeCaptureSupervisorClaimValidationV1({
      ...verificationInput(claimAuthority, envelope), serializedValidation: JSON.stringify(view),
    })).rejects.toThrow();
    for (const invalidRequest of [
      serializedRequest.replace('"schemaVersion": 1,', '"schemaVersion": 1,\n  "schemaVersion": 1,'),
      ' '.repeat(PROGRAMME_CAPTURE_SUPERVISOR_REQUEST_MAX_BYTES_V1 + 1),
    ]) expect(() => parseProgrammeCaptureSupervisorClaimRequestBlobV1(invalidRequest)).toThrow();
    for (const invalidValidation of [
      serializedValidation.replace('"schemaVersion": 1,', '"schemaVersion": 1,\n  "schemaVersion": 1,'),
      ' '.repeat(PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_MAX_BYTES_V1 + 1),
    ]) await expect(replayProgrammeCaptureSupervisorClaimValidationV1({
      ...verificationInput(claimAuthority, envelope), serializedValidation: invalidValidation,
    })).rejects.toThrow();
    const forgedView = structuredClone(view) as any;
    forgedView.captureAuthorized = true;
    await expect(replayProgrammeCaptureSupervisorClaimValidationV1({
      ...verificationInput(claimAuthority, envelope),
      serializedValidation: `${JSON.stringify(forgedView, null, 2)}\n`,
    })).rejects.toThrow();
    const selfConsistentForgery = structuredClone(view) as any;
    selfConsistentForgery.serializedEnvelopeDigest = 'a'.repeat(64);
    const { validationDigest: _discarded, ...forgedBody } = selfConsistentForgery;
    selfConsistentForgery.validationDigest = digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_DIGEST_DOMAIN_V1,
      validation: forgedBody,
    });
    await expect(replayProgrammeCaptureSupervisorClaimValidationV1({
      ...verificationInput(claimAuthority, envelope),
      serializedValidation: `${JSON.stringify(selfConsistentForgery, null, 2)}\n`,
    })).rejects.toThrow(/REPLAY_MISMATCH/);
    const corruptedEnvelope = structuredClone(envelope);
    corruptedEnvelope.signature.valueBase64Url = `${
      corruptedEnvelope.signature.valueBase64Url.startsWith('A') ? 'B' : 'A'
    }${corruptedEnvelope.signature.valueBase64Url.slice(1)}`;
    await expect(replayProgrammeCaptureSupervisorClaimValidationV1({
      ...verificationInput(claimAuthority, corruptedEnvelope), serializedValidation,
    })).rejects.toThrow();
    expect(Object.isFrozen(view)).toBe(true);
    expect(() => parseProgrammeCaptureStateV1(view)).toThrow();
    expect(() => parseProgrammeEnvelope(serialized)).toThrow();
  }, 15_000);
  it('rejects self-selected keys, supervisor/log anchors, and signature corruption', async () => {
    const claimAuthority = authorityInput();
    await reserveProgrammeCaptureRunClaimV1(claimAuthority);
    const request = await deriveProgrammeCaptureSupervisorClaimRequestV1({ claimAuthority });
    const envelope = signedEnvelope(request);
    const attacker = generateKeyPairSync('ed25519');
    const attackerSpki = attacker.publicKey.export({ format: 'der', type: 'spki' }) as Buffer;
    const wrongType = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
    const wrongTypeSpki = wrongType.publicKey.export({ format: 'der', type: 'spki' }) as Buffer;
    for (const override of [
      { expectedAuthorityKeyFingerprint: '4'.repeat(64) },
      { expectedSupervisorId: 'different_supervisor_20260829' },
      { expectedLogId: 'different_capture_log_20260829' },
      { expectedKeyEpoch: KEY_EPOCH + 1 },
      { expectedLogSequence: LOG_SEQUENCE + 1 },
      { expectedPreviousCheckpointDigest: '5'.repeat(64) },
      { trustedPublicKeySpkiDer: attackerSpki },
      { trustedPublicKeySpkiDer: wrongTypeSpki },
    ]) {
      await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1({
        ...verificationInput(claimAuthority, envelope), ...override,
      })).rejects.toThrow();
    }
    const substituted = signedEnvelope(request, {}, attacker.privateKey, sha256(attackerSpki));
    await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
      verificationInput(claimAuthority, substituted),
    )).rejects.toThrow();
    const corrupted = structuredClone(envelope);
    const first = corrupted.signature.valueBase64Url.startsWith('A') ? 'B' : 'A';
    corrupted.signature.valueBase64Url = `${first}${corrupted.signature.valueBase64Url.slice(1)}`;
    await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
      verificationInput(claimAuthority, corrupted),
    )).rejects.toThrow();
  }, 15_000);
  it('rejects trusted-key re-signed claim, authority, checkpoint, and permission mutations', async () => {
    const claimAuthority = authorityInput();
    await reserveProgrammeCaptureRunClaimV1(claimAuthority);
    const request = await deriveProgrammeCaptureSupervisorClaimRequestV1({ claimAuthority });
    const envelope = signedEnvelope(request);
    const mutations: Array<(value: any) => void> = [
      (value) => { value.acknowledgement.runId = 'capture_supervisor_other_run'; },
      (value) => { value.acknowledgement.projectAuthorityDigest = '4'.repeat(64); },
      (value) => { value.acknowledgement.claimKeyDigest = '5'.repeat(64); },
      (value) => { value.acknowledgement.claimDigest = '6'.repeat(64); },
      (value) => { value.acknowledgement.requestDigest = '7'.repeat(64); },
      (value) => { value.acknowledgement.supervisor.supervisorId = 'other_supervisor_20260829'; },
      (value) => { value.acknowledgement.supervisor.logId = 'other_claim_log_20260829'; },
      (value) => { value.acknowledgement.supervisor.keyEpoch += 1; },
      (value) => { value.acknowledgement.event.logSequence += 1; },
      (value) => { value.acknowledgement.event.previousCheckpointDigest = '8'.repeat(64); },
      (value) => { value.acknowledgement.supervisor.authorityKeyFingerprint = '9'.repeat(64); },
      (value) => { value.acknowledgement.externalAppendOnlyWitness = true; },
      (value) => { value.acknowledgement.appendOnlyPersistenceVerified = true; },
      (value) => { value.acknowledgement.rollbackResistance = 'proven'; },
      (value) => { value.acknowledgement.supervisorAdministration = 'attested'; },
      (value) => { value.acknowledgement.hostAdmission = 'admitted'; },
      (value) => { value.acknowledgement.runnerLeaseAcquired = true; },
      (value) => { value.acknowledgement.stateTransitionAuthorized = true; },
      (value) => { value.acknowledgement.attemptStartAuthorized = true; },
      (value) => { value.acknowledgement.captureAuthorized = true; },
      (value) => { value.acknowledgement.schemaVersion = 2; },
      (value) => { value.acknowledgement.transactionKind = 'other'; },
      (value) => { value.acknowledgement.recordKind = 'other'; },
      (value) => { value.acknowledgement.authority = 'other'; },
      (value) => { value.acknowledgement.verificationScope = 'other'; },
      (value) => { value.acknowledgement.event.kind = 'other'; },
      (value) => { value.acknowledgement.event.runSequence = 1; },
      (value) => { value.schemaVersion = 2; },
      (value) => { value.transactionKind = 'other'; },
      (value) => { value.envelopeKind = 'other'; },
      (value) => { value.signature.algorithm = 'rsa'; },
    ];
    for (const mutate of mutations) {
      const reminted = resignMutation(envelope, mutate);
      await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
        {
          ...verificationInput(claimAuthority, envelope),
          serializedEnvelope: `${JSON.stringify(reminted, null, 2)}\n`,
        },
      )).rejects.toThrow();
    }
    const crossRun = resignMutation(envelope, (value) => {
      value.acknowledgement.runId = 'capture_supervisor_cross_run_20260829';
      value.acknowledgement.claimKeyDigest = programmeCaptureRunClaimKeyDigestV1({
        projectAuthorityDigest: value.acknowledgement.projectAuthorityDigest,
        runId: value.acknowledgement.runId,
      });
    });
    await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
      verificationInput(claimAuthority, crossRun),
    )).rejects.toThrow(/AUTHORITY_MISMATCH/);
  }, 15_000);
  it('rejects noncanonical blobs and missing, replaced, or caller-forged rooted claims', async () => {
    const validAuthority = authorityInput();
    const reservation = await reserveProgrammeCaptureRunClaimV1(validAuthority);
    const request = await deriveProgrammeCaptureSupervisorClaimRequestV1({
      claimAuthority: validAuthority,
    });
    const envelope = signedEnvelope(request);
    const canonicalBlob = serializeProgrammeCaptureSupervisorClaimEnvelopeV1(envelope);
    const duplicate = canonicalBlob.replace(
      '"schemaVersion": 1,', '"schemaVersion": 1,\n  "schemaVersion": 1,',
    );
    for (const blob of [
      JSON.stringify(envelope), `${canonicalBlob} `, `\ufeff${canonicalBlob}`, duplicate,
      ' '.repeat(PROGRAMME_CAPTURE_SUPERVISOR_CLAIM_MAX_BYTES_V1 + 1),
    ]) {
      expect(() => parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1(blob)).toThrow();
      await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1({
        ...verificationInput(validAuthority, envelope), serializedEnvelope: blob,
      })).rejects.toThrow();
    }
    await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1({
      ...verificationInput(validAuthority, envelope), serializedEnvelope: envelope,
    } as any)).rejects.toThrow();
    const wrongAlgorithm = structuredClone(envelope) as any;
    wrongAlgorithm.signature.algorithm = 'rsa';
    expect(() => serializeProgrammeCaptureSupervisorClaimEnvelopeV1(wrongAlgorithm)).toThrow();
    const paddedSignature = structuredClone(envelope) as any;
    paddedSignature.signature.valueBase64Url += '==';
    expect(() => serializeProgrammeCaptureSupervisorClaimEnvelopeV1(paddedSignature)).toThrow();
    rmSync(reservation.path);
    await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
      verificationInput(validAuthority, envelope),
    )).rejects.toThrow(/CLAIM_MISSING/);
    await reserveProgrammeCaptureRunClaimV1({
      ...validAuthority, expectedRunnerIdentityDigest: '9'.repeat(64),
    });
    await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
      verificationInput(validAuthority, envelope),
    )).rejects.toThrow(/AUTHORITY_MISMATCH/);
    await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1({
      ...verificationInput(validAuthority, envelope), claim: reservation.record,
    } as any)).rejects.toThrow(/invalid keys/);
  }, 15_000);
  it('keeps byte-identical local rollback and every execution authority explicit nonclaims', async () => {
    const claimAuthority = authorityInput();
    const first = await reserveProgrammeCaptureRunClaimV1(claimAuthority);
    const request = await deriveProgrammeCaptureSupervisorClaimRequestV1({ claimAuthority });
    const envelope = signedEnvelope(request);
    rmSync(first.path);
    await reserveProgrammeCaptureRunClaimV1(claimAuthority);
    const view = await verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
      verificationInput(claimAuthority, envelope),
    );
    expect(view).toMatchObject({
      externalAppendOnlyWitness: false,
      rollbackResistance: 'not-proven',
      runnerLeaseAcquired: false,
      stateTransitionAuthorized: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
    });
    const stableVerification = verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
      verificationInput(claimAuthority, envelope),
    );
    (claimAuthority as any).authorityRoot = authorityRoot();
    await expect(stableVerification).resolves.toMatchObject({ signatureVerified: true });
  }, 15_000);
});
function signedEnvelope(
  request: ProgrammeCaptureSupervisorClaimRequestV1,
  overrides: Record<string, unknown> = {},
  privateKey = keyPair.privateKey,
  authorityKeyFingerprint = keyFingerprint,
): ProgrammeCaptureSupervisorClaimEnvelopeV1 {
  const acknowledgement = createProgrammeCaptureSupervisorClaimAcknowledgementV1({
    request,
    supervisorId: SUPERVISOR_ID,
    logId: LOG_ID,
    keyEpoch: KEY_EPOCH,
    authorityKeyFingerprint,
    logSequence: LOG_SEQUENCE,
    previousCheckpointDigest: PREVIOUS_CHECKPOINT,
    ...overrides,
  });
  const signature = sign(
    null, programmeCaptureSupervisorClaimSigningPayloadV1(acknowledgement), privateKey,
  ).toString('base64url');
  return {
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    envelopeKind: 'supervisor-claim-acknowledgement-envelope-v1',
    acknowledgement,
    signature: { algorithm: 'ed25519', valueBase64Url: signature },
  };
}
function resignMutation(
  original: ProgrammeCaptureSupervisorClaimEnvelopeV1,
  mutate: (value: any) => void,
): ProgrammeCaptureSupervisorClaimEnvelopeV1 {
  const value = structuredClone(original) as any;
  mutate(value);
  const { acknowledgementDigest: _discarded, ...body } = value.acknowledgement;
  value.acknowledgement.acknowledgementDigest = digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_ACK_DIGEST_DOMAIN_V1,
    acknowledgement: body,
  });
  const payload = Buffer.from(canonical({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_ACK_SIGNING_DOMAIN_V1,
    acknowledgement: value.acknowledgement,
  }), 'utf8');
  value.signature.valueBase64Url = sign(null, payload, keyPair.privateKey).toString('base64url');
  return value;
}
function verificationInput(
  claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1,
  envelope: ProgrammeCaptureSupervisorClaimEnvelopeV1,
) {
  return {
    claimAuthority,
    serializedEnvelope: serializeProgrammeCaptureSupervisorClaimEnvelopeV1(envelope),
    trustedPublicKeySpkiDer: publicKeySpki,
    expectedAuthorityKeyFingerprint: keyFingerprint,
    expectedSupervisorId: SUPERVISOR_ID,
    expectedLogId: LOG_ID,
    expectedKeyEpoch: KEY_EPOCH,
    expectedLogSequence: LOG_SEQUENCE,
    expectedPreviousCheckpointDigest: PREVIOUS_CHECKPOINT,
  };
}
function authorityInput(): ProgrammeCaptureRunClaimAuthorityInputV1 {
  return {
    authorityRoot: authorityRoot(),
    projectAuthorityDigest: PROJECT_AUTHORITY,
    runId: 'capture_supervisor_claim_20260829_0001',
    controllerStore: controller.root,
    controllerCommit: controller.commit,
    taskPath: TASK_PATH,
    expectedRunnerIdentityDigest: RUNNER_IDENTITY,
  };
}
function controllerRepository(): Readonly<{ root: string; commit: string }> {
  const root = mkdtempSync(join(tmpdir(), 'capture-supervisor-controller-'));
  const values = new Map<string, Buffer>();
  for (const path of PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1) {
    values.set(path, Buffer.from(`capture supervisor fixture: ${path}\n`, 'utf8'));
  }
  const binding = (path: string) => ({ path, sha256: sha256(values.get(path)!) });
  const task = {
    schemaVersion: 1,
    taskKind: 'controlled-performance-baseline',
    taskId: 'capture_supervisor_claim_20260829',
    workItem: 'completion-programme:m0-performance-baseline',
    objective: 'Bind one rooted claim to a non-authorizing supervisor acknowledgement.',
    invariants: ['A signature never grants runner or capture authority.'],
    exclusions: ['No append-only, lease, attempt, or measurement claim.'],
    authority: 'development-only-no-promotion',
    inputs: {
      runnerProfile: binding(PROGRAMME_CAPTURE_PROFILE_PATH),
      scenarios: binding(PROGRAMME_CAPTURE_SCENARIOS_PATH),
      cargoLock: binding('Cargo.lock'),
      workloadSha256: 'd'.repeat(64),
      sources: PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS.map(binding),
    },
    commands: {
      capture: captureCommand('capture-baseline', 1_800_000),
      verify: captureCommand('check-baseline', 60_000),
    },
    output: {
      path: PROGRAMME_CAPTURE_OUTPUT_PATH,
      mode: 'create-new',
      mediaType: 'text/tab-separated-values; charset=utf-8',
      maximumBytes: 1_048_576,
    },
    policy: {
      measurementNetwork: 'offline',
      modelTransport: 'native-first-party-only',
      nativeHosts: ['codex', 'claude-code'],
      dualReview: { preCapture: true, postCapture: true },
      maximumMeasurementAttempts: 1,
      automaticMeasurementRetries: 0,
      automaticRepairs: 0,
      modelMeasurementOverlap: 'forbidden',
      coreEvidence: 'fail-closed',
    },
    routing: {
      tags: ['controlled-capture', 'performance'], difficulty: 1, evolutionEligible: false,
    },
  };
  writeFixture(root, 'coding-harness/.harness/manifest.json', readFileSync(
    new URL('../.harness/manifest.json', import.meta.url), 'utf8',
  ));
  writeFixture(root, TASK_PATH, `${JSON.stringify(task)}\n`);
  for (const [path, bytes] of values) writeFixture(root, path, bytes);
  git(root, ['init', '--quiet']);
  git(root, ['add', '--all']);
  git(root, ['commit', '--quiet', '-m', 'capture supervisor fixture']);
  chmodSync(join(root, '.git'), 0o755);
  chmodSync(join(root, '.git', 'objects'), 0o755);
  return { root, commit: git(root, ['rev-parse', 'HEAD']).trim() };
}

function captureCommand(argument: string, timeoutMs: number) {
  return {
    commandId: `${argument}_0001`,
    command: {
      tool: 'sf-performance-receipt', executable: 'target/release/sf-performance-receipt',
      argv: [argument], cwd: '.', env: {}, timeoutMs, maxOutputBytes: 1_048_576,
    },
  };
}

function writeFixture(root: string, path: string, value: string | Buffer): void {
  const absolute = join(root, path);
  mkdirSync(dirname(absolute), { recursive: true });
  writeFileSync(absolute, value);
}

function git(cwd: string, args: readonly string[]): string {
  const environment = {
    ...process.env,
    GIT_AUTHOR_NAME: 'Harness Test', GIT_AUTHOR_EMAIL: 'harness@example.invalid',
    GIT_AUTHOR_DATE: '2000-01-01T00:00:00Z', GIT_COMMITTER_NAME: 'Harness Test',
    GIT_COMMITTER_EMAIL: 'harness@example.invalid',
    GIT_COMMITTER_DATE: '2000-01-01T00:00:00Z',
  };
  const result = spawnSync('/usr/bin/git', args, { cwd, env: environment, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout;
}

function authorityRoot(): string {
  const root = mkdtempSync(join(tmpdir(), 'capture-supervisor-authority-'));
  roots.push(root);
  chmodSync(root, 0o700);
  return root;
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

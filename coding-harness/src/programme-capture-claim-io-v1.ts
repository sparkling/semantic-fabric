// SPDX-License-Identifier: MIT

import {
  closeSync,
  constants,
  fchmodSync,
  fstatSync,
  fsyncSync,
  lstatSync,
  openSync,
  readSync,
  realpathSync,
  statSync,
  writeSync,
  type BigIntStats,
} from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { asClosedRecord, assertExactKeys, deepFreeze } from './contracts.js';
import {
  PROGRAMME_CAPTURE_RUN_CLAIM_MAX_BYTES_V1,
  createProgrammeCaptureRunClaimV1,
  parseProgrammeCaptureRunClaimBlobV1,
  programmeCaptureRunClaimKeyDigestV1,
  serializeProgrammeCaptureRunClaimV1,
  verifyProgrammeCaptureRunClaimV1,
  type ProgrammeCaptureRunClaimV1,
} from './programme-capture-claim-record-v1.js';
import {
  attestProgrammeCaptureInputsV1,
  type ProgrammeCaptureInputAttestationV1,
} from './programme-capture-input-attestation-v1.js';
import {
  createProgrammeCaptureStateV1,
  type ProgrammeCaptureStateV1,
} from './programme-capture-state-v1.js';
import { digestValue } from './receipts.js';

const CLAIM_SUFFIX = '.claim.json';
const AUTHORITY_INPUT_KEYS = [
  'authorityRoot', 'projectAuthorityDigest', 'runId', 'controllerStore',
  'controllerCommit', 'taskPath', 'expectedRunnerIdentityDigest',
] as const;

export interface ProgrammeCaptureRunClaimAuthorityInputV1 {
  readonly authorityRoot: string;
  readonly projectAuthorityDigest: string;
  readonly runId: string;
  readonly controllerStore: string;
  readonly controllerCommit: string;
  readonly taskPath: string;
  readonly expectedRunnerIdentityDigest: string;
}

export interface ProgrammeCaptureClaimAdmissionViewV1 {
  readonly schemaVersion: 1;
  readonly transactionKind: 'programme-capture-v1';
  readonly evidenceKind: 'run-claim-admission-view-v1';
  readonly runId: string;
  readonly claimKeyDigest: string;
  readonly claimDigest: string;
  readonly inputAttestationDigest: string;
  readonly evidenceDigest: string;
}

export interface ProgrammeCaptureRunClaimReservationV1 {
  readonly path: string;
  readonly record: ProgrammeCaptureRunClaimV1;
  readonly inputAttestation: ProgrammeCaptureInputAttestationV1;
  readonly admissionView: ProgrammeCaptureClaimAdmissionViewV1;
  readonly stateView: ProgrammeCaptureStateV1;
}

interface DirectoryIdentity {
  readonly path: string;
  readonly dev: bigint;
  readonly ino: bigint;
  readonly mode: bigint;
  readonly uid: bigint;
  readonly nlink: bigint;
}

export function programmeCaptureRunClaimPathV1(value: Readonly<{
  authorityRoot: string;
  projectAuthorityDigest: string;
  runId: string;
}>): string {
  const input = asClosedRecord(value, 'programme capture claim-path input');
  assertExactKeys(
    input, ['authorityRoot', 'projectAuthorityDigest', 'runId'],
    'programme capture claim-path input',
  );
  const authorityRoot = lexicalAuthorityRoot(input.authorityRoot);
  const claimKeyDigest = programmeCaptureRunClaimKeyDigestV1({
    projectAuthorityDigest: input.projectAuthorityDigest as string,
    runId: input.runId as string,
  });
  return join(authorityRoot, `${claimKeyDigest}${CLAIM_SUFFIX}`);
}

export async function reserveProgrammeCaptureRunClaimV1(
  value: ProgrammeCaptureRunClaimAuthorityInputV1,
): Promise<ProgrammeCaptureRunClaimReservationV1> {
  const input = await parseAuthorityInput(value);
  const record = createProgrammeCaptureRunClaimV1({
    projectAuthorityDigest: input.projectAuthorityDigest,
    runId: input.runId,
    inputAttestation: input.inputAttestation,
    expectedRunnerIdentityDigest: input.expectedRunnerIdentityDigest,
  });
  const path = programmeCaptureRunClaimPathV1({
    authorityRoot: input.authorityRoot,
    projectAuthorityDigest: input.projectAuthorityDigest,
    runId: input.runId,
  });
  const serialized = serializeProgrammeCaptureRunClaimV1(record);
  const persisted = createExclusiveClaim(
    input.authorityRoot, record.claimKeyDigest, serialized,
  );
  return bindPersistedReservation(path, persisted, input);
}

export async function readProgrammeCaptureRunClaimV1(
  value: ProgrammeCaptureRunClaimAuthorityInputV1,
): Promise<ProgrammeCaptureRunClaimReservationV1> {
  const input = await parseAuthorityInput(value);
  const claimKeyDigest = programmeCaptureRunClaimKeyDigestV1({
    projectAuthorityDigest: input.projectAuthorityDigest,
    runId: input.runId,
  });
  const bytes = readClaim(input.authorityRoot, claimKeyDigest);
  const record = parseProgrammeCaptureRunClaimBlobV1(decodeUtf8(bytes));
  const path = programmeCaptureRunClaimPathV1({
    authorityRoot: input.authorityRoot,
    projectAuthorityDigest: input.projectAuthorityDigest,
    runId: input.runId,
  });
  return bindPersistedReservation(path, record, input);
}

async function parseAuthorityInput(
  value: ProgrammeCaptureRunClaimAuthorityInputV1,
): Promise<ProgrammeCaptureRunClaimAuthorityInputV1 & Readonly<{
  inputAttestation: ProgrammeCaptureInputAttestationV1;
}>> {
  const input = asClosedRecord(value, 'programme capture claim authority input');
  assertExactKeys(input, AUTHORITY_INPUT_KEYS, 'programme capture claim authority input');
  const authorityRoot = lexicalAuthorityRoot(input.authorityRoot);
  const attested = await attestProgrammeCaptureInputsV1({
    controllerStore: input.controllerStore as string,
    controllerCommit: input.controllerCommit as string,
    taskPath: input.taskPath as string,
  });
  const inputAttestation = attested.record;
  const record = createProgrammeCaptureRunClaimV1({
    projectAuthorityDigest: input.projectAuthorityDigest as string,
    runId: input.runId as string,
    inputAttestation,
    expectedRunnerIdentityDigest: input.expectedRunnerIdentityDigest as string,
  });
  return deepFreeze({
    authorityRoot,
    projectAuthorityDigest: record.authority.projectAuthorityDigest,
    runId: record.runId,
    controllerStore: input.controllerStore as string,
    controllerCommit: inputAttestation.controller.commit,
    taskPath: inputAttestation.task.path,
    inputAttestation,
    expectedRunnerIdentityDigest: record.expectedRunnerIdentityDigest,
  });
}

function bindPersistedReservation(
  path: string,
  claim: ProgrammeCaptureRunClaimV1,
  input: ProgrammeCaptureRunClaimAuthorityInputV1 & Readonly<{
    inputAttestation: ProgrammeCaptureInputAttestationV1;
  }>,
): ProgrammeCaptureRunClaimReservationV1 {
  const record = verifyProgrammeCaptureRunClaimV1({
    claim,
    inputAttestation: input.inputAttestation,
    expectedProjectAuthorityDigest: input.projectAuthorityDigest,
    expectedRunId: input.runId,
    expectedRunnerIdentityDigest: input.expectedRunnerIdentityDigest,
  });
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    evidenceKind: 'run-claim-admission-view-v1' as const,
    runId: record.runId,
    claimKeyDigest: record.claimKeyDigest,
    claimDigest: record.claimDigest,
    inputAttestationDigest: record.inputAttestationDigest,
  };
  const admissionView = deepFreeze({ ...body, evidenceDigest: digestValue(body) });
  const stateView = createProgrammeCaptureStateV1({
    runId: record.runId,
    taskDigest: record.task.valueDigest,
    claimDigest: record.claimDigest,
    controller: record.controller,
    admissionEvidenceDigest: admissionView.evidenceDigest,
  });
  return deepFreeze({
    path, record, inputAttestation: input.inputAttestation, admissionView, stateView,
  });
}

function createExclusiveClaim(
  authorityRoot: string,
  claimKeyDigest: string,
  serialized: string,
): ProgrammeCaptureRunClaimV1 {
  const bytes = Buffer.from(serialized, 'utf8');
  if (bytes.length < 1 || bytes.length > PROGRAMME_CAPTURE_RUN_CLAIM_MAX_BYTES_V1
    || parseProgrammeCaptureRunClaimBlobV1(serialized).claimKeyDigest !== claimKeyDigest) {
    throw new Error('HARNESS_CAPTURE_CLAIM_INPUT_INVALID');
  }
  const root = openAuthorityRoot(authorityRoot);
  const anchoredPath = `/proc/self/fd/${root.descriptor}/${claimKeyDigest}${CLAIM_SUFFIX}`;
  let claimDescriptor: number | undefined;
  let created = false;
  let failure: unknown;
  let persisted: ProgrammeCaptureRunClaimV1 | undefined;
  try {
    try {
      claimDescriptor = openSync(
        anchoredPath,
        constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | requiredNoFollow(),
        0o600,
      );
      created = true;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
        throw new Error('HARNESS_CAPTURE_CLAIM_SPENT', { cause: error });
      }
      throw new Error('HARNESS_CAPTURE_CLAIM_CREATE_FAILED', { cause: error });
    }
    fchmodSync(claimDescriptor, 0o600);
    assertCreatedFile(fstatSync(claimDescriptor, { bigint: true }), 0n);
    let offset = 0;
    while (offset < bytes.length) {
      const written = writeSync(
        claimDescriptor, bytes, offset, bytes.length - offset, offset,
      );
      if (written < 1) throw new Error('claim write made no progress');
      offset += written;
    }
    fsyncSync(claimDescriptor);
    const identity = assertCreatedFile(
      fstatSync(claimDescriptor, { bigint: true }), BigInt(bytes.length),
    );
    closeSync(claimDescriptor);
    claimDescriptor = undefined;
    fsyncSync(root.descriptor);
    const readback = readAnchoredClaim(root, anchoredPath, identity);
    persisted = parseProgrammeCaptureRunClaimBlobV1(decodeUtf8(readback));
    if (!readback.equals(bytes) || serializeProgrammeCaptureRunClaimV1(persisted) !== serialized) {
      throw new Error('claim readback differs');
    }
    assertAuthorityStable(root);
  } catch (error) {
    failure = error;
  }
  const claimCloseFailure = closeDescriptor(claimDescriptor);
  const rootCloseFailure = closeDescriptor(root.descriptor);
  failure ??= claimCloseFailure ?? rootCloseFailure;
  if (failure !== undefined) {
    if (created) throw new Error('HARNESS_CAPTURE_CLAIM_POISONED', { cause: failure });
    throw failure;
  }
  return persisted!;
}

function readClaim(authorityRoot: string, claimKeyDigest: string): Buffer {
  const root = openAuthorityRoot(authorityRoot);
  const anchoredPath = `/proc/self/fd/${root.descriptor}/${claimKeyDigest}${CLAIM_SUFFIX}`;
  let bytes: Buffer | undefined;
  let failure: unknown;
  try {
    let stat: BigIntStats;
    try {
      stat = lstatSync(anchoredPath, { bigint: true });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
        throw new Error('HARNESS_CAPTURE_CLAIM_MISSING', { cause: error });
      }
      throw error;
    }
    const identity = assertStoredFile(stat);
    bytes = readAnchoredClaim(root, anchoredPath, identity);
    assertAuthorityStable(root);
  } catch (error) {
    failure = error;
  }
  failure ??= closeDescriptor(root.descriptor);
  if (failure !== undefined) throw failure;
  return bytes!;
}

function readAnchoredClaim(
  root: Readonly<{ descriptor: number; identity: DirectoryIdentity }>,
  anchoredPath: string,
  expected: BigIntStats,
): Buffer {
  let descriptor: number | undefined;
  try {
    if (!sameFile(expected, assertStoredFile(lstatSync(anchoredPath, { bigint: true })))) {
      throw new Error('HARNESS_CAPTURE_CLAIM_CHANGED');
    }
    descriptor = openSync(anchoredPath, constants.O_RDONLY | requiredNoFollow());
    const before = assertStoredFile(fstatSync(descriptor, { bigint: true }));
    if (!sameFile(expected, before)) throw new Error('HARNESS_CAPTURE_CLAIM_CHANGED');
    const bytes = Buffer.alloc(Number(before.size));
    let offset = 0;
    while (offset < bytes.length) {
      const count = readSync(descriptor, bytes, offset, bytes.length - offset, offset);
      if (count < 1) throw new Error('HARNESS_CAPTURE_CLAIM_CHANGED');
      offset += count;
    }
    const after = assertStoredFile(fstatSync(descriptor, { bigint: true }));
    const named = assertStoredFile(lstatSync(anchoredPath, { bigint: true }));
    if (!sameFile(before, after) || !sameFile(after, named)) {
      throw new Error('HARNESS_CAPTURE_CLAIM_CHANGED');
    }
    assertAuthorityStable(root);
    return bytes;
  } catch (error) {
    if ((error as Error).message.startsWith('HARNESS_CAPTURE_CLAIM_')) throw error;
    throw new Error('HARNESS_CAPTURE_CLAIM_CHANGED', { cause: error });
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function openAuthorityRoot(authorityRoot: string): Readonly<{
  descriptor: number;
  identity: DirectoryIdentity;
}> {
  requirePlatform();
  const identity = authorityIdentity(authorityRoot);
  let descriptor: number;
  try {
    descriptor = openSync(
      identity.path,
      constants.O_RDONLY | requiredDirectory() | requiredNoFollow(),
    );
  } catch (error) {
    throw new Error('HARNESS_CAPTURE_CLAIM_AUTHORITY_INVALID', { cause: error });
  }
  try {
    if (!sameDirectory(identity, fstatSync(descriptor, { bigint: true }))) {
      throw new Error('HARNESS_CAPTURE_CLAIM_AUTHORITY_CHANGED');
    }
    assertAuthorityStable({ descriptor, identity });
    return Object.freeze({ descriptor, identity });
  } catch (error) {
    closeSync(descriptor);
    throw error;
  }
}

function authorityIdentity(value: string): DirectoryIdentity {
  const path = lexicalAuthorityRoot(value);
  try {
    assertProtectedParentChain(path);
    const stat = lstatSync(path, { bigint: true });
    const uid = currentUid();
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(path) !== path
      || stat.uid !== uid || (stat.mode & 0o7777n) !== 0o700n) throw new Error();
    return Object.freeze({
      path, dev: stat.dev, ino: stat.ino, mode: stat.mode, uid: stat.uid, nlink: stat.nlink,
    });
  } catch (error) {
    throw new Error('HARNESS_CAPTURE_CLAIM_AUTHORITY_INVALID', { cause: error });
  }
}

function assertAuthorityStable(root: Readonly<{
  descriptor: number;
  identity: DirectoryIdentity;
}>): void {
  try {
    const named = authorityIdentity(root.identity.path);
    const opened = fstatSync(root.descriptor, { bigint: true });
    if (!sameDirectory(root.identity, named) || !sameDirectory(root.identity, opened)) {
      throw new Error();
    }
  } catch (error) {
    throw new Error('HARNESS_CAPTURE_CLAIM_AUTHORITY_CHANGED', { cause: error });
  }
}

function assertProtectedParentChain(path: string): void {
  let current = dirname(path);
  const uid = currentUid();
  while (true) {
    const stat = lstatSync(current, { bigint: true });
    const writable = (stat.mode & 0o022n) !== 0n;
    const sticky = (stat.mode & 0o1000n) !== 0n;
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(current) !== current
      || (stat.uid !== 0n && stat.uid !== uid) || (writable && !sticky)) throw new Error();
    const parent = dirname(current);
    if (parent === current) return;
    current = parent;
  }
}

function assertCreatedFile(stat: BigIntStats, size: bigint): BigIntStats {
  if (!stat.isFile() || stat.isSymbolicLink() || stat.uid !== currentUid()
    || stat.nlink !== 1n || (stat.mode & 0o7777n) !== 0o600n || stat.size !== size) {
    throw new Error('created claim identity invalid');
  }
  return stat;
}

function assertStoredFile(stat: BigIntStats): BigIntStats {
  if (!stat.isFile() || stat.isSymbolicLink() || stat.uid !== currentUid()
    || stat.nlink !== 1n || (stat.mode & 0o7777n) !== 0o600n
    || stat.size < 1n || stat.size > BigInt(PROGRAMME_CAPTURE_RUN_CLAIM_MAX_BYTES_V1)) {
    throw new Error('HARNESS_CAPTURE_CLAIM_FILE_INVALID');
  }
  return stat;
}

function sameDirectory(left: DirectoryIdentity | BigIntStats, right: DirectoryIdentity | BigIntStats) {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.uid === right.uid && left.nlink === right.nlink;
}

function sameFile(left: BigIntStats, right: BigIntStats): boolean {
  return sameDirectory(left, right) && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function lexicalAuthorityRoot(value: unknown): string {
  if (typeof value !== 'string' || !isAbsolute(value) || resolve(value) !== value
    || value.includes('\0')) throw new Error('HARNESS_CAPTURE_CLAIM_AUTHORITY_INVALID');
  return value;
}

function requirePlatform(): void {
  if (process.platform !== 'linux' || typeof process.getuid !== 'function'
    || typeof constants.O_DIRECTORY !== 'number' || typeof constants.O_NOFOLLOW !== 'number'
    || typeof constants.O_EXCL !== 'number') {
    throw new Error('HARNESS_CAPTURE_CLAIM_PLATFORM_UNAVAILABLE');
  }
  try {
    if (!statSync('/proc/self/fd').isDirectory()) throw new Error();
  } catch (error) {
    throw new Error('HARNESS_CAPTURE_CLAIM_PLATFORM_UNAVAILABLE', { cause: error });
  }
}

function currentUid(): bigint {
  if (typeof process.getuid !== 'function') {
    throw new Error('HARNESS_CAPTURE_CLAIM_PLATFORM_UNAVAILABLE');
  }
  return BigInt(process.getuid());
}

function requiredDirectory(): number {
  if (typeof constants.O_DIRECTORY !== 'number') {
    throw new Error('HARNESS_CAPTURE_CLAIM_PLATFORM_UNAVAILABLE');
  }
  return constants.O_DIRECTORY;
}

function requiredNoFollow(): number {
  if (typeof constants.O_NOFOLLOW !== 'number') {
    throw new Error('HARNESS_CAPTURE_CLAIM_PLATFORM_UNAVAILABLE');
  }
  return constants.O_NOFOLLOW;
}

function decodeUtf8(bytes: Buffer): string {
  try {
    const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    if (!Buffer.from(text, 'utf8').equals(bytes)) throw new Error();
    return text;
  } catch (error) {
    throw new Error('HARNESS_CAPTURE_CLAIM_UTF8_INVALID', { cause: error });
  }
}

function closeDescriptor(descriptor: number | undefined): unknown {
  if (descriptor === undefined) return undefined;
  try { closeSync(descriptor); return undefined; }
  catch (error) { return error; }
}

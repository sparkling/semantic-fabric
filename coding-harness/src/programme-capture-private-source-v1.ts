// SPDX-License-Identifier: MIT

import {
  chmodSync,
  existsSync,
  mkdirSync,
  realpathSync,
  rmSync,
} from 'node:fs';
import { dirname, join, sep } from 'node:path';
import { asClosedRecord, assertExactKeys, deepFreeze } from './contracts.js';
import { assertGitMaterializationSafe } from './git-materialization.js';
import { gitExecutableEvidence, runGitCommandBytes } from './git-process.js';
import {
  assertCaptureControllerStoreStableV1,
  openCaptureControllerStoreV1,
  readCaptureCommitTreeV1,
  type CaptureControllerStoreV1,
  type CaptureTreeSnapshotV1,
} from './programme-capture-git-v1.js';
import {
  readProgrammeCaptureRunClaimV1,
  type ProgrammeCaptureRunClaimAuthorityInputV1,
  type ProgrammeCaptureRunClaimReservationV1,
} from './programme-capture-claim-io-v1.js';
import { PROGRAMME_CAPTURE_OUTPUT_PATH } from './programme-capture-task-v1.js';
import {
  assertPrivateSourceIdentityV1,
  assertPrivateSourceBlobSizesV1,
  assertPrivateSourceIndexMatchesTreeV1,
  assertPrivateSourceRootEntriesV1,
  assertSupportedPrivateSourceTreeV1,
  capturePrivateSourceInventoryV1,
  normalizePrivateSourceModesV1,
  pinPrivateSourceDirectoryV1,
  pinPrivateSourceFileV1,
  privateSourceDirectoriesV1,
  stablePrivateSourceFileDigestV1,
  type PinnedSourceIdentityV1,
  type ProgrammeCapturePrivateSourceInventoryV1,
} from './programme-capture-private-source-fs-v1.js';
import { digestValue } from './receipts.js';

const handles = new WeakMap<ProgrammeCapturePrivateSourceHandleV1, PrivateState>();
const INTRINSIC_WEAK_MAP_DELETE = WeakMap.prototype.delete;
const INTRINSIC_WEAK_MAP_GET = WeakMap.prototype.get;
const INTRINSIC_WEAK_MAP_SET = WeakMap.prototype.set;
const INTRINSIC_REFLECT_APPLY = Reflect.apply;

export type { ProgrammeCapturePrivateSourceFileV1 }
  from './programme-capture-private-source-fs-v1.js';

export interface ProgrammeCapturePrivateSourceViewV1 {
  readonly schemaVersion: 1;
  readonly transactionKind: 'programme-capture-v1';
  readonly evidenceKind: 'private-source-materialization-view-v1';
  readonly runId: string;
  readonly projectAuthorityDigest: string;
  readonly claimKeyDigest: string;
  readonly claimDigest: string;
  readonly inputAttestationDigest: string;
  readonly controller: Readonly<{ commit: string; tree: string }>;
  readonly expectedRunnerIdentityDigest: string;
  readonly sourceRoot: string;
  readonly treeListingDigest: string;
  readonly inventoryDigest: string;
  readonly indexDigest: string;
  readonly fileCount: number;
  readonly directoryCount: number;
  readonly totalBytes: number;
  readonly outputAbsent: true;
  readonly gitExecutable: Readonly<{ path: string; digest: string }>;
  readonly hostAdmission: 'not-evaluated';
  readonly runnerLeaseAcquired: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
  readonly viewDigest: string;
}

export interface ProgrammeCapturePrivateSourceHandleV1 {
  readonly sourceRoot: string;
  readonly view: ProgrammeCapturePrivateSourceViewV1;
}

interface PrivateState {
  readonly authority: ProgrammeCaptureRunClaimAuthorityInputV1;
  readonly parent: PinnedSourceIdentityV1;
  readonly root: PinnedSourceIdentityV1;
  readonly source: PinnedSourceIdentityV1;
  readonly index: PinnedSourceIdentityV1;
  readonly rootPath: string;
  readonly indexPath: string;
  readonly snapshot: CaptureTreeSnapshotV1;
  readonly view: ProgrammeCapturePrivateSourceViewV1;
}

export async function prepareProgrammeCapturePrivateSourceV1(input: Readonly<{
  claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1;
  runtimeParent: string;
  signal?: AbortSignal;
}>): Promise<ProgrammeCapturePrivateSourceHandleV1> {
  assertInputKeys(input, ['claimAuthority', 'runtimeParent'], ['signal']);
  assertSignal(input.signal);
  const suppliedAuthority = snapshotAuthority(input.claimAuthority);
  const first = await readProgrammeCaptureRunClaimV1(suppliedAuthority);
  const authority = canonicalAuthority(suppliedAuthority, first);
  const initialParent = pinPrivateSourceDirectoryV1(
    input.runtimeParent, 0o700, 'PARENT_INVALID',
  );
  const rootPath = join(initialParent.path, `${first.record.claimKeyDigest}.source-v1`);
  assertPrivateSourcePlacement(rootPath, authority);
  try {
    mkdirSync(rootPath, { mode: 0o700 });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
      throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_SPENT', { cause: error });
    }
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_CREATE_FAILED', { cause: error });
  }
  chmodSync(rootPath, 0o700);
  const parent = pinPrivateSourceDirectoryV1(input.runtimeParent, 0o700, 'PARENT_CHANGED');
  assertSameDirectoryObject(initialParent, parent, 'PARENT_CHANGED');
  const sourceRoot = join(rootPath, 'source');
  const indexPath = join(rootPath, 'index');
  mkdirSync(sourceRoot, { mode: 0o700 });
  chmodSync(sourceRoot, 0o700);
  const store = await openBoundStore(authority, first, input.signal);
  await assertGitMaterializationSafe({
    repositoryRoot: store.path,
    commits: [first.record.controller.commit],
    requireProtectedAuthority: true,
    signal: input.signal,
  });
  const snapshot = await readBoundTree(store, first, input.signal);
  assertSupportedPrivateSourceTreeV1(snapshot);
  await assertPrivateSourceBlobSizesV1(store.path, snapshot, input.signal);
  await gitEmpty(store.path, ['read-tree', first.record.controller.commit], indexPath, input.signal);
  await gitEmpty(
    store.path,
    [`--work-tree=${sourceRoot}`, 'checkout-index', '--all', `--prefix=${sourceRoot}/`],
    indexPath,
    input.signal,
  );
  chmodSync(indexPath, 0o400);
  normalizePrivateSourceModesV1(sourceRoot, snapshot);
  await assertPrivateSourceIndexMatchesTreeV1(store.path, indexPath, snapshot, input.signal);
  const inventory = capturePrivateSourceInventoryV1(sourceRoot, snapshot, store.objectFormat);
  const second = await readProgrammeCaptureRunClaimV1(authority);
  assertSameClaim(first, second);
  await assertCaptureControllerStoreStableV1(store, input.signal);
  assertPrivateSourceIdentityV1(parent, 'PARENT_CHANGED');
  chmodSync(rootPath, 0o500);
  const root = pinPrivateSourceDirectoryV1(rootPath, 0o500, 'ROOT_INVALID');
  const source = pinPrivateSourceDirectoryV1(sourceRoot, 0o500, 'SOURCE_INVALID');
  const index = pinPrivateSourceFileV1(indexPath, 0o400, 'INDEX_INVALID');
  const view = createView(first, sourceRoot, snapshot, inventory, indexPath);
  assertPrivateSourceIdentityV1(index, 'INDEX_CHANGED');
  const handle = Object.freeze({ sourceRoot, view });
  INTRINSIC_REFLECT_APPLY(INTRINSIC_WEAK_MAP_SET, handles, [handle, Object.freeze({
    authority, parent, root, source, index, rootPath, indexPath, snapshot, view,
  })]);
  return handle;
}

export async function verifyProgrammeCapturePrivateSourceV1(input: Readonly<{
  handle: ProgrammeCapturePrivateSourceHandleV1;
  claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1;
  signal?: AbortSignal;
}>): Promise<ProgrammeCapturePrivateSourceViewV1> {
  assertInputKeys(input, ['handle', 'claimAuthority'], ['signal']);
  assertSignal(input.signal);
  const state = stateFor(input.handle);
  if (!sameAuthority(state.authority, snapshotAuthority(input.claimAuthority))) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_AUTHORITY_MISMATCH');
  }
  return await verifyState(state, input.signal);
}

export async function disposeUnusedProgrammeCapturePrivateSourceV1(
  handle: ProgrammeCapturePrivateSourceHandleV1,
): Promise<void> {
  const state = stateFor(handle);
  await verifyState(state);
  chmodSync(state.rootPath, 0o700);
  chmodSync(state.source.path, 0o700);
  for (const directory of privateSourceDirectoriesV1(state.snapshot)
    .sort((left, right) => left.split('/').length - right.split('/').length)) {
    chmodSync(join(state.source.path, directory), 0o700);
  }
  rmSync(state.rootPath, { recursive: true, force: false });
  if (existsSync(state.rootPath)) throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_DISPOSE_FAILED');
  const parent = pinPrivateSourceDirectoryV1(
    state.parent.path, 0o700, 'PARENT_CHANGED',
  );
  assertSameDirectoryObject(state.parent, parent, 'PARENT_CHANGED');
  INTRINSIC_REFLECT_APPLY(INTRINSIC_WEAK_MAP_DELETE, handles, [handle]);
}

async function verifyState(
  state: PrivateState,
  signal?: AbortSignal,
): Promise<ProgrammeCapturePrivateSourceViewV1> {
  const first = await readProgrammeCaptureRunClaimV1(state.authority);
  assertPrivateSourceIdentityV1(state.parent, 'PARENT_CHANGED');
  assertPrivateSourceIdentityV1(state.root, 'ROOT_CHANGED');
  assertPrivateSourceIdentityV1(state.source, 'SOURCE_CHANGED');
  assertPrivateSourceIdentityV1(state.index, 'INDEX_CHANGED');
  assertPrivateSourceRootEntriesV1(state.rootPath);
  const store = await openBoundStore(state.authority, first, signal);
  const snapshot = await readBoundTree(store, first, signal);
  assertSupportedPrivateSourceTreeV1(snapshot);
  if (snapshot.listingDigest !== state.snapshot.listingDigest) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_TREE_CHANGED');
  }
  await assertPrivateSourceIndexMatchesTreeV1(store.path, state.indexPath, snapshot, signal);
  const inventory = capturePrivateSourceInventoryV1(
    state.source.path, snapshot, store.objectFormat,
  );
  const current = createView(first, state.source.path, snapshot, inventory, state.indexPath);
  if (JSON.stringify(current) !== JSON.stringify(state.view)) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_VIEW_CHANGED');
  }
  const second = await readProgrammeCaptureRunClaimV1(state.authority);
  assertSameClaim(first, second);
  await assertCaptureControllerStoreStableV1(store, signal);
  assertPrivateSourceIdentityV1(state.parent, 'PARENT_CHANGED');
  assertPrivateSourceIdentityV1(state.root, 'ROOT_CHANGED');
  assertPrivateSourceIdentityV1(state.source, 'SOURCE_CHANGED');
  assertPrivateSourceIdentityV1(state.index, 'INDEX_CHANGED');
  return state.view;
}

async function openBoundStore(
  authority: ProgrammeCaptureRunClaimAuthorityInputV1,
  claim: ProgrammeCaptureRunClaimReservationV1,
  signal?: AbortSignal,
): Promise<CaptureControllerStoreV1> {
  const store = await openCaptureControllerStoreV1(authority.controllerStore, signal);
  if (claim.record.controller.commit !== authority.controllerCommit) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_CONTROLLER_MISMATCH');
  }
  return store;
}

async function readBoundTree(
  store: CaptureControllerStoreV1,
  claim: ProgrammeCaptureRunClaimReservationV1,
  signal?: AbortSignal,
): Promise<CaptureTreeSnapshotV1> {
  const snapshot = await readCaptureCommitTreeV1(
    store.path,
    claim.record.controller.commit,
    claim.record.controller.tree,
    store.objectFormat,
    signal,
  );
  if (snapshot.entries.has(PROGRAMME_CAPTURE_OUTPUT_PATH)) {
    throw new Error('HARNESS_CAPTURE_OUTPUT_PRESENT_AT_COMMIT');
  }
  return snapshot;
}

function createView(
  claim: ProgrammeCaptureRunClaimReservationV1,
  sourceRoot: string,
  snapshot: CaptureTreeSnapshotV1,
  inventory: ProgrammeCapturePrivateSourceInventoryV1,
  indexPath: string,
): ProgrammeCapturePrivateSourceViewV1 {
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    evidenceKind: 'private-source-materialization-view-v1' as const,
    runId: claim.record.runId,
    projectAuthorityDigest: claim.record.authority.projectAuthorityDigest,
    claimKeyDigest: claim.record.claimKeyDigest,
    claimDigest: claim.record.claimDigest,
    inputAttestationDigest: claim.record.inputAttestationDigest,
    controller: claim.record.controller,
    expectedRunnerIdentityDigest: claim.record.expectedRunnerIdentityDigest,
    sourceRoot,
    treeListingDigest: snapshot.listingDigest,
    inventoryDigest: inventory.digest,
    indexDigest: stablePrivateSourceFileDigestV1(indexPath),
    fileCount: inventory.files.length,
    directoryCount: inventory.directories.length,
    totalBytes: inventory.totalBytes,
    outputAbsent: true as const,
    gitExecutable: gitExecutableEvidence(),
    hostAdmission: 'not-evaluated' as const,
    runnerLeaseAcquired: false as const,
    attemptStartAuthorized: false as const,
    captureAuthorized: false as const,
  };
  return deepFreeze({ ...body, viewDigest: digestValue(body) });
}

async function gitEmpty(
  root: string,
  args: readonly string[],
  indexPath: string,
  signal?: AbortSignal,
): Promise<void> {
  const result = await runGitCommandBytes(root, args, {
    environment: { GIT_INDEX_FILE: indexPath }, signal, maxOutputBytes: 4096,
  });
  if (result.exitCode !== 0 || result.stderr !== '' || result.stdout.length !== 0) {
    const command = args.find((argument) => !argument.startsWith('--')) ?? 'unknown';
    throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_GIT_FAILED:${command}`);
  }
}

function canonicalAuthority(
  input: ProgrammeCaptureRunClaimAuthorityInputV1,
  claim: ProgrammeCaptureRunClaimReservationV1,
): ProgrammeCaptureRunClaimAuthorityInputV1 {
  return Object.freeze({
    authorityRoot: dirname(claim.path),
    projectAuthorityDigest: claim.record.authority.projectAuthorityDigest,
    runId: claim.record.runId,
    controllerStore: realpathSync(input.controllerStore),
    controllerCommit: claim.record.controller.commit,
    taskPath: claim.record.task.path,
    expectedRunnerIdentityDigest: claim.record.expectedRunnerIdentityDigest,
  });
}

function snapshotAuthority(
  value: ProgrammeCaptureRunClaimAuthorityInputV1,
): ProgrammeCaptureRunClaimAuthorityInputV1 {
  const input = asClosedRecord(value, 'programme capture private-source claim authority');
  assertExactKeys(input, [
    'authorityRoot', 'projectAuthorityDigest', 'runId', 'controllerStore',
    'controllerCommit', 'taskPath', 'expectedRunnerIdentityDigest',
  ], 'programme capture private-source claim authority');
  return Object.freeze({
    authorityRoot: input.authorityRoot as string,
    projectAuthorityDigest: input.projectAuthorityDigest as string,
    runId: input.runId as string,
    controllerStore: input.controllerStore as string,
    controllerCommit: input.controllerCommit as string,
    taskPath: input.taskPath as string,
    expectedRunnerIdentityDigest: input.expectedRunnerIdentityDigest as string,
  });
}

function assertPrivateSourcePlacement(
  rootPath: string,
  authority: ProgrammeCaptureRunClaimAuthorityInputV1,
): void {
  if (pathsOverlap(rootPath, authority.authorityRoot)
    || pathsOverlap(rootPath, authority.controllerStore)) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_PLACEMENT_INVALID');
  }
}

function pathsOverlap(left: string, right: string): boolean {
  return left === right || left.startsWith(`${right}${sep}`) || right.startsWith(`${left}${sep}`);
}

function assertSameClaim(
  left: ProgrammeCaptureRunClaimReservationV1,
  right: ProgrammeCaptureRunClaimReservationV1,
): void {
  if (left.record.claimDigest !== right.record.claimDigest
    || left.inputAttestation.attestationDigest !== right.inputAttestation.attestationDigest) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_CLAIM_CHANGED');
  }
}

function assertSameDirectoryObject(
  left: PinnedSourceIdentityV1,
  right: PinnedSourceIdentityV1,
  error: string,
): void {
  if (left.path !== right.path || left.kind !== 'directory' || right.kind !== 'directory'
    || left.dev !== right.dev || left.ino !== right.ino || left.mode !== right.mode
    || left.uid !== right.uid) {
    throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_${error}`);
  }
}

function sameAuthority(
  expected: ProgrammeCaptureRunClaimAuthorityInputV1,
  actual: ProgrammeCaptureRunClaimAuthorityInputV1,
): boolean {
  try {
    return expected.authorityRoot === actual.authorityRoot
      && expected.projectAuthorityDigest === actual.projectAuthorityDigest
      && expected.runId === actual.runId
      && expected.controllerStore === realpathSync(actual.controllerStore)
      && expected.controllerCommit === actual.controllerCommit
      && expected.taskPath === actual.taskPath
      && expected.expectedRunnerIdentityDigest === actual.expectedRunnerIdentityDigest;
  } catch { return false; }
}

function stateFor(handle: ProgrammeCapturePrivateSourceHandleV1): PrivateState {
  if ((typeof handle !== 'object' && typeof handle !== 'function') || handle === null) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_HANDLE_INVALID');
  }
  const state = INTRINSIC_REFLECT_APPLY(
    INTRINSIC_WEAK_MAP_GET, handles, [handle],
  ) as PrivateState | undefined;
  if (state === undefined || handle.sourceRoot !== state.source.path || handle.view !== state.view) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_HANDLE_INVALID');
  }
  return state;
}

function assertInputKeys(
  value: object,
  required: readonly string[],
  optional: readonly string[],
): void {
  const keys = Reflect.ownKeys(value);
  const allowed = [...required, ...optional];
  if (Object.getPrototypeOf(value) !== Object.prototype
    || required.some((key) => !keys.includes(key))
    || keys.some((key) => {
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      return typeof key !== 'string' || !allowed.includes(key)
        || descriptor?.enumerable !== true || !Object.hasOwn(descriptor, 'value');
    })) {
    throw new TypeError('programme capture private-source input has invalid keys');
  }
}

function assertSignal(signal: AbortSignal | undefined): void {
  if (signal !== undefined && !(signal instanceof AbortSignal)) {
    throw new TypeError('programme capture private-source signal is invalid');
  }
}

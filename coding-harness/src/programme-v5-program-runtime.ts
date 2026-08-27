// SPDX-License-Identifier: MIT

import { chmodSync, lstatSync, readdirSync, realpathSync } from 'node:fs';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { runGitCommand } from './git-process.js';
import { cleanupRequiresAncestorPreservation } from './resource-cleanup.js';
import { normalizeAcceptanceTaskPath } from './manifest.js';
import { METAHARNESS_DIAGNOSTICS_PATH } from './metaharness-diagnostics.js';
import { requireProgrammeV5ArtifactPath } from './programme-v5-receipt-io.js';

export const PROGRAMME_V5_ACCEPTANCE_TASK_PATH =
  'coding-harness/config/programme-v5-acceptance.json';

const OPAQUE_ID = /^[A-Za-z0-9_-]{8,160}$/;
const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const DIGEST = /^[a-f0-9]{64}$/;
const SCRATCH_PARENT = '/home/claude/.cache/semantic-fabric-harness';

export interface ProgrammeV5BootstrapEvidence {
  readonly schemaVersion: 3;
  readonly source: 'verified-packed-private-runtime';
  readonly controllerCommit: string;
  readonly taskPath: string;
  readonly controllerStoreDigest: string;
  readonly buildManifestDigest: string;
  readonly runtimeTreeDigest: string;
  readonly nodeDigest: string;
  readonly gitDigest: string;
}

export interface ProgrammeV5BaseInvocation {
  readonly repositoryRoot: string;
  readonly controllerStore: string;
  readonly controllerCommit: string;
  readonly taskPath: string;
  readonly runId: string;
  readonly swarmId: string;
  readonly coordinationTaskId: string;
  readonly hiveId: string;
  readonly consensusId: string;
}

export interface ProgrammeV5Invocation extends ProgrammeV5BaseInvocation {
  readonly policyReviewReceipt: string;
  readonly expectedPolicy: Readonly<{
    readonly controllerCommit: string;
    readonly taskPath: string;
    readonly fingerprint: string;
  }>;
}

export interface ProgrammeV5PolicyReviewInvocation extends ProgrammeV5BaseInvocation {
  readonly reviewMode: 'prepare-only';
}

export interface ProgrammeV5ReplayInvocation extends ProgrammeV5BaseInvocation {
  readonly replayMode: 'verify-only';
  readonly policyReviewReceipt: string;
  readonly envelopeReceipt: string;
  readonly replayReceipt: string;
  readonly expectedPolicy: Readonly<{
    readonly controllerCommit: string;
    readonly taskPath: string;
    readonly fingerprint: string;
  }>;
}

export interface ProgrammeV5ScratchLayout {
  readonly worktrees: string;
  readonly evaluator: string;
  readonly frozen: string;
  readonly native: string;
  readonly sast: string;
  readonly repository: string;
  readonly gitTemplate: string;
}

export function parseProgrammeV5Invocation(argv: readonly string[]): ProgrammeV5Invocation {
  const values = parseInvocationValues(
    argv, ['expected-policy-fingerprint', 'policy-review-receipt'],
  );
  const base = parseBaseInvocation(values);
  const expectedPolicyFingerprint = policyFingerprint(
    required(values, 'expected-policy-fingerprint'),
  );
  return Object.freeze({
    ...base,
    policyReviewReceipt: requireProgrammeV5ArtifactPath(
      base.repositoryRoot,
      base.runId,
      'policy-review',
      required(values, 'policy-review-receipt'),
    ),
    expectedPolicy: Object.freeze({
      controllerCommit: base.controllerCommit,
      taskPath: base.taskPath,
      fingerprint: expectedPolicyFingerprint,
    }),
  });
}

export function parseProgrammeV5PolicyReviewInvocation(
  argv: readonly string[],
): ProgrammeV5PolicyReviewInvocation {
  const values = parseInvocationValues(argv, ['policy-review']);
  if (required(values, 'policy-review') !== 'prepare-only') {
    throw new Error('HARNESS_PROGRAMME_V5_POLICY_REVIEW_MODE_INVALID');
  }
  return Object.freeze({ ...parseBaseInvocation(values), reviewMode: 'prepare-only' });
}

export function parseProgrammeV5ReplayInvocation(
  argv: readonly string[],
): ProgrammeV5ReplayInvocation {
  const values = parseInvocationValues(argv, [
    'replay', 'expected-policy-fingerprint', 'policy-review-receipt',
    'envelope-receipt', 'receipt-path',
  ]);
  if (required(values, 'replay') !== 'verify-only') {
    throw new Error('HARNESS_PROGRAMME_V5_REPLAY_MODE_INVALID');
  }
  const base = parseBaseInvocation(values);
  return Object.freeze({
    ...base,
    replayMode: 'verify-only',
    policyReviewReceipt: requireProgrammeV5ArtifactPath(
      base.repositoryRoot, base.runId, 'policy-review', required(values, 'policy-review-receipt'),
    ),
    envelopeReceipt: requireProgrammeV5ArtifactPath(
      base.repositoryRoot, base.runId, 'execution', required(values, 'envelope-receipt'),
    ),
    replayReceipt: requireProgrammeV5ArtifactPath(
      base.repositoryRoot, base.runId, 'replay', required(values, 'receipt-path'),
    ),
    expectedPolicy: Object.freeze({
      controllerCommit: base.controllerCommit,
      taskPath: base.taskPath,
      fingerprint: policyFingerprint(required(values, 'expected-policy-fingerprint')),
    }),
  });
}

function parseInvocationValues(
  argv: readonly string[],
  operationFlags: readonly string[],
): Map<string, string> {
  const requiredFlags = [
    'repository', 'controller-store', 'controller-commit', 'run-id', 'swarm-id',
    'coordination-task-id', 'hive-id', 'consensus-id', ...operationFlags,
  ];
  const allowed = [...requiredFlags, 'task-path'];
  if (![requiredFlags.length * 2, allowed.length * 2].includes(argv.length)) {
    throw new Error('HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID');
  }
  const values = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const name = flag?.startsWith('--') ? flag.slice(2) : '';
    const value = argv[index + 1];
    if (!allowed.includes(name) || values.has(name) || value === undefined
      || value.length === 0 || value.includes('\0')) {
      throw new Error('HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID');
    }
    values.set(name, value);
  }
  if (requiredFlags.some((name) => !values.has(name))) {
    throw new Error('HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID');
  }
  return values;
}

function parseBaseInvocation(
  values: ReadonlyMap<string, string>,
): ProgrammeV5BaseInvocation {
  const controllerCommit = required(values, 'controller-commit');
  if (!GIT_OBJECT.test(controllerCommit)) {
    throw new Error('HARNESS_PROGRAMME_V5_CONTROLLER_COMMIT_INVALID');
  }
  const taskPath = normalizeAcceptanceTaskPath(
    values.get('task-path') ?? PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
  );
  return Object.freeze({
    repositoryRoot: canonicalDirectory(required(values, 'repository')),
    controllerStore: privateDirectory(required(values, 'controller-store'), 'CONTROLLER_STORE'),
    controllerCommit,
    taskPath,
    runId: opaque(required(values, 'run-id'), 'RUN_ID'),
    swarmId: opaque(required(values, 'swarm-id'), 'SWARM_ID'),
    coordinationTaskId: opaque(required(values, 'coordination-task-id'), 'TASK_ID'),
    hiveId: exactRufloSetting(required(values, 'hive-id'), 'hierarchical', 'HIVE_ID'),
    consensusId: exactRufloSetting(required(values, 'consensus-id'), 'raft', 'CONSENSUS_ID'),
  });
}

export function parseProgrammeV5Bootstrap(value: unknown): ProgrammeV5BootstrapEvidence {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('HARNESS_PROGRAMME_V5_BOOTSTRAP_EVIDENCE_INVALID');
  }
  const input = value as Record<string, unknown>;
  const expected = [
    'buildManifestDigest', 'controllerCommit', 'controllerStoreDigest', 'gitDigest',
    'nodeDigest', 'runtimeTreeDigest', 'schemaVersion', 'source', 'taskPath',
  ].sort();
  if (JSON.stringify(Object.keys(input).sort()) !== JSON.stringify(expected)
    || input.schemaVersion !== 3 || input.source !== 'verified-packed-private-runtime'
    || typeof input.controllerCommit !== 'string' || !GIT_OBJECT.test(input.controllerCommit)) {
    throw new Error('HARNESS_PROGRAMME_V5_BOOTSTRAP_EVIDENCE_INVALID');
  }
  return Object.freeze({
    schemaVersion: 3,
    source: 'verified-packed-private-runtime',
    controllerCommit: input.controllerCommit,
    taskPath: normalizeAcceptanceTaskPath(input.taskPath),
    controllerStoreDigest: digest(input.controllerStoreDigest),
    buildManifestDigest: digest(input.buildManifestDigest),
    runtimeTreeDigest: digest(input.runtimeTreeDigest),
    nodeDigest: digest(input.nodeDigest),
    gitDigest: digest(input.gitDigest),
  });
}

export function verifyProgrammeV5ExpectedPolicyFingerprint(
  invocation: ProgrammeV5Invocation,
  actualFingerprint: unknown,
): string {
  const expected = invocation.expectedPolicy;
  if (expected.controllerCommit !== invocation.controllerCommit
    || expected.taskPath !== invocation.taskPath) {
    throw new Error('HARNESS_PROGRAMME_V5_EXPECTED_POLICY_BINDING_INVALID');
  }
  const actual = policyFingerprint(actualFingerprint);
  if (actual !== expected.fingerprint) {
    throw new Error('HARNESS_PROGRAMME_V5_EXPECTED_POLICY_FINGERPRINT_MISMATCH');
  }
  return expected.fingerprint;
}

export async function createProgrammeV5ScratchRoot(): Promise<string> {
  await mkdir(SCRATCH_PARENT, { recursive: true, mode: 0o700 });
  const parent = privateDirectory(SCRATCH_PARENT, 'SCRATCH_PARENT');
  const root = await mkdtemp(join(parent, 'v5-'));
  chmodSync(root, 0o700);
  return privateDirectory(root, 'SCRATCH_ROOT');
}

export async function prepareProgrammeV5ScratchLayout(
  scratch: string,
): Promise<ProgrammeV5ScratchLayout> {
  const paths = {
    worktrees: join(scratch, 'worktrees'),
    evaluator: join(scratch, 'evaluator'),
    frozen: join(scratch, 'worktrees', 'frozen'),
    native: join(scratch, 'n'),
    sast: join(scratch, 'sast'),
    repository: join(scratch, 'repository.git'),
    gitTemplate: join(scratch, 'git-template'),
  };
  await mkdir(paths.native, { mode: 0o700 });
  await mkdir(paths.sast, { mode: 0o700 });
  await mkdir(paths.gitTemplate, { mode: 0o700 });
  return Object.freeze(paths);
}

export async function cloneProgrammeV5ControllerRepository(
  source: string,
  target: string,
  template: string,
  commit: string,
): Promise<string> {
  assertAbsent(target, 'HARNESS_PROGRAMME_V5_TRANSACTION_REPOSITORY_EXISTS');
  const clone = await runGitCommand(dirname(target), [
    'clone', '--bare', '--no-hardlinks', '--local', '--no-tags',
    `--template=${template}`, '--', source, target,
  ], { timeoutMs: 120_000, maxOutputBytes: 1_000_000 });
  if (clone.exitCode !== 0) {
    throw new Error('HARNESS_PROGRAMME_V5_TRANSACTION_REPOSITORY_CLONE_FAILED');
  }
  const remote = await runGitCommand(target, ['config', '--remove-section', 'remote.origin']);
  if (remote.exitCode !== 0) {
    throw new Error('HARNESS_PROGRAMME_V5_TRANSACTION_REPOSITORY_CONFIG_INVALID');
  }
  const fsck = await runGitCommand(target, [
    'fsck', '--strict', '--full', '--no-reflogs', commit,
  ], { timeoutMs: 120_000, maxOutputBytes: 10_000_000 });
  const head = await runGitCommand(target, ['rev-parse', '--verify', 'HEAD']);
  if (fsck.exitCode !== 0 || head.exitCode !== 0 || head.stdout.trim() !== commit) {
    throw new Error('HARNESS_PROGRAMME_V5_TRANSACTION_REPOSITORY_VERIFY_FAILED');
  }
  const root = privateDirectory(target, 'TRANSACTION_REPOSITORY');
  const pending = [root];
  let entries = 0;
  while (pending.length > 0) {
    const directory = pending.pop()!;
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      entries += 1;
      if (entries > 100) {
        throw new Error('HARNESS_PROGRAMME_V5_TRANSACTION_REPOSITORY_LAYOUT_INVALID');
      }
      const path = join(directory, entry.name);
      const stat = lstatSync(path);
      if (stat.isSymbolicLink() || (stat.mode & 0o077) !== 0
        || (!stat.isDirectory() && (!stat.isFile() || stat.nlink !== 1))) {
        throw new Error('HARNESS_PROGRAMME_V5_TRANSACTION_REPOSITORY_LAYOUT_INVALID');
      }
      if (stat.isDirectory()) pending.push(path);
    }
  }
  assertAbsent(join(root, 'objects', 'info', 'alternates'),
    'HARNESS_PROGRAMME_V5_TRANSACTION_REPOSITORY_ALTERNATE');
  return root;
}

export function assertProgrammeV5ControlledRoot(scratch: string, controlledRoot: string): string {
  const child = canonicalDirectory(controlledRoot);
  const delta = relative(scratch, child);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error('HARNESS_PROGRAMME_V5_CONTROLLED_ROOT_INVALID');
  }
  return child;
}

export async function readProgrammeV5Diagnostics(store: string, commit: string): Promise<string> {
  const result = await runGitCommand(store, [
    'show', `${commit}:${METAHARNESS_DIAGNOSTICS_PATH}`,
  ], { maxOutputBytes: 1_000_000 });
  if (result.exitCode !== 0 || Buffer.byteLength(result.stdout, 'utf8') < 1) {
    throw new Error('HARNESS_PROGRAMME_V5_DIAGNOSTICS_READ_FAILED');
  }
  return result.stdout;
}

export async function removeProgrammeV5Scratch(
  path: string,
  priorFailure?: unknown,
): Promise<boolean> {
  if (cleanupRequiresAncestorPreservation(priorFailure)) return false;
  privateDirectory(path, 'SCRATCH_ROOT_CHANGED');
  await rm(path, { recursive: true, force: true });
  return true;
}

export function assertAbsent(path: string, error: string): void {
  try {
    lstatSync(path);
  } catch (caught) {
    if ((caught as NodeJS.ErrnoException).code === 'ENOENT') return;
    throw caught;
  }
  throw new Error(error);
}

function privateDirectory(value: string, label: string): string {
  const path = canonicalDirectory(value);
  const stat = lstatSync(path);
  const uid = process.getuid?.() ?? stat.uid;
  if (stat.uid !== uid || (stat.mode & 0o077) !== 0) {
    throw new Error(`HARNESS_PROGRAMME_V5_${label}_INVALID`);
  }
  return path;
}

function canonicalDirectory(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error('HARNESS_PROGRAMME_V5_DIRECTORY_INVALID');
  }
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error('HARNESS_PROGRAMME_V5_DIRECTORY_INVALID');
  }
  return value;
}

function required(values: ReadonlyMap<string, string>, name: string): string {
  const value = values.get(name);
  if (value === undefined || value.length === 0) {
    throw new Error('HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID');
  }
  return value;
}

function opaque(value: string, label: string): string {
  if (!OPAQUE_ID.test(value)) throw new Error(`HARNESS_PROGRAMME_V5_${label}_INVALID`);
  return value;
}

function exactRufloSetting(value: string, expected: string, label: string): string {
  if (value !== expected) throw new Error(`HARNESS_PROGRAMME_V5_${label}_INVALID`);
  return value;
}

function digest(value: unknown): string {
  if (typeof value !== 'string' || !DIGEST.test(value) || value === '0'.repeat(64)) {
    throw new Error('HARNESS_PROGRAMME_V5_BOOTSTRAP_DIGEST_INVALID');
  }
  return value;
}

function policyFingerprint(value: unknown): string {
  if (typeof value !== 'string' || !DIGEST.test(value) || value === '0'.repeat(64)) {
    throw new Error('HARNESS_PROGRAMME_V5_EXPECTED_POLICY_FINGERPRINT_INVALID');
  }
  return value;
}

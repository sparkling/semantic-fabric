// SPDX-License-Identifier: MIT

import { chmodSync, lstatSync, readdirSync, realpathSync } from 'node:fs';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SECURE_HARNESS_CONFIG } from './config.js';
import {
  createStructuredFrozenCargoLockExecutor,
  prepareFrozenCargoLock,
} from './frozen-cargo-lock.js';
import { runGitCommand } from './git-process.js';
import { runIssue8Transaction, type Issue8DriverResult } from './issue-8-driver.js';
import { createIssue8NativeSession } from './issue-8-native-session.js';
import { createIssue8QeCollector } from './issue-8-qe.js';
import {
  ISSUE_8_NATIVE_LIMITS,
  ISSUE_8_RUST_LIMITS,
  ISSUE_8_SYSTEM_PATHS,
  attestIssue8SystemTools,
  prepareCargoExtension,
} from './issue-8-system.js';
import { ReceiptChain } from './receipts.js';
import { SystemdResourceBoundary } from './resource-boundary.js';
import { prepareIssue8RustClosure } from './rust-closure.js';
import { createRustOfflineProfile } from './rust-sandbox.js';

const ISSUE_8_TASK_ID = 'bprune_8_20260825';
const MODELS = Object.freeze({ codex: 'gpt-5.6-sol', claude: 'claude-sonnet-4-6' });
const OPAQUE_ID = /^[A-Za-z0-9_-]{8,160}$/;
const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const SCRATCH_PARENT = '/home/claude/.cache/semantic-fabric-harness';

export interface TrustedBootstrapEvidence {
  readonly schemaVersion: 2;
  readonly source: 'verified-packed-private-runtime';
  readonly controllerCommit: string;
  readonly controllerStoreDigest: string;
  readonly buildManifestDigest: string;
  readonly runtimeTreeDigest: string;
  readonly nodeDigest: string;
  readonly gitDigest: string;
}

export interface TrustedControllerOutcome {
  readonly status: 'pass' | 'fail' | 'gated' | 'cancelled';
  readonly reason: string | null;
  seal(): Promise<Readonly<{ status: string; receiptPath: string; receiptDigest: string }>>;
}

export async function trustedControllerMain(
  argv: readonly string[],
  rawBootstrap: unknown,
): Promise<TrustedControllerOutcome> {
  const invocation = parseInvocation(argv);
  const bootstrap = parseBootstrap(rawBootstrap);
  if (bootstrap.controllerCommit !== invocation.controllerCommit) {
    throw new Error('HARNESS_ISSUE_8_BOOTSTRAP_COMMIT_MISMATCH');
  }
  const receiptPath = join(
    invocation.repositoryRoot, 'coding-harness', '.metaharness', 'runs', `${invocation.runId}.json`,
  );
  assertAbsent(receiptPath, 'HARNESS_ISSUE_8_RECEIPT_EXISTS');
  const scratch = await createScratchRoot();
  let result: Issue8DriverResult | undefined;
  let executionError: unknown;
  try {
    result = await executeIssue8(invocation, bootstrap, scratch);
  } catch (error) {
    executionError = error;
  }
  try {
    await removeScratch(scratch);
  } catch (cleanupError) {
    if (executionError !== undefined) {
      throw new AggregateError(
        [executionError, cleanupError],
        'HARNESS_ISSUE_8_EXECUTION_AND_SCRATCH_CLEANUP_FAILED',
      );
    }
    throw cleanupError;
  }
  if (executionError !== undefined) throw executionError;
  if (result === undefined) throw new Error('HARNESS_ISSUE_8_RESULT_MISSING');

  const serialized = `${JSON.stringify({
    schemaVersion: 2,
    receipts: [result.transaction.receipt],
  }, null, 2)}\n`;
  const verified = ReceiptChain.import(serialized);
  if (verified.length !== 1 || verified.headDigest !== result.transaction.receipt.digest) {
    throw new Error('HARNESS_ISSUE_8_RECEIPT_CHAIN_INVALID');
  }
  let sealed = false;
  return Object.freeze({
    status: result.transaction.status,
    reason: result.transaction.reason,
    async seal() {
      if (sealed) throw new Error('HARNESS_ISSUE_8_OUTCOME_ALREADY_SEALED');
      await prepareResultsRoot(invocation.repositoryRoot);
      assertAbsent(receiptPath, 'HARNESS_ISSUE_8_RECEIPT_EXISTS');
      await writeFile(receiptPath, serialized, { encoding: 'utf8', flag: 'wx', mode: 0o600 });
      sealed = true;
      return Object.freeze({
        status: result.transaction.status,
        receiptPath,
        receiptDigest: result.transaction.receipt.digest,
      });
    },
  });
}

interface Issue8Invocation {
  readonly repositoryRoot: string;
  readonly controllerStore: string;
  readonly controllerCommit: string;
  readonly runId: string;
  readonly swarmId: string;
  readonly coordinationTaskId: string;
  readonly hiveId: string;
  readonly consensusId: string;
}

async function executeIssue8(
  invocation: Issue8Invocation,
  bootstrap: TrustedBootstrapEvidence,
  scratch: string,
): Promise<Issue8DriverResult> {
  const paths = await prepareScratchLayout(scratch);
  const transactionRepository = await cloneControllerRepository(
    invocation.controllerStore,
    paths.repository,
    paths.gitTemplate,
    invocation.controllerCommit,
  );
  const rustClosure = prepareIssue8RustClosure({
    scratchRoot: scratch,
    toolchainSource: ISSUE_8_SYSTEM_PATHS.toolchain,
    registrySource: ISSUE_8_SYSTEM_PATHS.registry,
  });
  const systemTools = { ...attestIssue8SystemTools(), ...rustClosure.evidence };
  const cargoExtensionRoot = prepareCargoExtension(scratch);
  const qe = createIssue8QeCollector({
    taskId: ISSUE_8_TASK_ID,
    runId: invocation.runId,
    snapshotParent: paths.sast,
    nodeExecutable: ISSUE_8_SYSTEM_PATHS.node,
    bwrapExecutable: ISSUE_8_SYSTEM_PATHS.bwrap,
    packageRoot: ISSUE_8_SYSTEM_PATHS.agenticQeRoot,
    mcpExecutable: ISSUE_8_SYSTEM_PATHS.agenticQeMcp,
  });
  const proxyLauncher = join(dirname(fileURLToPath(import.meta.url)), 'native-proxy-launcher.js');
  return await runIssue8Transaction({
    repositoryRoot: transactionRepository,
    controllerSourceRoot: invocation.repositoryRoot,
    controllerRepositoryRoot: invocation.controllerStore,
    runRoot: paths.worktrees,
    evaluatorScratchRoot: paths.evaluator,
    controllerCommit: invocation.controllerCommit,
    runId: invocation.runId,
    models: MODELS,
    toolVersions: {
      ...systemTools,
      bootstrapSource: bootstrap.source,
      bootstrapControllerStoreDigest: bootstrap.controllerStoreDigest,
      bootstrapBuildManifestDigest: bootstrap.buildManifestDigest,
      bootstrapRuntimeTreeDigest: bootstrap.runtimeTreeDigest,
      bootstrapNodeDigest: bootstrap.nodeDigest,
      bootstrapGitDigest: bootstrap.gitDigest,
      rufloHive: invocation.hiveId,
      rufloConsensus: invocation.consensusId,
      agenticQe: '3.13.10#sast-only-flat-v1+lcov-gap',
    },
    createRustProfile: (controlledRoot) => createRustOfflineProfile({
      writableRoot: assertControlledRoot(scratch, controlledRoot),
      cargoExecutable: rustClosure.cargoExecutable,
      toolchainRoot: rustClosure.toolchainRoot,
      registryRoot: rustClosure.registryRoot,
      registryKey: ISSUE_8_SYSTEM_PATHS.registryKey,
      cargoExtensionRoot,
      bwrapExecutable: ISSUE_8_SYSTEM_PATHS.bwrap,
      resourceBoundary: new SystemdResourceBoundary({
        executablePath: ISSUE_8_SYSTEM_PATHS.systemdRun,
        systemctlPath: ISSUE_8_SYSTEM_PATHS.systemctl,
        terminationGraceMs: SECURE_HARNESS_CONFIG.limits.terminationGraceMs,
        sourceEnvironment: process.env,
      }),
      resourceLimits: ISSUE_8_RUST_LIMITS,
      assertClosureStable: rustClosure.assertStable,
    }),
    prepareFrozenLockfile: async ({ task, rustProfile }) => await prepareFrozenCargoLock({
      repositoryRoot: transactionRepository,
      scratchRoot: paths.frozen,
      baseline: task.baseline,
      source: task.sourceFix,
      cargoExecutable: rustProfile.cargoExecutable,
      cargoEnvironment: rustProfile.environment,
      config: SECURE_HARNESS_CONFIG,
      executor: createStructuredFrozenCargoLockExecutor({
        config: SECURE_HARNESS_CONFIG,
        offlineIsolator: rustProfile.isolator,
        sourceEnvironment: {},
      }),
    }),
    createNativeSession: async ({ prepared, evaluatorPaths, models }) =>
      await createIssue8NativeSession({
        config: SECURE_HARNESS_CONFIG,
        controllerRoot: invocation.repositoryRoot,
        runtimeParent: paths.native,
        prepared,
        evaluatorPaths,
        models,
        executables: {
          codex: ISSUE_8_SYSTEM_PATHS.codex,
          claude: ISSUE_8_SYSTEM_PATHS.claude,
          node: ISSUE_8_SYSTEM_PATHS.node,
          bwrap: ISSUE_8_SYSTEM_PATHS.bwrap,
          systemdRun: ISSUE_8_SYSTEM_PATHS.systemdRun,
          systemctl: ISSUE_8_SYSTEM_PATHS.systemctl,
          proxyLauncher,
        },
        credentials: {
          codex: ISSUE_8_SYSTEM_PATHS.codexCredential,
          'claude-code': ISSUE_8_SYSTEM_PATHS.claudeCredential,
        },
        resourceLimits: ISSUE_8_NATIVE_LIMITS,
        controllerEnvironment: process.env,
      }),
    captureRufloEvidence: ({ taskId, runId, routeSnapshotDigest }) => ({
      schemaVersion: 1,
      source: 'ruflo-coordination-ledger',
      taskId,
      runId,
      swarmId: invocation.swarmId,
      coordinationTaskId: invocation.coordinationTaskId,
      hookIds: [],
      traceIds: [],
      routeSnapshotDigest,
      authoritative: false,
      capturedAt: new Date().toISOString(),
    }),
    agenticQeEvidence: qe,
  });
}

async function cloneControllerRepository(
  source: string,
  target: string,
  template: string,
  commit: string,
): Promise<string> {
  assertAbsent(target, 'HARNESS_ISSUE_8_TRANSACTION_REPOSITORY_EXISTS');
  const clone = await runGitCommand(dirname(target), [
    'clone', '--bare', '--no-hardlinks', '--local', '--no-tags',
    `--template=${template}`, '--', source, target,
  ], { timeoutMs: 120_000, maxOutputBytes: 1_000_000 });
  if (clone.exitCode !== 0) throw new Error('HARNESS_ISSUE_8_TRANSACTION_REPOSITORY_CLONE_FAILED');
  const removeRemote = await runGitCommand(target, ['config', '--remove-section', 'remote.origin']);
  if (removeRemote.exitCode !== 0) {
    throw new Error('HARNESS_ISSUE_8_TRANSACTION_REPOSITORY_CONFIG_INVALID');
  }
  const fsck = await runGitCommand(target, [
    'fsck', '--strict', '--full', '--no-reflogs', commit,
  ], { timeoutMs: 120_000, maxOutputBytes: 10_000_000 });
  const head = await runGitCommand(target, ['rev-parse', '--verify', 'HEAD']);
  if (fsck.exitCode !== 0 || head.exitCode !== 0 || head.stdout.trim() !== commit) {
    throw new Error('HARNESS_ISSUE_8_TRANSACTION_REPOSITORY_VERIFY_FAILED');
  }
  const root = privateDirectory(target, 'TRANSACTION_REPOSITORY');
  const pending = [root];
  let entries = 0;
  while (pending.length > 0) {
    const directory = pending.pop()!;
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      entries += 1;
      if (entries > 100) throw new Error('HARNESS_ISSUE_8_TRANSACTION_REPOSITORY_LAYOUT_INVALID');
      const path = join(directory, entry.name);
      const stat = lstatSync(path);
      if (stat.isSymbolicLink() || (stat.mode & 0o077) !== 0
        || (!stat.isDirectory() && (!stat.isFile() || stat.nlink !== 1))) {
        throw new Error('HARNESS_ISSUE_8_TRANSACTION_REPOSITORY_LAYOUT_INVALID');
      }
      if (stat.isDirectory()) pending.push(path);
    }
  }
  assertAbsent(join(root, 'objects', 'info', 'alternates'),
    'HARNESS_ISSUE_8_TRANSACTION_REPOSITORY_ALTERNATE');
  return root;
}

async function prepareScratchLayout(scratch: string) {
  const paths = {
    worktrees: join(scratch, 'worktrees'),
    evaluator: join(scratch, 'evaluator'),
    frozen: join(scratch, 'frozen'),
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

function parseInvocation(argv: readonly string[]): Issue8Invocation {
  const expected = [
    'repository', 'controller-store', 'controller-commit', 'run-id', 'swarm-id',
    'coordination-task-id', 'hive-id', 'consensus-id',
  ];
  if (argv.length !== expected.length * 2) throw new Error('HARNESS_ISSUE_8_ARGUMENTS_INVALID');
  const values = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const name = flag.startsWith('--') ? flag.slice(2) : '';
    const value = argv[index + 1];
    if (!expected.includes(name) || values.has(name) || value === undefined || value.includes('\0')) {
      throw new Error('HARNESS_ISSUE_8_ARGUMENTS_INVALID');
    }
    values.set(name, value);
  }
  const repositoryRoot = canonicalDirectory(required(values, 'repository'));
  const controllerStore = privateDirectory(required(values, 'controller-store'), 'CONTROLLER_STORE');
  const controllerCommit = required(values, 'controller-commit');
  if (!GIT_OBJECT.test(controllerCommit)) throw new Error('HARNESS_ISSUE_8_CONTROLLER_COMMIT_INVALID');
  return Object.freeze({
    repositoryRoot,
    controllerStore,
    controllerCommit,
    runId: opaque(required(values, 'run-id'), 'RUN_ID'),
    swarmId: opaque(required(values, 'swarm-id'), 'SWARM_ID'),
    coordinationTaskId: opaque(required(values, 'coordination-task-id'), 'TASK_ID'),
    hiveId: opaque(required(values, 'hive-id'), 'HIVE_ID'),
    consensusId: opaque(required(values, 'consensus-id'), 'CONSENSUS_ID'),
  });
}

function parseBootstrap(value: unknown): TrustedBootstrapEvidence {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('HARNESS_ISSUE_8_BOOTSTRAP_EVIDENCE_INVALID');
  }
  const input = value as Record<string, unknown>;
  const keys = Object.keys(input).sort();
  const expected = [
    'buildManifestDigest', 'controllerCommit', 'controllerStoreDigest', 'gitDigest', 'nodeDigest',
    'runtimeTreeDigest', 'schemaVersion', 'source',
  ].sort();
  if (JSON.stringify(keys) !== JSON.stringify(expected)
    || input.schemaVersion !== 2 || input.source !== 'verified-packed-private-runtime') {
    throw new Error('HARNESS_ISSUE_8_BOOTSTRAP_EVIDENCE_INVALID');
  }
  const controllerCommit = String(input.controllerCommit);
  if (!GIT_OBJECT.test(controllerCommit)) throw new Error('HARNESS_ISSUE_8_BOOTSTRAP_EVIDENCE_INVALID');
  const parsed = {
    schemaVersion: 2 as const,
    source: 'verified-packed-private-runtime' as const,
    controllerCommit,
    controllerStoreDigest: hash(input.controllerStoreDigest),
    buildManifestDigest: hash(input.buildManifestDigest),
    runtimeTreeDigest: hash(input.runtimeTreeDigest),
    nodeDigest: hash(input.nodeDigest),
    gitDigest: hash(input.gitDigest),
  };
  return Object.freeze(parsed);
}

async function prepareResultsRoot(repositoryRoot: string): Promise<string> {
  const root = join(repositoryRoot, 'coding-harness', '.metaharness');
  const runs = join(root, 'runs');
  await mkdir(root, { recursive: true, mode: 0o700 });
  await mkdir(runs, { recursive: true, mode: 0o700 });
  privateDirectory(root, 'RESULT_ROOT');
  return privateDirectory(runs, 'RESULTS_ROOT');
}

async function createScratchRoot(): Promise<string> {
  await mkdir(SCRATCH_PARENT, { recursive: true, mode: 0o700 });
  const parent = privateDirectory(SCRATCH_PARENT, 'SCRATCH_PARENT');
  const root = await mkdtemp(join(parent, 'i8-'));
  chmodSync(root, 0o700);
  return privateDirectory(root, 'SCRATCH_ROOT');
}

function assertControlledRoot(scratch: string, controlledRoot: string): string {
  const child = canonicalDirectory(controlledRoot);
  const delta = relative(scratch, child);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error('HARNESS_ISSUE_8_CONTROLLED_ROOT_INVALID');
  }
  return scratch;
}

async function removeScratch(path: string): Promise<void> {
  privateDirectory(path, 'SCRATCH_ROOT_CHANGED');
  await rm(path, { recursive: true, force: true });
}

function privateDirectory(value: string, label: string): string {
  const path = canonicalDirectory(value);
  const stat = lstatSync(path);
  const uid = process.getuid?.() ?? stat.uid;
  if (stat.uid !== uid || (stat.mode & 0o077) !== 0) {
    throw new Error(`HARNESS_ISSUE_8_${label}_INVALID`);
  }
  return path;
}

function canonicalDirectory(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error('HARNESS_ISSUE_8_DIRECTORY_INVALID');
  }
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error('HARNESS_ISSUE_8_DIRECTORY_INVALID');
  }
  return value;
}

function assertAbsent(path: string, error: string): void {
  try { lstatSync(path); } catch (caught) {
    if ((caught as NodeJS.ErrnoException).code === 'ENOENT') return;
    throw caught;
  }
  throw new Error(error);
}

function required(values: ReadonlyMap<string, string>, name: string): string {
  const value = values.get(name);
  if (value === undefined || value.length === 0) throw new Error('HARNESS_ISSUE_8_ARGUMENTS_INVALID');
  return value;
}

function opaque(value: string, label: string): string {
  if (!OPAQUE_ID.test(value)) throw new Error(`HARNESS_ISSUE_8_${label}_INVALID`);
  return value;
}

function hash(value: unknown): string {
  const text = typeof value === 'string' ? value : '';
  if (!/^[a-f0-9]{64}$/.test(text) || text === '0'.repeat(64)) {
    throw new Error('HARNESS_ISSUE_8_BOOTSTRAP_DIGEST_INVALID');
  }
  return text;
}

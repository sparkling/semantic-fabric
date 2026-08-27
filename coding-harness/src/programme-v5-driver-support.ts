// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { VerifierRegistry, predicateVerifier } from '@metaharness/harness';
import {
  acceptanceTaskPrompt,
  type AcceptanceTask,
  type AcceptanceTaskV3,
} from './acceptance-task.js';
import type { ControllerAttestation } from './controller-attestation.js';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  deepFreeze,
  parseTaskContract,
} from './contracts.js';
import { runGitCommand } from './git-process.js';
import type { Issue8NativeSession } from './issue-8-driver.js';
import type { NativePreflightExecutableIdentitySnapshot } from './native-runtime-ledger.js';
import {
  PersistentRoutedAgentPool,
  VerifiedRoutingHistory,
  type NativeModelCandidate,
} from './models/routing.js';
import type { ProgrammeGateContractV1 } from './programme-gate-contract-v1.js';
import { SECURE_HARNESS_CONFIG } from './config.js';
import { digestValue, type GitIdentity, type HostEvidence } from './receipts.js';

const DRIVER_KEYS = Object.freeze([
  'controllerExecutionDigest',
  'controllerBuildManifestDigest',
  'controllerRuntimeTreeDigest',
  'controllerManifestDigest',
  'controllerTaskPath',
  'controllerTaskPathDigest',
  'controllerTaskDigest',
  'taskEvidencePlanDigest',
  'boundTaskDigest',
  'programmePolicyFingerprint',
  'frozenCargoLockDigest',
  'codex',
  'claude',
] as const);

const LOCKED_RUST_KEYS = Object.freeze([
  'rustRegistryClosure',
  'rustRegistryLock',
  'rustRegistrySelection',
  'rustRegistryMetadata',
] as const);

const RECHECKED_SYSTEM_TOOL_KEYS = Object.freeze([
  'cargo', 'cargoLlvmCov', 'node', 'codexExecutable', 'claudeExecutable',
  'bwrap', 'systemdRun', 'systemctl', 'caBundle', 'agenticQeMcp', 'agenticQe',
  'agenticQePackageTreeDigest',
] as const);

export interface ProgrammeV5BootstrapInputs {
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

export function assertProgrammeV5ControllerTask(
  controller: ControllerAttestation,
  execution: Readonly<{ taskPath: string; controllerCommit: string }>,
  bootstrap: ProgrammeV5BootstrapInputs,
): asserts controller is ControllerAttestation & { readonly task: AcceptanceTaskV3 } {
  if (controller.taskPath !== execution.taskPath || controller.task.schemaVersion !== 3
    || controller.task.candidateOracle.mode !== 'verifier-only') {
    throw new Error('HARNESS_PROGRAMME_V5_VERIFIER_ONLY_TASK_REQUIRED');
  }
  if (bootstrap.controllerCommit !== execution.controllerCommit
    || bootstrap.taskPath !== execution.taskPath
    || bootstrap.buildManifestDigest !== controller.buildManifestBlobDigest
    || bootstrap.runtimeTreeDigest !== controller.build.runtimeTreeDigest) {
    throw new Error('HARNESS_PROGRAMME_V5_BOOTSTRAP_BINDING_MISMATCH');
  }
}

export function requireProgrammeV5Task(task: AcceptanceTask): AcceptanceTaskV3 {
  if (task.schemaVersion !== 3 || task.candidateOracle.mode !== 'verifier-only') {
    throw new Error('HARNESS_PROGRAMME_V5_VERIFIER_ONLY_TASK_REQUIRED');
  }
  return task;
}

export function assertProgrammeV5RunBindings(runId: string, controllerCommit: string): void {
  if (!/^[A-Za-z0-9_-]{8,128}$/.test(runId)
    || !/^[a-f0-9]{40,64}$/.test(controllerCommit)) {
    throw new Error('HARNESS_PROGRAMME_V5_RUN_BINDINGS_INVALID');
  }
}

export function uniqueProgrammeV5Strings(values: readonly string[]): string[] {
  return [...new Set(values)];
}

export function canonicalProgrammePolicyJson(value: unknown): string {
  const seen = new Set<object>();
  const canonical = (entry: unknown, label: string): unknown => {
    if (entry === null || typeof entry === 'string' || typeof entry === 'boolean') return entry;
    if (typeof entry === 'number') {
      if (!Number.isFinite(entry)) throw new TypeError(`${label} contains a non-finite number`);
      return entry;
    }
    if (typeof entry !== 'object') throw new TypeError(`${label} is not JSON serializable`);
    if (seen.has(entry)) throw new TypeError(`${label} contains a cycle`);
    seen.add(entry);
    try {
      if (Array.isArray(entry)) {
        return entry.map((item, index) => canonical(item, `${label}[${index}]`));
      }
      const record = entry as Record<string, unknown>;
      const prototype = Object.getPrototypeOf(record);
      if (prototype !== Object.prototype && prototype !== null) {
        throw new TypeError(`${label} contains a non-JSON object`);
      }
      if (Reflect.ownKeys(record).some((key) => typeof key !== 'string'
        || !Object.prototype.propertyIsEnumerable.call(record, key))) {
        throw new TypeError(`${label} contains hidden properties`);
      }
      return Object.fromEntries(Object.keys(record).sort().map((key) => [
        key,
        canonical(record[key], `${label}.${key}`),
      ]));
    } finally {
      seen.delete(entry);
    }
  };
  return JSON.stringify(canonical(value, 'programme policy v5'));
}

export function captureProgrammeV5Bootstrap(value: ProgrammeV5BootstrapInputs): ProgrammeV5BootstrapInputs {
  assertExactKeys(value, [
    'schemaVersion', 'source', 'controllerCommit', 'taskPath', 'controllerStoreDigest',
    'buildManifestDigest', 'runtimeTreeDigest', 'nodeDigest', 'gitDigest',
  ], 'HARNESS_PROGRAMME_V5_BOOTSTRAP_EVIDENCE_INVALID');
  if (value.schemaVersion !== 3 || value.source !== 'verified-packed-private-runtime'
    || !/^[a-f0-9]{40,64}$/.test(value.controllerCommit)
    || typeof value.taskPath !== 'string' || value.taskPath.length === 0) {
    throw new Error('HARNESS_PROGRAMME_V5_BOOTSTRAP_EVIDENCE_INVALID');
  }
  for (const digest of [
    value.controllerStoreDigest, value.buildManifestDigest, value.runtimeTreeDigest,
    value.nodeDigest, value.gitDigest,
  ]) assertDigest(digest, 'HARNESS_PROGRAMME_V5_BOOTSTRAP_EVIDENCE_INVALID');
  return deepFreeze({ ...value });
}

export function assembleProgrammeV5ToolVersions(input: Readonly<{
  contract: ProgrammeGateContractV1;
  base: Readonly<Record<string, string>>;
  lockedRust: Readonly<Record<string, string>>;
  bootstrap: ProgrammeV5BootstrapInputs;
  controller: ControllerAttestation;
  taskEvidencePlanDigest: string;
  boundTaskDigest: string;
  policyFingerprint: string;
  frozenCargoLockDigest: string;
  hosts: readonly HostEvidence[];
}>): Readonly<Record<string, string>> {
  const required = [...input.contract.tools.requiredKeys];
  const baseKeys = required.filter((key) => !DRIVER_KEYS.includes(key as never)
    && !LOCKED_RUST_KEYS.includes(key as never));
  assertStringRecord(input.base, baseKeys, 'HARNESS_PROGRAMME_V5_BASE_TOOL_SET_MISMATCH');
  assertStringRecord(input.lockedRust, LOCKED_RUST_KEYS,
    'HARNESS_PROGRAMME_V5_LOCKED_RUST_TOOL_SET_MISMATCH');
  const clients = new Map(input.hosts.map((host) => [host.host, host.clientVersion]));
  if (input.hosts.length !== 2 || clients.size !== 2
    || !clients.has('codex') || !clients.has('claude-code')) {
    throw new Error('HARNESS_PROGRAMME_V5_NATIVE_SESSION_MISMATCH');
  }
  const dynamic = {
    controllerExecutionDigest: input.controller.executionDigest,
    controllerBuildManifestDigest: input.controller.buildManifestBlobDigest,
    controllerRuntimeTreeDigest: input.controller.build.runtimeTreeDigest,
    controllerManifestDigest: input.controller.manifestBlobDigest,
    controllerTaskPath: input.controller.taskPath,
    controllerTaskPathDigest: digestValue(input.controller.taskPath),
    controllerTaskDigest: input.controller.taskBlobDigest,
    taskEvidencePlanDigest: input.taskEvidencePlanDigest,
    boundTaskDigest: input.boundTaskDigest,
    programmePolicyFingerprint: input.policyFingerprint,
    frozenCargoLockDigest: input.frozenCargoLockDigest,
    codex: clients.get('codex')!,
    claude: clients.get('claude-code')!,
  };
  const tools: Readonly<Record<string, string>> = deepFreeze({
    ...input.base, ...input.lockedRust, ...dynamic,
  });
  assertStringRecord(tools, required, 'HARNESS_PROGRAMME_V5_TOOL_SET_MISMATCH');
  assertBootstrapToolBindings(tools, input.bootstrap);
  if (tools.codexExecutable === tools.claudeExecutable) {
    throw new Error('HARNESS_PROGRAMME_V5_NATIVE_EXECUTABLES_NOT_DISTINCT');
  }
  for (const [key, expected] of Object.entries(input.contract.tools.exactValues)) {
    if (tools[key] !== expected) throw new Error(`HARNESS_PROGRAMME_V5_TOOL_VALUE_MISMATCH:${key}`);
  }
  for (const key of input.contract.tools.digestValueKeys) {
    assertDigest(tools[key], `HARNESS_PROGRAMME_V5_TOOL_VALUE_MISMATCH:${key}`);
  }
  for (const key of input.contract.tools.nonEmptyValueKeys) {
    if (tools[key]?.trim().length === 0) {
      throw new Error(`HARNESS_PROGRAMME_V5_TOOL_VALUE_MISMATCH:${key}`);
    }
  }
  assertRustEvidence(tools, input.frozenCargoLockDigest);
  return tools;
}

export function programmeV5TaskContract(
  task: AcceptanceTaskV3,
  runId: string,
  workspaceRoot: string,
  protectedPaths: readonly string[],
) {
  return parseTaskContract({
    schemaVersion: 1,
    taskId: task.taskId,
    runId,
    workspaceRoot,
    readablePaths: task.implementationPaths,
    mutablePaths: task.implementationPaths,
    protectedPaths,
    tools: ['cargo', 'apply_patch'],
    commands: programmeV5Commands(task),
    network: { mode: 'offline', allowedOrigins: [] },
    authority: DEVELOPMENT_AUTHORITY,
  }, SECURE_HARNESS_CONFIG);
}

export function programmeV5Commands(task: AcceptanceTaskV3) {
  return [
    ...task.redBaseline.commands,
    ...task.commands.build,
    ...task.commands.public,
    ...task.commands.independent,
    ...task.commands.regression,
    ...task.commands.mutation,
  ].map(({ command }) => command);
}

export function programmeV5RoutedPool(
  runId: string,
  task: AcceptanceTaskV3,
  candidates: readonly NativeModelCandidate[],
): PersistentRoutedAgentPool {
  return new PersistentRoutedAgentPool({
    runId,
    task: {
      id: task.taskId,
      digest: digestValue(task),
      prompt: acceptanceTaskPrompt(task),
      tags: task.routing.tags,
      difficulty: task.routing.difficulty,
    },
    candidates,
    history: new VerifiedRoutingHistory(),
    embedder: {
      dimensions: 16,
      embed: (text) => [...createHash('sha256').update(text).digest().subarray(0, 16)]
        .map((value) => value / 255),
    },
  });
}

export function programmeV5ArchitectureVerifiers(): VerifierRegistry {
  return new VerifierRegistry().register(predicateVerifier(
    'repository-change-architecture-shape',
    'architecture',
    (value) => value !== null && typeof value === 'object' && !Array.isArray(value)
      && Buffer.byteLength(JSON.stringify(value), 'utf8') <= 64_000,
  ));
}

export function assertProgrammeV5NativeSession(
  session: Issue8NativeSession,
  models: Readonly<{ codex: string; claude: string }>,
): void {
  const candidates = new Map(session.candidates.map(({ host, model }) => [host, model]));
  const hosts = new Map(session.hosts.map(({ host, model }) => [host, model]));
  if (session.candidates.length !== 2 || candidates.size !== 2
    || session.hosts.length !== 2 || hosts.size !== 2
    || candidates.get('codex') !== models.codex || candidates.get('claude-code') !== models.claude
    || hosts.get('codex') !== models.codex || hosts.get('claude-code') !== models.claude) {
    throw new Error('HARNESS_PROGRAMME_V5_NATIVE_SESSION_MISMATCH');
  }
}

export function assertProgrammeV5NativeExecutableBindings(
  snapshot: NativePreflightExecutableIdentitySnapshot,
  toolVersions: Readonly<Record<string, string>>,
): void {
  const hosts = ['codex', 'claude-code'] as const;
  const keys = ['codexExecutable', 'claudeExecutable'] as const;
  for (let index = 0; index < hosts.length; index += 1) {
    const identity = snapshot[index];
    const claim = identity === undefined
      ? ''
      : `${identity.path}#sha256:${identity.digest}`;
    if (identity?.host !== hosts[index] || toolVersions[keys[index]] !== claim) {
      throw new Error(`HARNESS_PROGRAMME_V5_NATIVE_EXECUTABLE_BINDING_MISMATCH:${hosts[index]}`);
    }
  }
}

export function assertProgrammeV5ToolRecheck(
  initial: Readonly<Record<string, string>>,
  current: Readonly<Record<string, string>>,
): void {
  assertStringRecord(
    current,
    RECHECKED_SYSTEM_TOOL_KEYS,
    'HARNESS_PROGRAMME_V5_TOOL_RECHECK_SET_MISMATCH',
  );
  for (const key of RECHECKED_SYSTEM_TOOL_KEYS) {
    if (current[key] !== initial[key]) {
      throw new Error(`HARNESS_PROGRAMME_V5_SYSTEM_TOOL_CHANGED:${key}`);
    }
  }
}

export async function assertProgrammeV5GitIdentity(
  root: string,
  expected: GitIdentity,
  label: string,
): Promise<void> {
  const commit = await gitValue(root, ['rev-parse', '--verify', `${expected.commit}^{commit}`]);
  const tree = await gitValue(root, ['rev-parse', `${expected.commit}^{tree}`]);
  if (commit !== expected.commit || tree !== expected.tree) {
    throw new Error(`HARNESS_PROGRAMME_V5_${label}_IDENTITY_MISMATCH`);
  }
}

export function canonicalProgrammeTimestamp(value: string): string {
  if (Number.isNaN(Date.parse(value)) || new Date(value).toISOString() !== value) {
    throw new Error('HARNESS_PROGRAMME_V5_FROZEN_AT_INVALID');
  }
  return value;
}

export async function cleanupProgrammeV5Resources(
  cleanups: readonly (() => Promise<void>)[],
): Promise<void> {
  const outcomes = await Promise.allSettled(cleanups.map(async (cleanup) => await cleanup()));
  const failures = outcomes.filter((outcome) => outcome.status === 'rejected');
  if (failures.length > 0) {
    throw new AggregateError(
      failures.map((outcome) => (outcome as PromiseRejectedResult).reason),
      'HARNESS_PROGRAMME_V5_RESOURCE_CLEANUP_FAILED',
    );
  }
}

async function gitValue(root: string, args: readonly string[]): Promise<string> {
  const result = await runGitCommand(root, args, { maxOutputBytes: 1024 });
  if (result.exitCode !== 0) throw new Error(`HARNESS_PROGRAMME_V5_GIT_FAILED:${args[0]}`);
  return result.stdout.trim();
}

function assertBootstrapToolBindings(
  tools: Readonly<Record<string, string>>,
  bootstrap: ProgrammeV5BootstrapInputs,
): void {
  const expected = {
    bootstrapSource: bootstrap.source,
    bootstrapControllerStoreDigest: bootstrap.controllerStoreDigest,
    bootstrapBuildManifestDigest: bootstrap.buildManifestDigest,
    bootstrapRuntimeTreeDigest: bootstrap.runtimeTreeDigest,
    bootstrapNodeDigest: bootstrap.nodeDigest,
    bootstrapGitDigest: bootstrap.gitDigest,
  };
  for (const [key, value] of Object.entries(expected)) {
    if (tools[key] !== value) throw new Error(`HARNESS_PROGRAMME_V5_BOOTSTRAP_TOOL_MISMATCH:${key}`);
  }
}

function assertRustEvidence(tools: Readonly<Record<string, string>>, lockDigest: string): void {
  const positive = '[1-9][0-9]*';
  const digest = '[a-f0-9]{64}';
  const rules: Readonly<Record<string, RegExp>> = {
    rustToolchainClosure: new RegExp(`^81cc515ef94bae07d2451ff3701ce6e6eee7878327dc8088ebac773f1570f7c4:${positive}:${positive}$`),
    rustRegistryBootstrapSnapshot: new RegExp(`^${digest}:${positive}:${positive}$`),
    rustRegistryClosure: new RegExp(`^1bb717af28554b8cbb83ff1a219bbbd294ccee98691191bc9f65dc431106e908:${positive}:${positive}$`),
    rustRegistryLock: new RegExp(`^${lockDigest}:${positive}:${positive}$`),
    rustRegistrySelection: new RegExp(`^x86_64-unknown-linux-gnu:${digest}$`),
  };
  for (const [key, rule] of Object.entries(rules)) {
    if (!rule.test(tools[key] ?? '')) throw new Error(`HARNESS_PROGRAMME_V5_TOOL_VALUE_MISMATCH:${key}`);
  }
}

function assertStringRecord(
  value: Readonly<Record<string, string>>,
  expectedKeys: readonly string[],
  error: string,
): void {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) throw new Error(error);
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) throw new Error(error);
  assertExactKeys(value, expectedKeys, error);
  if (Object.values(value).some((entry) => typeof entry !== 'string' || entry.length === 0)) {
    throw new Error(error);
  }
}

function assertExactKeys(value: object, expectedKeys: readonly string[], error: string): void {
  const ownKeys = Reflect.ownKeys(value);
  if (ownKeys.some((key) => typeof key !== 'string'
    || !Object.prototype.propertyIsEnumerable.call(value, key))
    || JSON.stringify((ownKeys as string[]).sort()) !== JSON.stringify([...expectedKeys].sort())) {
    throw new Error(error);
  }
}

function assertDigest(value: unknown, error: string): asserts value is string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || value === '0'.repeat(64)) {
    throw new Error(error);
  }
}

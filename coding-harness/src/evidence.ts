// SPDX-License-Identifier: MIT

import {
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  asUniqueStrings,
  deepFreeze,
} from './contracts.js';
import { isAbsolute, resolve } from 'node:path';
import { digestValue, type HostEvidence } from './receipts.js';
import type { NativeHost } from './models/types.js';

export type AgenticQeProfile =
  | 'lcov-gap'
  | 'rust-testgen-no-ai'
  | 'quality-contract'
  | 'sast';

export interface RufloEvidence {
  schemaVersion: 1;
  source: 'ruflo-coordination-ledger';
  taskId: string;
  runId: string;
  swarmId: string;
  coordinationTaskId: string;
  hookIds: string[];
  traceIds: string[];
  routeSnapshotDigest: string;
  authoritative: false;
  capturedAt: string;
}

export interface AgenticQeEvidence {
  schemaVersion: 1;
  source: 'agentic-qe-local-profile';
  profile: AgenticQeProfile;
  taskId: string;
  runId: string;
  candidateTree: string;
  commandDigest: string;
  outputDigest: string;
  providerVariablesStripped: true;
  authoritative: false;
  capturedAt: string;
}

export interface BoundExternalEvidence {
  ruflo: RufloEvidence;
  qe: AgenticQeEvidence[];
  qeDigests: string[];
}

export type NativeModelOperation = 'architecture' | 'implementation' | 'repair' | 'review';

export interface NativeInvocationExpectation {
  invocationId: string;
  operation: NativeModelOperation;
  candidateTree: string;
  host?: NativeHost;
}

export interface NativeRuntimeEvidence {
  schemaVersion: 1;
  source: 'trusted-native-runtime';
  taskId: string;
  runId: string;
  hosts: ReadonlyArray<{
    host: NativeHost;
    model: string;
    authentication: 'chatgpt-subscription' | 'claude-subscription';
    clientVersion: string;
    executablePath: string;
    executableDigest: string;
    preflightDigest: string;
  }>;
  invocations: ReadonlyArray<{
    invocationId: string;
    host: NativeHost;
    model: string;
    operation: NativeModelOperation;
    candidateTree: string;
    environmentDigest: string;
    outputDigest: string;
    exitCode: 0;
    network: {
      enforcement: 'origin-pinned-process-boundary';
      mechanism: string;
      pinnedOrigins: string[];
    };
    filesystem: {
      enforcement: 'os-filesystem-namespace';
      mechanism: string;
      workspaceRootDigest: string;
      mountManifestDigest: string;
      outputChannelDigest: string;
      hostFileConfidentiality: true;
      emptyPrivateHome: true;
      hostRootMounted: false;
      gitMetadataMasked: true;
    };
  }>;
}

const RUFLO_KEYS = [
  'schemaVersion', 'source', 'taskId', 'runId', 'swarmId', 'coordinationTaskId',
  'hookIds', 'traceIds', 'routeSnapshotDigest', 'authoritative', 'capturedAt',
] as const;
const QE_KEYS = [
  'schemaVersion', 'source', 'profile', 'taskId', 'runId', 'candidateTree',
  'commandDigest', 'outputDigest', 'providerVariablesStripped', 'authoritative',
  'capturedAt',
] as const;
const QE_PROFILES = new Set<AgenticQeProfile>([
  'lcov-gap', 'rust-testgen-no-ai', 'quality-contract', 'sast',
]);
const NATIVE_HOST_KEYS = [
  'host', 'model', 'authentication', 'clientVersion', 'executablePath',
  'executableDigest', 'preflightDigest',
] as const;
const NATIVE_INVOCATION_KEYS = [
  'invocationId', 'host', 'model', 'operation', 'candidateTree', 'environmentDigest',
  'outputDigest', 'exitCode', 'network', 'filesystem',
] as const;
const NATIVE_ORIGINS: Readonly<Record<NativeHost, readonly string[]>> = Object.freeze({
  codex: Object.freeze(['https://api.openai.com', 'https://chatgpt.com']),
  'claude-code': Object.freeze(['https://api.anthropic.com', 'https://claude.ai']),
});

export function parseRufloEvidence(value: unknown): RufloEvidence {
  const input = asRecord(value, 'ruflo evidence');
  assertExactKeys(input, RUFLO_KEYS, 'ruflo evidence');
  if (input.schemaVersion !== 1) throw new TypeError('ruflo evidence schemaVersion must be 1');
  if (input.source !== 'ruflo-coordination-ledger') throw new TypeError('ruflo evidence source is invalid');
  if (input.authoritative !== false) throw new TypeError('Ruflo evidence must remain non-authoritative');
  return deepFreeze({
    schemaVersion: 1,
    source: 'ruflo-coordination-ledger',
    taskId: asNonEmptyString(input.taskId, 'ruflo evidence.taskId'),
    runId: asNonEmptyString(input.runId, 'ruflo evidence.runId'),
    swarmId: asNonEmptyString(input.swarmId, 'ruflo evidence.swarmId'),
    coordinationTaskId: asNonEmptyString(
      input.coordinationTaskId,
      'ruflo evidence.coordinationTaskId',
    ),
    hookIds: asUniqueStrings(input.hookIds, 'ruflo evidence.hookIds', true),
    traceIds: asUniqueStrings(input.traceIds, 'ruflo evidence.traceIds', true),
    routeSnapshotDigest: parseDigest(input.routeSnapshotDigest, 'ruflo evidence.routeSnapshotDigest'),
    authoritative: false,
    capturedAt: parseTimestamp(input.capturedAt, 'ruflo evidence.capturedAt'),
  });
}

export function parseAgenticQeEvidence(value: unknown): AgenticQeEvidence {
  const input = asRecord(value, 'Agentic-QE evidence');
  assertExactKeys(input, QE_KEYS, 'Agentic-QE evidence');
  if (input.schemaVersion !== 1) throw new TypeError('Agentic-QE evidence schemaVersion must be 1');
  if (input.source !== 'agentic-qe-local-profile') throw new TypeError('Agentic-QE evidence source is invalid');
  if (!QE_PROFILES.has(input.profile as AgenticQeProfile)) throw new TypeError('Agentic-QE evidence profile is invalid');
  if (input.providerVariablesStripped !== true) {
    throw new TypeError('Agentic-QE evidence must prove provider variables were stripped');
  }
  if (input.authoritative !== false) throw new TypeError('Agentic-QE evidence must remain non-authoritative');
  return deepFreeze({
    schemaVersion: 1,
    source: 'agentic-qe-local-profile',
    profile: input.profile as AgenticQeProfile,
    taskId: asNonEmptyString(input.taskId, 'Agentic-QE evidence.taskId'),
    runId: asNonEmptyString(input.runId, 'Agentic-QE evidence.runId'),
    candidateTree: parseGitTree(input.candidateTree),
    commandDigest: parseDigest(input.commandDigest, 'Agentic-QE evidence.commandDigest'),
    outputDigest: parseDigest(input.outputDigest, 'Agentic-QE evidence.outputDigest'),
    providerVariablesStripped: true,
    authoritative: false,
    capturedAt: parseTimestamp(input.capturedAt, 'Agentic-QE evidence.capturedAt'),
  });
}

export function bindExternalEvidence(input: {
  taskId: string;
  runId: string;
  candidateTree: string;
  ruflo: unknown;
  qe: readonly unknown[];
}): BoundExternalEvidence {
  const taskId = asNonEmptyString(input.taskId, 'binding.taskId');
  const runId = asNonEmptyString(input.runId, 'binding.runId');
  const candidateTree = parseGitTree(input.candidateTree);
  const ruflo = parseRufloEvidence(input.ruflo);
  if (ruflo.taskId !== taskId || ruflo.runId !== runId) {
    throw new Error('HARNESS_RUFLO_TASK_BINDING_MISMATCH');
  }
  if (!Array.isArray(input.qe)) throw new TypeError('binding.qe must be an array');
  const qe = input.qe.map(parseAgenticQeEvidence);
  const profiles = qe.map(({ profile }) => profile);
  if (new Set(profiles).size !== profiles.length) {
    throw new Error('HARNESS_QE_PROFILE_DUPLICATE');
  }
  for (const evidence of qe) {
    if (evidence.taskId !== taskId || evidence.runId !== runId) {
      throw new Error('HARNESS_QE_TASK_BINDING_MISMATCH');
    }
    if (evidence.candidateTree !== candidateTree) {
      throw new Error('HARNESS_QE_CANDIDATE_IDENTITY_MISMATCH');
    }
  }
  return deepFreeze({
    ruflo,
    qe,
    qeDigests: qe.map((evidence) => digestValue(evidence)),
  });
}

export function bindNativeRuntimeEvidence(input: {
  value: unknown;
  taskId: string;
  runId: string;
  hosts: readonly HostEvidence[];
  expectations: readonly NativeInvocationExpectation[];
}): NativeRuntimeEvidence {
  const value = asRecord(input.value, 'native runtime evidence');
  assertExactKeys(
    value,
    ['schemaVersion', 'source', 'taskId', 'runId', 'hosts', 'invocations'],
    'native runtime evidence',
  );
  if (value.schemaVersion !== 1 || value.source !== 'trusted-native-runtime') {
    throw new TypeError('native runtime evidence provenance is invalid');
  }
  const taskId = asNonEmptyString(value.taskId, 'native runtime evidence.taskId');
  const runId = asNonEmptyString(value.runId, 'native runtime evidence.runId');
  if (taskId !== input.taskId || runId !== input.runId) {
    throw new Error('HARNESS_NATIVE_TASK_BINDING_MISMATCH');
  }
  if (!Array.isArray(value.hosts) || value.hosts.length !== 2) {
    throw new TypeError('native runtime evidence must contain exactly two hosts');
  }
  const expectedHosts = new Map(input.hosts.map((host) => [host.host, host]));
  const hosts = value.hosts.map((entry, index) => parseNativeHostEvidence(
    entry,
    expectedHosts,
    `native runtime evidence.hosts[${index}]`,
  ));
  if (new Set(hosts.map(({ host }) => host)).size !== 2) {
    throw new Error('HARNESS_NATIVE_RUNTIME_HOST_COVERAGE_REQUIRED');
  }
  if (!Array.isArray(value.invocations) || value.invocations.length === 0) {
    throw new TypeError('native runtime evidence.invocations must be a non-empty array');
  }
  const invocations = value.invocations.map((entry, index) => parseNativeInvocation(
    entry,
    expectedHosts,
    `native runtime evidence.invocations[${index}]`,
  ));
  const ids = invocations.map(({ invocationId }) => invocationId);
  if (new Set(ids).size !== ids.length) throw new Error('HARNESS_NATIVE_INVOCATION_DUPLICATE');
  if (new Set(invocations.map(({ host }) => host)).size !== 2) {
    throw new Error('HARNESS_NATIVE_INVOCATION_DUAL_HOST_REQUIRED');
  }
  const expectations = new Map(input.expectations.map((entry) => [entry.invocationId, entry]));
  if (expectations.size !== input.expectations.length
    || expectations.size !== invocations.length
    || ids.some((id) => !expectations.has(id))) {
    throw new Error('HARNESS_NATIVE_INVOCATION_SET_MISMATCH');
  }
  for (const invocation of invocations) {
    const expected = expectations.get(invocation.invocationId) as NativeInvocationExpectation;
    if (invocation.operation !== expected.operation
      || invocation.candidateTree !== expected.candidateTree
      || (expected.host !== undefined && invocation.host !== expected.host)) {
      throw new Error('HARNESS_NATIVE_INVOCATION_BINDING_MISMATCH');
    }
  }
  return deepFreeze({
    schemaVersion: 1,
    source: 'trusted-native-runtime',
    taskId,
    runId,
    hosts,
    invocations,
  });
}

function parseNativeHostEvidence(
  value: unknown,
  expectedHosts: ReadonlyMap<NativeHost, HostEvidence>,
  label: string,
): NativeRuntimeEvidence['hosts'][number] {
  const entry = asRecord(value, label);
  assertExactKeys(entry, NATIVE_HOST_KEYS, label);
  const host = parseNativeHost(entry.host, `${label}.host`);
  const expected = expectedHosts.get(host);
  if (expected === undefined) throw new Error('HARNESS_NATIVE_RUNTIME_HOST_UNEXPECTED');
  const authentication = host === 'codex' ? 'chatgpt-subscription' : 'claude-subscription';
  if (entry.model !== expected.model || entry.clientVersion !== expected.clientVersion
    || entry.authentication !== authentication) {
    throw new Error('HARNESS_NATIVE_PREFLIGHT_BINDING_MISMATCH');
  }
  const executablePath = asNonEmptyString(entry.executablePath, `${label}.executablePath`);
  if (!isAbsolute(executablePath) || resolve(executablePath) !== executablePath || executablePath.includes('\0')) {
    throw new TypeError(`${label}.executablePath must be normalized and absolute`);
  }
  return deepFreeze({
    host,
    model: expected.model,
    authentication,
    clientVersion: expected.clientVersion,
    executablePath,
    executableDigest: parseDigest(entry.executableDigest, `${label}.executableDigest`),
    preflightDigest: parseDigest(entry.preflightDigest, `${label}.preflightDigest`),
  });
}

function parseNativeInvocation(
  value: unknown,
  expectedHosts: ReadonlyMap<NativeHost, HostEvidence>,
  label: string,
): NativeRuntimeEvidence['invocations'][number] {
  const entry = asRecord(value, label);
  assertExactKeys(entry, NATIVE_INVOCATION_KEYS, label);
  const host = parseNativeHost(entry.host, `${label}.host`);
  const expected = expectedHosts.get(host);
  if (expected === undefined || entry.model !== expected.model) {
    throw new Error('HARNESS_NATIVE_INVOCATION_MODEL_MISMATCH');
  }
  const operation = entry.operation;
  if (!['architecture', 'implementation', 'repair', 'review'].includes(operation as string)) {
    throw new TypeError(`${label}.operation is invalid`);
  }
  const network = parseNativeNetwork(entry.network, host, `${label}.network`);
  const filesystem = parseNativeFilesystem(entry.filesystem, `${label}.filesystem`);
  if (entry.exitCode !== 0) throw new Error('HARNESS_NATIVE_INVOCATION_DID_NOT_SUCCEED');
  return deepFreeze({
    invocationId: asNonEmptyString(entry.invocationId, `${label}.invocationId`),
    host,
    model: expected.model,
    operation: operation as NativeModelOperation,
    candidateTree: parseGitTree(entry.candidateTree, label),
    environmentDigest: parseDigest(entry.environmentDigest, `${label}.environmentDigest`),
    outputDigest: parseDigest(entry.outputDigest, `${label}.outputDigest`),
    exitCode: 0,
    network,
    filesystem,
  });
}

function parseNativeNetwork(
  value: unknown,
  host: NativeHost,
  label: string,
): NativeRuntimeEvidence['invocations'][number]['network'] {
  const entry = asRecord(value, label);
  assertExactKeys(entry, ['enforcement', 'mechanism', 'pinnedOrigins'], label);
  if (entry.enforcement !== 'origin-pinned-process-boundary') {
    throw new Error('HARNESS_NATIVE_NETWORK_ENFORCEMENT_REQUIRED');
  }
  const pinnedOrigins = asUniqueStrings(entry.pinnedOrigins, `${label}.pinnedOrigins`).sort();
  if (JSON.stringify(pinnedOrigins) !== JSON.stringify([...NATIVE_ORIGINS[host]].sort())) {
    throw new Error('HARNESS_NATIVE_ORIGIN_BINDING_MISMATCH');
  }
  return deepFreeze({
    enforcement: 'origin-pinned-process-boundary',
    mechanism: asNonEmptyString(entry.mechanism, `${label}.mechanism`),
    pinnedOrigins,
  });
}

function parseNativeFilesystem(
  value: unknown,
  label: string,
): NativeRuntimeEvidence['invocations'][number]['filesystem'] {
  const entry = asRecord(value, label);
  assertExactKeys(entry, [
    'enforcement', 'mechanism', 'workspaceRootDigest', 'mountManifestDigest',
    'outputChannelDigest', 'hostFileConfidentiality', 'emptyPrivateHome', 'hostRootMounted',
    'gitMetadataMasked',
  ], label);
  if (entry.enforcement !== 'os-filesystem-namespace'
    || entry.hostFileConfidentiality !== true
    || entry.emptyPrivateHome !== true
    || entry.hostRootMounted !== false
    || entry.gitMetadataMasked !== true) {
    throw new Error('HARNESS_NATIVE_FILESYSTEM_BOUNDARY_REQUIRED');
  }
  return deepFreeze({
    enforcement: 'os-filesystem-namespace',
    mechanism: asNonEmptyString(entry.mechanism, `${label}.mechanism`),
    workspaceRootDigest: parseDigest(entry.workspaceRootDigest, `${label}.workspaceRootDigest`),
    mountManifestDigest: parseDigest(entry.mountManifestDigest, `${label}.mountManifestDigest`),
    outputChannelDigest: parseDigest(entry.outputChannelDigest, `${label}.outputChannelDigest`),
    hostFileConfidentiality: true,
    emptyPrivateHome: true,
    hostRootMounted: false,
    gitMetadataMasked: true,
  });
}

function parseNativeHost(value: unknown, label: string): NativeHost {
  if (value !== 'codex' && value !== 'claude-code') throw new TypeError(`${label} is invalid`);
  return value;
}

function parseDigest(value: unknown, label: string): string {
  const digest = asNonEmptyString(value, label);
  if (!SHA256_PATTERN.test(digest) || digest === '0'.repeat(64)) {
    throw new TypeError(`${label} must be a non-genesis SHA-256 digest`);
  }
  return digest;
}

function parseGitTree(value: unknown, label = 'Agentic-QE evidence'): string {
  const tree = asNonEmptyString(value, `${label}.candidateTree`);
  if (!/^[a-f0-9]{40,64}$/.test(tree)) throw new TypeError('Agentic-QE evidence candidate identity is invalid');
  return tree;
}

function parseTimestamp(value: unknown, label: string): string {
  const timestamp = asNonEmptyString(value, label);
  const date = new Date(timestamp);
  if (!Number.isFinite(date.valueOf()) || date.toISOString() !== timestamp) {
    throw new TypeError(`${label} must be a canonical ISO timestamp`);
  }
  return timestamp;
}

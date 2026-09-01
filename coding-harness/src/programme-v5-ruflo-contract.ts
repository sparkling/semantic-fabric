// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  SHA256_PATTERN,
  asInteger,
  asNonEmptyString,
  asRecord,
  asUniqueStrings,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';

export const PROGRAMME_V5_RUFLO_MCP_IDENTITY = Object.freeze({
  transport: 'stdio' as const,
  protocolVersion: '2024-11-05' as const,
  serverName: 'ruflo' as const,
  serverVersion: '3.0.0' as const,
});

export const PROGRAMME_V5_RUFLO_CLI_IDENTITY = Object.freeze({
  packageName: '@claude-flow/cli' as const,
  packageVersion: '3.38.20' as const,
  // Frozen schema-V2 identity label; actual source identity is the sealed aggregate below.
  entryPath: '/home/claude/.npm-global/lib/node_modules/@claude-flow/cli/bin/mcp-server.js' as const,
  entryDigest: 'b2baccea433793c53ec1f7638134ea76ee7516c8e4808dda92fc6f6948ee43fa' as const,
  nodePath: '/usr/bin/node' as const,
  nodeDigest: '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc' as const,
  bwrapPath: '/usr/bin/bwrap' as const,
  bwrapDigest: '52231e1caf55bcbc667b269f49c63599a6f7db4767ae6a039580d0ff853db712' as const,
  packageSourceDigest: 'f574f094c233d47e9cb450dcb4f7a31aa01a53056063e62c1174560b60814b02' as const,
  packageSourceFileCount: 1_552 as const,
  packageSourceBytes: 14_036_904 as const,
  executionIsolation: 'immutable-private-node-package-closure-bwrap-v1' as const,
  dependencyClosure: 'exact-static-closure-immutable-private-v1' as const,
});

export const PROGRAMME_V5_RUFLO_NODE_IDENTITY = Object.freeze({
  path: PROGRAMME_V5_RUFLO_CLI_IDENTITY.nodePath,
  digest: PROGRAMME_V5_RUFLO_CLI_IDENTITY.nodeDigest,
});
export const PROGRAMME_V5_RUFLO_BWRAP_IDENTITY = Object.freeze({
  path: PROGRAMME_V5_RUFLO_CLI_IDENTITY.bwrapPath,
  digest: PROGRAMME_V5_RUFLO_CLI_IDENTITY.bwrapDigest,
});
export const PROGRAMME_V5_RUFLO_CAPTURE_WINDOW_MS = 60_000;

export interface ProgrammeV5RufloTaskStatus {
  taskId: string;
  type: string;
  description: string;
  status: 'in_progress';
  progress: number;
  priority: 'low' | 'normal' | 'high' | 'critical';
  assignedTo: string[];
  tags: string[];
  createdAt: string;
  startedAt: string;
  completedAt: null;
  result: null;
}

export interface ProgrammeV5RufloSwarmStatus {
  swarmId: string;
  status: 'running';
  topology: 'hierarchical';
  maxAgents: number;
  agentCount: number;
  taskCount: number;
  config: {
    topology: 'hierarchical';
    maxAgents: number;
    strategy: 'specialized';
    communicationProtocol: 'message-bus';
    autoScaling: boolean;
    consensusMechanism: 'raft';
  };
  createdAt: string;
  updatedAt: string;
}

export interface ProgrammeV5RufloTaskStatusRequest {
  jsonrpc: '2.0';
  id: 2;
  method: 'tools/call';
  params: { name: 'task_status'; arguments: { taskId: string } };
}

export interface ProgrammeV5RufloSwarmStatusRequest {
  jsonrpc: '2.0';
  id: 3;
  method: 'tools/call';
  params: { name: 'swarm_status'; arguments: { swarmId: string } };
}

export interface ProgrammeV5RufloEvidence {
  schemaVersion: 2;
  source: 'ruflo-coordination-ledger';
  taskId: string;
  runId: string;
  swarmId: string;
  coordinationTaskId: string;
  hookIds: string[];
  traceIds: string[];
  routeSnapshotDigest: string;
  captureNonce: string;
  transactionStartedAt: string;
  captureBindingDigest: string;
  mcp: typeof PROGRAMME_V5_RUFLO_MCP_IDENTITY;
  cli: typeof PROGRAMME_V5_RUFLO_CLI_IDENTITY;
  taskStatusRequest: ProgrammeV5RufloTaskStatusRequest;
  swarmStatusRequest: ProgrammeV5RufloSwarmStatusRequest;
  taskStatus: ProgrammeV5RufloTaskStatus;
  taskStatusDigest: string;
  swarmStatus: ProgrammeV5RufloSwarmStatus;
  swarmStatusDigest: string;
  providerVariablesStripped: true;
  authoritative: false;
  capturedAt: string;
}

const EVIDENCE_KEYS = [
  'schemaVersion', 'source', 'taskId', 'runId', 'swarmId', 'coordinationTaskId',
  'hookIds', 'traceIds', 'routeSnapshotDigest', 'captureNonce', 'transactionStartedAt',
  'captureBindingDigest', 'mcp', 'cli', 'taskStatusRequest', 'swarmStatusRequest',
  'taskStatus', 'taskStatusDigest', 'swarmStatus', 'swarmStatusDigest',
  'providerVariablesStripped', 'authoritative', 'capturedAt',
] as const;
const TASK_STATUS_KEYS = [
  'taskId', 'type', 'description', 'status', 'progress', 'priority', 'assignedTo',
  'tags', 'createdAt', 'startedAt', 'completedAt', 'result',
] as const;
const SWARM_STATUS_KEYS = [
  'swarmId', 'status', 'topology', 'maxAgents', 'agentCount', 'taskCount', 'config',
  'createdAt', 'updatedAt',
] as const;
const SWARM_CONFIG_KEYS = [
  'topology', 'maxAgents', 'strategy', 'communicationProtocol', 'autoScaling',
  'consensusMechanism',
] as const;
const OPAQUE_ID = /^[A-Za-z0-9_-]{8,160}$/;

export function programmeV5RufloRequests(
  coordinationTaskId: string,
  swarmId: string,
): Readonly<{
  taskStatusRequest: ProgrammeV5RufloTaskStatusRequest;
  swarmStatusRequest: ProgrammeV5RufloSwarmStatusRequest;
}> {
  return deepFreeze({
    taskStatusRequest: {
      jsonrpc: '2.0', id: 2, method: 'tools/call',
      params: { name: 'task_status', arguments: { taskId: coordinationTaskId } },
    },
    swarmStatusRequest: {
      jsonrpc: '2.0', id: 3, method: 'tools/call',
      params: { name: 'swarm_status', arguments: { swarmId } },
    },
  });
}

export function programmeV5RufloSnapshotDigest(value: unknown): string {
  return createHash('sha256').update(canonicalJson(value)).digest('hex');
}

export function programmeV5RufloCaptureBindingDigest(input: Readonly<{
  captureNonce: string;
  transactionStartedAt: string;
  taskId: string;
  runId: string;
  routeSnapshotDigest: string;
  swarmId: string;
  coordinationTaskId: string;
}>): string {
  return programmeV5RufloSnapshotDigest({
    schemaVersion: 1,
    source: 'programme-v5-ruflo-capture-binding',
    captureNonce: digest(input.captureNonce, 'programme v5 Ruflo capture nonce'),
    transactionStartedAt: canonicalTimestamp(
      input.transactionStartedAt, 'programme v5 Ruflo transactionStartedAt',
    ),
    taskId: opaqueId(input.taskId, 'programme v5 Ruflo capture taskId'),
    runId: opaqueId(input.runId, 'programme v5 Ruflo capture runId'),
    routeSnapshotDigest: digest(input.routeSnapshotDigest, 'programme v5 Ruflo route digest'),
    swarmId: opaqueId(input.swarmId, 'programme v5 Ruflo capture swarmId'),
    coordinationTaskId: opaqueId(
      input.coordinationTaskId, 'programme v5 Ruflo capture coordinationTaskId',
    ),
  });
}

export function parseProgrammeV5RufloTaskStatus(
  value: unknown,
  coordinationTaskId: string,
): ProgrammeV5RufloTaskStatus {
  const input = exactRecord(value, TASK_STATUS_KEYS, 'programme v5 Ruflo task_status');
  const taskId = opaqueId(input.taskId, 'programme v5 Ruflo task_status.taskId');
  if (taskId !== coordinationTaskId) throw new Error('HARNESS_RUFLO_TASK_STATUS_ID_MISMATCH');
  if (input.status !== 'in_progress' || input.completedAt !== null || input.result !== null) {
    throw new Error('HARNESS_RUFLO_TASK_STATUS_INVALID');
  }
  const progress = boundedInteger(input.progress, 0, 99, 'programme v5 Ruflo task_status.progress');
  const priority = input.priority;
  if (!['low', 'normal', 'high', 'critical'].includes(priority as string)) {
    throw new TypeError('programme v5 Ruflo task_status.priority is invalid');
  }
  const createdAt = canonicalTimestamp(input.createdAt, 'programme v5 Ruflo task_status.createdAt');
  const startedAt = canonicalTimestamp(input.startedAt, 'programme v5 Ruflo task_status.startedAt');
  if (Date.parse(startedAt) < Date.parse(createdAt)) {
    throw new Error('HARNESS_RUFLO_TASK_STATUS_TIME_INVALID');
  }
  return deepFreeze({
    taskId,
    type: boundedString(input.type, 'programme v5 Ruflo task_status.type', 128),
    description: boundedString(input.description, 'programme v5 Ruflo task_status.description', 16_384),
    status: 'in_progress',
    progress,
    priority: priority as ProgrammeV5RufloTaskStatus['priority'],
    assignedTo: boundedStrings(input.assignedTo, 'programme v5 Ruflo task_status.assignedTo'),
    tags: boundedStrings(input.tags, 'programme v5 Ruflo task_status.tags'),
    createdAt,
    startedAt,
    completedAt: null,
    result: null,
  });
}

export function parseProgrammeV5RufloSwarmStatus(
  value: unknown,
  swarmIdBinding: string,
): ProgrammeV5RufloSwarmStatus {
  const input = exactRecord(value, SWARM_STATUS_KEYS, 'programme v5 Ruflo swarm_status');
  const swarmId = opaqueId(input.swarmId, 'programme v5 Ruflo swarm_status.swarmId');
  if (swarmId !== swarmIdBinding) throw new Error('HARNESS_RUFLO_SWARM_STATUS_ID_MISMATCH');
  if (input.status !== 'running' || input.topology !== 'hierarchical') {
    throw new Error('HARNESS_RUFLO_SWARM_STATUS_INVALID');
  }
  const maxAgents = boundedInteger(input.maxAgents, 1, 50, 'programme v5 Ruflo swarm_status.maxAgents');
  const agentCount = boundedInteger(input.agentCount, 0, maxAgents, 'programme v5 Ruflo swarm_status.agentCount');
  const taskCount = boundedInteger(input.taskCount, 0, Number.MAX_SAFE_INTEGER,
    'programme v5 Ruflo swarm_status.taskCount');
  const config = parseSwarmConfig(input.config, maxAgents);
  const createdAt = canonicalTimestamp(input.createdAt, 'programme v5 Ruflo swarm_status.createdAt');
  const updatedAt = canonicalTimestamp(input.updatedAt, 'programme v5 Ruflo swarm_status.updatedAt');
  if (Date.parse(updatedAt) < Date.parse(createdAt)) {
    throw new Error('HARNESS_RUFLO_SWARM_STATUS_TIME_INVALID');
  }
  return deepFreeze({
    swarmId, status: 'running', topology: 'hierarchical', maxAgents, agentCount,
    taskCount, config, createdAt, updatedAt,
  });
}

export function parseProgrammeV5RufloEvidence(value: unknown): ProgrammeV5RufloEvidence {
  const input = exactRecord(value, EVIDENCE_KEYS, 'programme v5 Ruflo evidence');
  if (input.schemaVersion !== 2 || input.source !== 'ruflo-coordination-ledger') {
    throw new TypeError('programme v5 Ruflo evidence provenance is invalid');
  }
  if (input.providerVariablesStripped !== true || input.authoritative !== false) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_BOUNDARY_INVALID');
  }
  const taskId = opaqueId(input.taskId, 'programme v5 Ruflo evidence.taskId');
  const runId = opaqueId(input.runId, 'programme v5 Ruflo evidence.runId');
  const swarmId = opaqueId(input.swarmId, 'programme v5 Ruflo evidence.swarmId');
  const coordinationTaskId = opaqueId(
    input.coordinationTaskId, 'programme v5 Ruflo evidence.coordinationTaskId',
  );
  const routeSnapshotDigest = digest(input.routeSnapshotDigest, 'programme v5 Ruflo route digest');
  const captureNonce = digest(input.captureNonce, 'programme v5 Ruflo capture nonce');
  const transactionStartedAt = canonicalTimestamp(
    input.transactionStartedAt, 'programme v5 Ruflo transactionStartedAt',
  );
  const captureBindingDigest = digest(
    input.captureBindingDigest, 'programme v5 Ruflo capture binding digest',
  );
  if (captureBindingDigest !== programmeV5RufloCaptureBindingDigest({
    captureNonce, transactionStartedAt, taskId, runId, routeSnapshotDigest,
    swarmId, coordinationTaskId,
  })) throw new Error('HARNESS_PROGRAMME_V5_RUFLO_CAPTURE_BINDING_MISMATCH');
  const mcp = parseMcpIdentity(input.mcp);
  const cli = parseCliIdentity(input.cli);
  const requests = parseRequests(input.taskStatusRequest, input.swarmStatusRequest,
    coordinationTaskId, swarmId);
  const taskStatus = parseProgrammeV5RufloTaskStatus(input.taskStatus, coordinationTaskId);
  const swarmStatus = parseProgrammeV5RufloSwarmStatus(input.swarmStatus, swarmId);
  const taskStatusDigest = digest(input.taskStatusDigest, 'programme v5 Ruflo task status digest');
  const swarmStatusDigest = digest(input.swarmStatusDigest, 'programme v5 Ruflo swarm status digest');
  if (taskStatusDigest !== programmeV5RufloSnapshotDigest(taskStatus)
    || swarmStatusDigest !== programmeV5RufloSnapshotDigest(swarmStatus)) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_STATUS_DIGEST_MISMATCH');
  }
  const capturedAt = canonicalTimestamp(input.capturedAt, 'programme v5 Ruflo evidence.capturedAt');
  const captureTime = Date.parse(capturedAt);
  const transactionTime = Date.parse(transactionStartedAt);
  if (captureTime < transactionTime
    || captureTime - transactionTime > PROGRAMME_V5_RUFLO_CAPTURE_WINDOW_MS
    || Date.parse(taskStatus.startedAt) > captureTime
    || Date.parse(swarmStatus.updatedAt) > captureTime) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_FRESHNESS_INVALID');
  }
  return deepFreeze({
    schemaVersion: 2,
    source: 'ruflo-coordination-ledger',
    taskId,
    runId,
    swarmId,
    coordinationTaskId,
    hookIds: boundedStrings(input.hookIds, 'programme v5 Ruflo evidence.hookIds'),
    traceIds: boundedStrings(input.traceIds, 'programme v5 Ruflo evidence.traceIds'),
    routeSnapshotDigest,
    captureNonce,
    transactionStartedAt,
    captureBindingDigest,
    mcp,
    cli,
    ...requests,
    taskStatus,
    taskStatusDigest,
    swarmStatus,
    swarmStatusDigest,
    providerVariablesStripped: true,
    authoritative: false,
    capturedAt,
  });
}

export function validProgrammeV5RufloBinding(
  evidence: unknown,
  expected: Readonly<{
    taskId: string;
    runId: string;
    routeSnapshotDigest: string;
    swarmId: string | null;
    coordinationTaskId: string | null;
    hookIds: readonly string[];
    traceIds: readonly string[];
    transactionStartedAt: string;
    receiptIssuedAt: string;
  }>,
): boolean {
  try {
    const parsed = parseProgrammeV5RufloEvidence(evidence);
    const receiptIssuedAt = canonicalTimestamp(
      expected.receiptIssuedAt, 'programme v5 Ruflo expected receiptIssuedAt',
    );
    return parsed.taskId === expected.taskId
      && parsed.runId === expected.runId
      && parsed.routeSnapshotDigest === expected.routeSnapshotDigest
      && parsed.swarmId === expected.swarmId
      && parsed.coordinationTaskId === expected.coordinationTaskId
      && sameStrings(parsed.hookIds, expected.hookIds)
      && sameStrings(parsed.traceIds, expected.traceIds)
      && parsed.transactionStartedAt === expected.transactionStartedAt
      && Date.parse(parsed.capturedAt) <= Date.parse(receiptIssuedAt);
  } catch {
    return false;
  }
}

function parseSwarmConfig(value: unknown, maxAgents: number): ProgrammeV5RufloSwarmStatus['config'] {
  const input = exactRecord(value, SWARM_CONFIG_KEYS, 'programme v5 Ruflo swarm_status.config');
  if (input.topology !== 'hierarchical' || input.maxAgents !== maxAgents
    || input.strategy !== 'specialized' || input.communicationProtocol !== 'message-bus'
    || typeof input.autoScaling !== 'boolean' || input.consensusMechanism !== 'raft') {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_SWARM_CONFIG_INVALID');
  }
  return deepFreeze({
    topology: 'hierarchical', maxAgents, strategy: 'specialized',
    communicationProtocol: 'message-bus', autoScaling: input.autoScaling,
    consensusMechanism: 'raft',
  });
}

function parseMcpIdentity(value: unknown): typeof PROGRAMME_V5_RUFLO_MCP_IDENTITY {
  const input = exactRecord(value, [
    'transport', 'protocolVersion', 'serverName', 'serverVersion',
  ], 'programme v5 Ruflo MCP identity');
  if (!sameRecord(input, PROGRAMME_V5_RUFLO_MCP_IDENTITY)) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_IDENTITY_MISMATCH');
  }
  return PROGRAMME_V5_RUFLO_MCP_IDENTITY;
}

function parseCliIdentity(value: unknown): typeof PROGRAMME_V5_RUFLO_CLI_IDENTITY {
  const input = exactRecord(value, [
    'packageName', 'packageVersion', 'entryPath', 'entryDigest', 'nodePath', 'nodeDigest',
    'bwrapPath', 'bwrapDigest',
    'packageSourceDigest', 'packageSourceFileCount', 'packageSourceBytes',
    'executionIsolation', 'dependencyClosure',
  ], 'programme v5 Ruflo CLI identity');
  if (!sameRecord(input, PROGRAMME_V5_RUFLO_CLI_IDENTITY)) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_CLI_IDENTITY_MISMATCH');
  }
  return PROGRAMME_V5_RUFLO_CLI_IDENTITY;
}

function parseRequests(
  taskValue: unknown,
  swarmValue: unknown,
  coordinationTaskId: string,
  swarmId: string,
): ReturnType<typeof programmeV5RufloRequests> {
  const expected = programmeV5RufloRequests(coordinationTaskId, swarmId);
  parseRequest(taskValue, expected.taskStatusRequest, 'programme v5 Ruflo task_status request');
  parseRequest(swarmValue, expected.swarmStatusRequest, 'programme v5 Ruflo swarm_status request');
  return expected;
}

function parseRequest(value: unknown, expected: object, label: string): void {
  const input = exactRecord(value, ['jsonrpc', 'id', 'method', 'params'], label);
  const expectedRecord = expected as Record<string, unknown>;
  const params = exactRecord(input.params, ['name', 'arguments'], `${label}.params`);
  const expectedParams = expectedRecord.params as Record<string, unknown>;
  const argumentName = params.name === 'task_status' ? 'taskId' : 'swarmId';
  const args = exactRecord(params.arguments, [argumentName], `${label}.params.arguments`);
  const expectedArgs = expectedParams.arguments as Record<string, unknown>;
  if (input.jsonrpc !== expectedRecord.jsonrpc || input.id !== expectedRecord.id
    || input.method !== expectedRecord.method || params.name !== expectedParams.name
    || args[argumentName] !== expectedArgs[argumentName]) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_REQUEST_BINDING_MISMATCH');
  }
}

function exactRecord(value: unknown, keys: readonly string[], label: string): Record<string, unknown> {
  const input = asRecord(value, label);
  const prototype = Object.getPrototypeOf(input);
  const ownKeys = Reflect.ownKeys(input);
  if ((prototype !== Object.prototype && prototype !== null)
    || ownKeys.some((key) => typeof key !== 'string'
      || !Object.prototype.propertyIsEnumerable.call(input, key))) {
    throw new TypeError(`${label} has invalid keys`);
  }
  assertExactKeys(input, keys, label);
  if (ownKeys.length !== keys.length) throw new TypeError(`${label} has invalid keys`);
  return input;
}

function opaqueId(value: unknown, label: string): string {
  const id = asNonEmptyString(value, label);
  if (!OPAQUE_ID.test(id)) throw new TypeError(`${label} is invalid`);
  return id;
}

function boundedString(value: unknown, label: string, maximum = 256): string {
  const text = asNonEmptyString(value, label);
  if (Buffer.byteLength(text) > maximum) throw new TypeError(`${label} is too large`);
  return text;
}

function boundedStrings(value: unknown, label: string): string[] {
  const strings = asUniqueStrings(value, label, true);
  if (strings.length > 256) throw new TypeError(`${label} has too many entries`);
  return strings.map((entry, index) => boundedString(entry, `${label}[${index}]`));
}

function boundedInteger(value: unknown, minimum: number, maximum: number, label: string): number {
  const parsed = asInteger(value, label, minimum);
  if (parsed > maximum) throw new TypeError(`${label} must be <= ${maximum}`);
  return parsed;
}

function canonicalTimestamp(value: unknown, label: string): string {
  const timestamp = asNonEmptyString(value, label);
  const date = new Date(timestamp);
  if (!Number.isFinite(date.valueOf()) || date.toISOString() !== timestamp) {
    throw new TypeError(`${label} must be a canonical ISO timestamp`);
  }
  return timestamp;
}

function digest(value: unknown, label: string): string {
  const parsed = asNonEmptyString(value, label);
  if (!SHA256_PATTERN.test(parsed) || parsed === '0'.repeat(64)) {
    throw new TypeError(`${label} must be a non-genesis SHA-256 digest`);
  }
  return parsed;
}

function canonicalJson(value: unknown): string {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') {
    return JSON.stringify(value);
  }
  if (typeof value === 'number' && Number.isFinite(value)) return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  const input = asRecord(value, 'programme v5 Ruflo canonical snapshot');
  return `{${Object.keys(input).sort().map((key) =>
    `${JSON.stringify(key)}:${canonicalJson(input[key])}`).join(',')}}`;
}

function sameRecord(
  actual: Readonly<Record<string, unknown>>,
  expected: Readonly<Record<string, unknown>>,
): boolean {
  return Object.keys(expected).every((key) => actual[key] === expected[key]);
}

function sameStrings(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

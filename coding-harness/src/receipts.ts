// SPDX-License-Identifier: MIT

import { canonical, hash } from '@metaharness/harness';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asInteger,
  asNonEmptyString,
  asRecord,
  asUniqueStrings,
  assertExactKeys,
  assertStructuredText,
  deepFreeze,
  normalizeWorkspacePath,
} from './contracts.js';

export type ReceiptStatus = 'pass' | 'fail' | 'gated' | 'cancelled';

export interface GitIdentity {
  commit: string;
  tree: string;
}

export interface HostEvidence {
  host: 'codex' | 'claude-code';
  model: string;
  role: string;
  clientVersion: string;
  authClass: 'native-openai-subscription' | 'native-anthropic-subscription';
  subscriptionCostUsd: 0;
}

export interface CommandEvidence {
  stage: 'red-baseline' | 'build' | 'mutation';
  attempt: number;
  candidateTree: string;
  tool: string;
  executable: string;
  argv: string[];
  cwd: string;
  exitCode: number | null;
  signal: string | null;
  durationMs: number;
  stdoutDigest: string;
  stderrDigest: string;
  timedOut: boolean;
  cancelled: boolean;
  outputLimitExceeded: boolean;
  spawnErrorDigest: string | null;
}

export interface ReceiptDraft {
  schemaVersion: 2;
  runId: string;
  taskId: string;
  step: string;
  status: ReceiptStatus;
  authority: typeof DEVELOPMENT_AUTHORITY;
  issuedAt: string;
  identities: {
    controller: GitIdentity;
    baseline: GitIdentity;
    evaluator: GitIdentity;
    candidate: GitIdentity;
  };
  protectedInputs: Record<string, string>;
  route: {
    snapshotDigest: string;
    frozenAt: string;
    routerVersion: string;
  };
  hosts: HostEvidence[];
  admittedPaths: string[];
  patchDigest: string | null;
  patchDigests: string[];
  toolVersions: Record<string, string>;
  commands: CommandEvidence[];
  artifactDigests: Record<string, string>;
  verifierDigests: Record<string, string>;
  critiqueDigests: string[];
  reviewDigests: string[];
  recovery: {
    retryCount: number;
    breakerState: 'closed' | 'open' | 'half-open';
    cancelled: boolean;
    repairCount: number;
  };
  coordination: {
    swarmId: string | null;
    taskId: string | null;
    hookIds: string[];
    traceIds: string[];
    agenticQeEvidenceDigests: string[];
    nativeEvidenceDigests: string[];
    nativeRuntimeEvidenceDigest: string | null;
  };
}

export interface Receipt extends ReceiptDraft {
  sequence: number;
  previousDigest: string;
  digest: string;
}

export type ChainVerification = { ok: true } | { ok: false; brokenAt: number; reason: string };

const GENESIS_DIGEST = '0'.repeat(64);
const DRAFT_KEYS = [
  'schemaVersion', 'runId', 'taskId', 'step', 'status', 'authority', 'issuedAt', 'identities',
  'protectedInputs', 'route', 'hosts', 'admittedPaths', 'patchDigest', 'patchDigests', 'toolVersions', 'commands',
  'artifactDigests', 'verifierDigests', 'critiqueDigests', 'reviewDigests', 'recovery', 'coordination',
] as const;
const RECEIPT_KEYS = [...DRAFT_KEYS, 'sequence', 'previousDigest', 'digest'] as const;

export class ReceiptChain {
  readonly #receipts: Receipt[] = [];

  append(value: unknown): Receipt {
    const draft = parseReceiptDraft(value);
    const sequence = this.#receipts.length;
    const previousDigest = sequence === 0 ? GENESIS_DIGEST : this.#receipts[sequence - 1].digest;
    const body = { ...draft, sequence, previousDigest };
    const receipt = deepFreeze({ ...body, digest: digestValue(body) });
    this.#receipts.push(receipt);
    return receipt;
  }

  entries(): readonly Receipt[] {
    return [...this.#receipts];
  }

  get length(): number {
    return this.#receipts.length;
  }

  get headDigest(): string {
    return this.#receipts.at(-1)?.digest ?? GENESIS_DIGEST;
  }

  verify(): ChainVerification {
    let previousDigest = GENESIS_DIGEST;
    for (let index = 0; index < this.#receipts.length; index += 1) {
      const receipt = this.#receipts[index];
      if (receipt.sequence !== index) return { ok: false, brokenAt: index, reason: 'sequence is not contiguous' };
      if (receipt.previousDigest !== previousDigest) {
        return { ok: false, brokenAt: index, reason: 'previousDigest does not chain' };
      }
      const { digest, ...body } = receipt;
      if (digestValue(body) !== digest) return { ok: false, brokenAt: index, reason: 'digest does not match body' };
      previousDigest = digest;
    }
    return { ok: true };
  }

  export(): string {
    return canonical({ schemaVersion: 2, receipts: this.#receipts });
  }

  static import(serialized: string): ReceiptChain {
    let value: unknown;
    try {
      value = JSON.parse(serialized);
    } catch {
      throw new TypeError('receipt chain is not valid JSON');
    }
    const input = asRecord(value, 'receipt chain');
    assertExactKeys(input, ['schemaVersion', 'receipts'], 'receipt chain');
    if (input.schemaVersion !== 2 || !Array.isArray(input.receipts)) {
      throw new TypeError('receipt chain must use schemaVersion 2 and contain receipts');
    }
    const chain = new ReceiptChain();
    for (const entry of input.receipts) chain.#receipts.push(parseReceipt(entry));
    const verification = chain.verify();
    if (!verification.ok) {
      throw new Error(`receipt chain broken at ${verification.brokenAt}: ${verification.reason}`);
    }
    return chain;
  }
}

export function digestValue(value: unknown): string {
  return hash(value);
}

export function parseReceiptDraft(value: unknown): ReceiptDraft {
  const input = asRecord(value, 'receipt');
  assertExactKeys(input, DRAFT_KEYS, 'receipt');
  if (input.schemaVersion !== 2) throw new TypeError('receipt.schemaVersion must be 2');
  if (input.authority !== DEVELOPMENT_AUTHORITY) throw new TypeError('receipt cannot grant promotion authority');
  const status = parseStatus(input.status);
  const issuedAt = parseIsoTimestamp(input.issuedAt, 'receipt.issuedAt');

  const identitiesInput = asRecord(input.identities, 'receipt.identities');
  assertExactKeys(
    identitiesInput,
    ['controller', 'baseline', 'evaluator', 'candidate'],
    'receipt.identities',
  );
  const identities = {
    controller: parseGitIdentity(identitiesInput.controller, 'receipt.identities.controller'),
    baseline: parseGitIdentity(identitiesInput.baseline, 'receipt.identities.baseline'),
    evaluator: parseGitIdentity(identitiesInput.evaluator, 'receipt.identities.evaluator'),
    candidate: parseGitIdentity(identitiesInput.candidate, 'receipt.identities.candidate'),
  };

  const routeInput = asRecord(input.route, 'receipt.route');
  assertExactKeys(routeInput, ['snapshotDigest', 'frozenAt', 'routerVersion'], 'receipt.route');
  const route = {
    snapshotDigest: parseDigest(routeInput.snapshotDigest, 'receipt.route.snapshotDigest'),
    frozenAt: parseIsoTimestamp(routeInput.frozenAt, 'receipt.route.frozenAt'),
    routerVersion: asNonEmptyString(routeInput.routerVersion, 'receipt.route.routerVersion'),
  };

  if (!Array.isArray(input.hosts)) throw new TypeError('receipt.hosts must be an array');
  if (!Array.isArray(input.commands)) throw new TypeError('receipt.commands must be an array');
  const recovery = parseRecovery(input.recovery);
  const coordination = parseCoordination(input.coordination);

  const draft: ReceiptDraft = deepFreeze({
    schemaVersion: 2,
    runId: asNonEmptyString(input.runId, 'receipt.runId'),
    taskId: asNonEmptyString(input.taskId, 'receipt.taskId'),
    step: asNonEmptyString(input.step, 'receipt.step'),
    status,
    authority: DEVELOPMENT_AUTHORITY,
    issuedAt,
    identities,
    protectedInputs: parseDigestRecord(input.protectedInputs, 'receipt.protectedInputs', true),
    route,
    hosts: input.hosts.map((host, index) => parseHostEvidence(host, index)),
    admittedPaths: asUniqueStrings(input.admittedPaths, 'receipt.admittedPaths', true)
      .map((path, index) => normalizeWorkspacePath(path, `receipt.admittedPaths[${index}]`)),
    patchDigest: input.patchDigest === null ? null : parseDigest(input.patchDigest, 'receipt.patchDigest'),
    patchDigests: parseDigestArray(input.patchDigests, 'receipt.patchDigests'),
    toolVersions: parseStringRecord(input.toolVersions, 'receipt.toolVersions'),
    commands: input.commands.map((command, index) => parseCommandEvidence(command, index)),
    artifactDigests: parseDigestRecord(input.artifactDigests, 'receipt.artifactDigests', true),
    verifierDigests: parseDigestRecord(input.verifierDigests, 'receipt.verifierDigests'),
    critiqueDigests: parseDigestArray(input.critiqueDigests, 'receipt.critiqueDigests'),
    reviewDigests: parseDigestArray(input.reviewDigests, 'receipt.reviewDigests'),
    recovery,
    coordination,
  });
  assertPassReceipt(draft);
  return draft;
}

function parseReceipt(value: unknown): Receipt {
  const input = asRecord(value, 'receipt');
  assertExactKeys(input, RECEIPT_KEYS, 'receipt');
  const draftInput = Object.fromEntries(DRAFT_KEYS.map((key) => [key, input[key]]));
  const draft = parseReceiptDraft(draftInput);
  return deepFreeze({
    ...draft,
    sequence: asInteger(input.sequence, 'receipt.sequence'),
    previousDigest: parseDigest(input.previousDigest, 'receipt.previousDigest', true),
    digest: parseDigest(input.digest, 'receipt.digest'),
  });
}

function parseGitIdentity(value: unknown, label: string): GitIdentity {
  const input = asRecord(value, label);
  assertExactKeys(input, ['commit', 'tree'], label);
  const parseGitHash = (entry: unknown, field: string) => {
    const digest = asNonEmptyString(entry, field);
    if (!/^[a-f0-9]{40,64}$/.test(digest)) throw new TypeError(`${field} is not a Git object ID`);
    return digest;
  };
  return {
    commit: parseGitHash(input.commit, `${label}.commit`),
    tree: parseGitHash(input.tree, `${label}.tree`),
  };
}

function parseHostEvidence(value: unknown, index: number): HostEvidence {
  const label = `receipt.hosts[${index}]`;
  const input = asRecord(value, label);
  assertExactKeys(input, ['host', 'model', 'role', 'clientVersion', 'authClass', 'subscriptionCostUsd'], label);
  if (input.host !== 'codex' && input.host !== 'claude-code') throw new TypeError(`${label}.host is invalid`);
  const expectedAuth = input.host === 'codex' ? 'native-openai-subscription' : 'native-anthropic-subscription';
  if (input.authClass !== expectedAuth) throw new TypeError(`${label}.authClass does not match the native host`);
  if (input.subscriptionCostUsd !== 0) throw new TypeError(`${label}.subscriptionCostUsd must be 0`);
  const model = asNonEmptyString(input.model, `${label}.model`);
  if (/openrouter|requesty/i.test(model)) throw new TypeError(`${label}.model names an indirect gateway`);
  return {
    host: input.host,
    model,
    role: asNonEmptyString(input.role, `${label}.role`),
    clientVersion: asNonEmptyString(input.clientVersion, `${label}.clientVersion`),
    authClass: expectedAuth,
    subscriptionCostUsd: 0,
  };
}

function parseCommandEvidence(value: unknown, index: number): CommandEvidence {
  const label = `receipt.commands[${index}]`;
  const input = asRecord(value, label);
  assertExactKeys(input, [
    'stage', 'attempt', 'candidateTree', 'tool', 'executable', 'argv', 'cwd', 'exitCode', 'signal',
    'durationMs', 'stdoutDigest', 'stderrDigest', 'timedOut', 'cancelled',
    'outputLimitExceeded', 'spawnErrorDigest',
  ], label);
  if (!Array.isArray(input.argv)) throw new TypeError(`${label}.argv must be an array`);
  if (input.stage !== 'red-baseline' && input.stage !== 'build' && input.stage !== 'mutation') {
    throw new TypeError(`${label}.stage is invalid`);
  }
  const exitCode = input.exitCode === null ? null : asInteger(input.exitCode, `${label}.exitCode`);
  const signal = input.signal === null ? null : asNonEmptyString(input.signal, `${label}.signal`);
  return {
    stage: input.stage,
    attempt: asInteger(input.attempt, `${label}.attempt`),
    candidateTree: parseGitObject(input.candidateTree, `${label}.candidateTree`),
    tool: assertStructuredText(input.tool, `${label}.tool`),
    executable: assertStructuredText(input.executable, `${label}.executable`),
    argv: input.argv.map((arg, argIndex) => assertStructuredText(arg, `${label}.argv[${argIndex}]`)),
    cwd: normalizeWorkspacePath(input.cwd, `${label}.cwd`, true),
    exitCode,
    signal,
    durationMs: asInteger(input.durationMs, `${label}.durationMs`),
    stdoutDigest: parseDigest(input.stdoutDigest, `${label}.stdoutDigest`),
    stderrDigest: parseDigest(input.stderrDigest, `${label}.stderrDigest`),
    timedOut: parseBoolean(input.timedOut, `${label}.timedOut`),
    cancelled: parseBoolean(input.cancelled, `${label}.cancelled`),
    outputLimitExceeded: parseBoolean(input.outputLimitExceeded, `${label}.outputLimitExceeded`),
    spawnErrorDigest: input.spawnErrorDigest === null
      ? null
      : parseDigest(input.spawnErrorDigest, `${label}.spawnErrorDigest`),
  };
}

function parseRecovery(value: unknown): ReceiptDraft['recovery'] {
  const input = asRecord(value, 'receipt.recovery');
  assertExactKeys(input, ['retryCount', 'breakerState', 'cancelled', 'repairCount'], 'receipt.recovery');
  if (input.breakerState !== 'closed' && input.breakerState !== 'open' && input.breakerState !== 'half-open') {
    throw new TypeError('receipt.recovery.breakerState is invalid');
  }
  return {
    retryCount: asInteger(input.retryCount, 'receipt.recovery.retryCount'),
    breakerState: input.breakerState,
    cancelled: parseBoolean(input.cancelled, 'receipt.recovery.cancelled'),
    repairCount: asInteger(input.repairCount, 'receipt.recovery.repairCount'),
  };
}

function parseCoordination(value: unknown): ReceiptDraft['coordination'] {
  const input = asRecord(value, 'receipt.coordination');
  assertExactKeys(
    input,
    [
      'swarmId', 'taskId', 'hookIds', 'traceIds', 'agenticQeEvidenceDigests',
      'nativeEvidenceDigests', 'nativeRuntimeEvidenceDigest',
    ],
    'receipt.coordination',
  );
  const nullable = (entry: unknown, label: string) => entry === null ? null : asNonEmptyString(entry, label);
  return {
    swarmId: nullable(input.swarmId, 'receipt.coordination.swarmId'),
    taskId: nullable(input.taskId, 'receipt.coordination.taskId'),
    hookIds: asUniqueStrings(input.hookIds, 'receipt.coordination.hookIds', true),
    traceIds: asUniqueStrings(input.traceIds, 'receipt.coordination.traceIds', true),
    agenticQeEvidenceDigests: parseDigestArray(
      input.agenticQeEvidenceDigests,
      'receipt.coordination.agenticQeEvidenceDigests',
    ),
    nativeEvidenceDigests: parseDigestArray(
      input.nativeEvidenceDigests,
      'receipt.coordination.nativeEvidenceDigests',
    ),
    nativeRuntimeEvidenceDigest: input.nativeRuntimeEvidenceDigest === null
      ? null
      : parseDigest(
        input.nativeRuntimeEvidenceDigest,
        'receipt.coordination.nativeRuntimeEvidenceDigest',
      ),
  };
}

function assertPassReceipt(receipt: ReceiptDraft): void {
  if (receipt.status !== 'pass') return;
  const hosts = new Set(receipt.hosts.map(({ host }) => host));
  const stages = new Set(receipt.commands.map(({ stage }) => stage));
  const attempt = receipt.recovery.repairCount;
  const normallyCompleted = (command: CommandEvidence) => command.signal === null
    && !command.timedOut && !command.cancelled && !command.outputLimitExceeded
    && command.spawnErrorDigest === null;
  const red = receipt.commands.filter(({ stage }) => stage === 'red-baseline');
  const build = receipt.commands.filter((command) => command.stage === 'build' && command.attempt === attempt);
  const mutation = receipt.commands.filter((command) => command.stage === 'mutation' && command.attempt === attempt);
  const verifierKeys = [
    `attempt-${attempt}:public`,
    `attempt-${attempt}:independent`,
    `attempt-${attempt}:regression`,
  ];
  if (receipt.step !== 'candidate-transaction'
    || receipt.hosts.length !== 2
    || hosts.size !== 2 || !hosts.has('codex') || !hosts.has('claude-code')
    || receipt.identities.candidate.tree === receipt.identities.evaluator.tree
    || receipt.admittedPaths.length === 0 || receipt.patchDigest === null
    || receipt.patchDigests.length !== receipt.recovery.repairCount + 1
    || receipt.patchDigests.at(-1) !== receipt.patchDigest
    || stages.size !== 3 || red.length === 0 || build.length === 0 || mutation.length === 0
    || red.some((command) => command.attempt !== 0
      || command.candidateTree !== receipt.identities.evaluator.tree
      || command.exitCode !== 101 || !normallyCompleted(command))
    || build.some((command) => command.candidateTree !== receipt.identities.candidate.tree
      || command.exitCode !== 0 || !normallyCompleted(command))
    || mutation.some((command) => command.candidateTree !== receipt.identities.candidate.tree
      || command.exitCode !== 101 || !normallyCompleted(command))
    || Object.keys(receipt.artifactDigests).length === 0
    || Object.keys(receipt.protectedInputs).length === 0
    || Object.keys(receipt.toolVersions).length === 0
    || !verifierKeys.every((key) => key in receipt.verifierDigests)
    || !('red-baseline' in receipt.verifierDigests)
    || !Object.keys(receipt.verifierDigests)
      .some((key) => key.startsWith(`attempt-${attempt}:mutation`))
    || receipt.critiqueDigests.length === 0 || receipt.reviewDigests.length !== 2
    || new Set(receipt.reviewDigests).size !== receipt.reviewDigests.length
    || receipt.recovery.cancelled || receipt.recovery.breakerState !== 'closed'
    || receipt.coordination.agenticQeEvidenceDigests.length === 0
    || receipt.coordination.nativeEvidenceDigests.length < 4
    || new Set(receipt.coordination.nativeEvidenceDigests).size
      !== receipt.coordination.nativeEvidenceDigests.length
    || receipt.coordination.nativeRuntimeEvidenceDigest === null) {
    throw new Error('HARNESS_PASS_RECEIPT_EVIDENCE_INCOMPLETE');
  }
}

function parseGitObject(value: unknown, label: string): string {
  const object = asNonEmptyString(value, label);
  if (!/^[a-f0-9]{40,64}$/.test(object)) throw new TypeError(`${label} is not a Git object ID`);
  return object;
}

function parseDigestRecord(value: unknown, label: string, pathKeys = false): Record<string, string> {
  const input = asRecord(value, label);
  const output: Record<string, string> = {};
  for (const [rawKey, rawDigest] of Object.entries(input)) {
    const key = pathKeys ? normalizeWorkspacePath(rawKey, `${label} key`) : asNonEmptyString(rawKey, `${label} key`);
    output[key] = parseDigest(rawDigest, `${label}.${rawKey}`);
  }
  return output;
}

function parseStringRecord(value: unknown, label: string): Record<string, string> {
  const input = asRecord(value, label);
  return Object.fromEntries(Object.entries(input).map(([key, entry]) => [
    asNonEmptyString(key, `${label} key`),
    asNonEmptyString(entry, `${label}.${key}`),
  ]));
}

function parseDigestArray(value: unknown, label: string): string[] {
  if (!Array.isArray(value)) throw new TypeError(`${label} must be an array`);
  return value.map((digest, index) => parseDigest(digest, `${label}[${index}]`));
}

function parseDigest(value: unknown, label: string, allowGenesis = false): string {
  const digest = asNonEmptyString(value, label);
  if (!SHA256_PATTERN.test(digest) || (!allowGenesis && digest === GENESIS_DIGEST)) {
    throw new TypeError(`${label} must be a non-genesis SHA-256 digest`);
  }
  return digest;
}

function parseIsoTimestamp(value: unknown, label: string): string {
  const timestamp = asNonEmptyString(value, label);
  const parsed = new Date(timestamp);
  if (!Number.isFinite(parsed.valueOf()) || parsed.toISOString() !== timestamp) {
    throw new TypeError(`${label} must be a canonical ISO-8601 timestamp`);
  }
  return timestamp;
}

function parseStatus(value: unknown): ReceiptStatus {
  if (value !== 'pass' && value !== 'fail' && value !== 'gated' && value !== 'cancelled') {
    throw new TypeError('receipt.status is invalid');
  }
  return value;
}

function parseBoolean(value: unknown, label: string): boolean {
  if (typeof value !== 'boolean') throw new TypeError(`${label} must be boolean`);
  return value;
}

// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { parseAcceptanceTask, type AcceptanceTaskV3 } from './acceptance-task.js';
import type { ControllerAttestation } from './controller-attestation.js';
import { HARNESS_MANIFEST_PATH } from './controller-attestation.js';
import {
  CONTROLLER_BUILD_PATH,
  parseControllerBuildManifest,
  type ControllerBuildManifest,
} from './controller-build.js';
import { SECURE_HARNESS_CONFIG } from './config.js';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
  parseHarnessConfig,
  type HarnessConfig,
} from './contracts.js';
import {
  parseHarnessManifest,
  selectAcceptanceTaskPath,
  type HarnessManifest,
} from './manifest.js';
import { digestValue, type GitIdentity } from './receipts.js';
import {
  createProgrammeGateContractV1,
  parseProgrammeGateContractV1,
  type ProgrammeGateContractV1,
} from './programme-gate-contract-v1.js';
import {
  bindProgrammeTaskRuntimeV1,
  programmeBoundTaskDigestV1,
} from './programme-task-runtime-v1.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';
import { resolveTaskEvidencePlanV1, type TaskEvidencePlan } from './task-evidence-plan.js';

export const PROGRAMME_POLICY_V5_ID = 'semantic-fabric-programme-v5-policy-v1' as const;

export interface FrozenProgrammePolicyV1 {
  readonly schemaVersion: 1;
  readonly policyId: typeof PROGRAMME_POLICY_V5_ID;
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly gateContract: ProgrammeGateContractV1;
  readonly harnessConfig: HarnessConfig;
  readonly bootstrap: Readonly<{
    controllerStoreDigest: string;
    nodeDigest: string;
    gitDigest: string;
  }>;
  readonly controller: Readonly<{
    identity: GitIdentity;
    manifestPath: typeof HARNESS_MANIFEST_PATH;
    manifestBlob: string;
    manifestBlobDigest: string;
    taskPath: string;
    taskBlob: string;
    taskBlobDigest: string;
    buildManifestPath: typeof CONTROLLER_BUILD_PATH;
    buildManifestBlob: string;
    buildManifestBlobDigest: string;
    runtimeTreeDigest: string;
    lockfileDigest: string;
    executionDigest: string;
  }>;
  readonly execution: Readonly<{
    evaluator: GitIdentity;
    protectedInputs: Readonly<Record<string, string>>;
    boundTaskDigest: string;
    routeSnapshotBlob: string;
    routeSnapshotBlobDigest: string;
    routeSnapshotDigest: string;
  }>;
  readonly taskEvidencePlanDigest: string;
}

export interface ParsedProgrammePolicyV1 {
  readonly snapshot: FrozenProgrammePolicyV1;
  readonly fingerprint: string;
  readonly config: HarnessConfig;
  readonly manifest: HarnessManifest;
  readonly task: AcceptanceTaskV3;
  readonly boundTask: AcceptanceTaskV3;
  readonly build: ControllerBuildManifest;
  readonly evidencePlan: TaskEvidencePlan;
  readonly routeSnapshot: unknown;
}

export interface ControllerPolicyInputs {
  readonly bootstrap: Readonly<{
    controllerStoreDigest: string;
    nodeDigest: string;
    gitDigest: string;
  }>;
  readonly controller: Pick<ControllerAttestation,
    'identity' | 'manifestPath' | 'manifestBlob' | 'manifestBlobDigest'
    | 'taskPath' | 'taskBlob' | 'taskBlobDigest' | 'buildManifestPath'
    | 'buildManifestBlob' | 'buildManifestBlobDigest' | 'build' | 'executionDigest' | 'task'>;
  readonly execution: Readonly<{
    evaluator: GitIdentity;
    protectedInputs: Readonly<Record<string, string>>;
    routeSnapshot: unknown;
  }>;
  readonly taskEvidencePlanDigest: string;
  readonly maxRepairs: number;
}

export function createFrozenProgrammePolicyV1(
  input: ControllerPolicyInputs,
): FrozenProgrammePolicyV1 {
  assertExactKeys(asRecord(input, 'programme policy v5 creation input'), [
    'bootstrap', 'controller', 'execution', 'taskEvidencePlanDigest', 'maxRepairs',
  ], 'programme policy v5 creation input');
  assertExactKeys(asRecord(input.execution, 'programme policy v5 creation execution'), [
    'evaluator', 'protectedInputs', 'routeSnapshot',
  ], 'programme policy v5 creation execution');
  if (input.controller.task.schemaVersion !== 3) {
    throw new Error('HARNESS_PROGRAMME_POLICY_TASK_SCHEMA_INVALID');
  }
  const routeSnapshotBlob = serializePolicyJson(
    input.execution.routeSnapshot,
    'programme policy v5 route snapshot',
  );
  const snapshot = {
    schemaVersion: 1 as const,
    policyId: PROGRAMME_POLICY_V5_ID,
    authority: DEVELOPMENT_AUTHORITY,
    gateContract: createProgrammeGateContractV1(input.maxRepairs),
    harnessConfig: SECURE_HARNESS_CONFIG,
    bootstrap: input.bootstrap,
    controller: {
      identity: {
        commit: input.controller.identity.commit,
        tree: input.controller.identity.tree,
      },
      manifestPath: input.controller.manifestPath,
      manifestBlob: input.controller.manifestBlob,
      manifestBlobDigest: input.controller.manifestBlobDigest,
      taskPath: input.controller.taskPath,
      taskBlob: input.controller.taskBlob,
      taskBlobDigest: input.controller.taskBlobDigest,
      buildManifestPath: input.controller.buildManifestPath,
      buildManifestBlob: input.controller.buildManifestBlob,
      buildManifestBlobDigest: input.controller.buildManifestBlobDigest,
      runtimeTreeDigest: input.controller.build.runtimeTreeDigest,
      lockfileDigest: input.controller.build.lockfileDigest,
      executionDigest: input.controller.executionDigest,
    },
    execution: {
      evaluator: {
        commit: input.execution.evaluator.commit,
        tree: input.execution.evaluator.tree,
      },
      protectedInputs: input.execution.protectedInputs,
      boundTaskDigest: programmeBoundTaskDigestV1(input.controller.task),
      routeSnapshotBlob,
      routeSnapshotBlobDigest: sha256(routeSnapshotBlob),
      routeSnapshotDigest: digestValue(input.execution.routeSnapshot),
    },
    taskEvidencePlanDigest: input.taskEvidencePlanDigest,
  };
  return canonicalizeFrozenProgrammePolicyV1(snapshot).snapshot;
}

export function verifyFrozenProgrammePolicyV1(
  value: unknown,
  expectedFingerprint: string,
): ParsedProgrammePolicyV1 {
  const parsed = canonicalizeFrozenProgrammePolicyV1(value);
  if (parseDigest(expectedFingerprint, 'programme policy v5 expected fingerprint')
    !== parsed.fingerprint) {
    throw new Error('HARNESS_PROGRAMME_POLICY_FINGERPRINT_MISMATCH');
  }
  return parsed;
}

function canonicalizeFrozenProgrammePolicyV1(value: unknown): ParsedProgrammePolicyV1 {
  const input = asRecord(value, 'programme policy v5');
  assertExactKeys(input, [
    'schemaVersion', 'policyId', 'authority', 'gateContract', 'harnessConfig',
    'bootstrap', 'controller', 'execution', 'taskEvidencePlanDigest',
  ], 'programme policy v5');
  if (input.schemaVersion !== 1 || input.policyId !== PROGRAMME_POLICY_V5_ID
    || input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('programme policy v5 identity is invalid');
  }
  const config = parseHarnessConfig(input.harnessConfig);
  const bootstrap = parseBootstrapPolicy(input.bootstrap);
  const controller = parseControllerPolicy(input.controller, config);
  const execution = parseExecutionPolicy(input.execution, config, controller.task);
  for (const [path, digest] of [
    [controller.snapshot.manifestPath, controller.snapshot.manifestBlobDigest],
    [controller.snapshot.taskPath, controller.snapshot.taskBlobDigest],
    [controller.snapshot.buildManifestPath, controller.snapshot.buildManifestBlobDigest],
    ['coding-harness/package-lock.json', controller.snapshot.lockfileDigest],
    ['Cargo.lock', controller.task.rust.frozenLockSha256],
  ] as const) {
    if (execution.snapshot.protectedInputs[path] !== digest) {
      throw new Error('HARNESS_PROGRAMME_POLICY_PROTECTED_INPUT_BINDING_MISMATCH');
    }
  }
  const taskEvidencePlanDigest = parseDigest(
    input.taskEvidencePlanDigest,
    'programme policy v5.taskEvidencePlanDigest',
  );
  const boundTask = bindProgrammeTaskRuntimeV1(controller.task);
  if (programmeBoundTaskDigestV1(controller.task) !== execution.snapshot.boundTaskDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_BOUND_TASK_MISMATCH');
  }
  const evidencePlan = resolveTaskEvidencePlanV1({
    task: boundTask,
    taskPath: controller.taskPath,
  });
  if (evidencePlan.declarationDigest !== taskEvidencePlanDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_EVIDENCE_PLAN_MISMATCH');
  }
  const gateContract = parseProgrammeGateContractV1(input.gateContract);
  if (controller.task.candidateOracle.mode !== gateContract.candidate.oracle.requiredMode) {
    throw new Error('HARNESS_PROGRAMME_POLICY_CANDIDATE_ORACLE_INVALID');
  }
  if (bootstrap.nodeDigest !== gateContract.tools.exactValues.bootstrapNodeDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_BOOTSTRAP_NODE_MISMATCH');
  }
  if (bootstrap.gitDigest !== gateContract.tools.exactValues.bootstrapGitDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_BOOTSTRAP_GIT_MISMATCH');
  }
  if (digestValue(controller.manifest.runtime) !== digestValue(gateContract.tools.runtimePackages)) {
    throw new Error('HARNESS_PROGRAMME_POLICY_RUNTIME_BINDING_MISMATCH');
  }
  const snapshot = deepFreeze({
    schemaVersion: 1 as const,
    policyId: PROGRAMME_POLICY_V5_ID,
    authority: DEVELOPMENT_AUTHORITY,
    gateContract,
    harnessConfig: config,
    bootstrap,
    controller: controller.snapshot,
    execution: execution.snapshot,
    taskEvidencePlanDigest,
  });
  const fingerprint = digestValue(snapshot);
  return deepFreeze({
    snapshot,
    fingerprint,
    config,
    manifest: controller.manifest,
    task: controller.task,
    boundTask,
    build: controller.build,
    evidencePlan,
    routeSnapshot: execution.routeSnapshot,
  });
}

function parseBootstrapPolicy(value: unknown): FrozenProgrammePolicyV1['bootstrap'] {
  const input = asRecord(value, 'programme policy v5.bootstrap');
  assertExactKeys(input, [
    'controllerStoreDigest', 'nodeDigest', 'gitDigest',
  ], 'programme policy v5.bootstrap');
  return deepFreeze({
    controllerStoreDigest: parseDigest(
      input.controllerStoreDigest,
      'programme policy v5 bootstrap controllerStoreDigest',
    ),
    nodeDigest: parseDigest(input.nodeDigest, 'programme policy v5 bootstrap nodeDigest'),
    gitDigest: parseDigest(input.gitDigest, 'programme policy v5 bootstrap gitDigest'),
  });
}

function parseExecutionPolicy(
  value: unknown,
  config: HarnessConfig,
  task: AcceptanceTaskV3,
): Readonly<{
  snapshot: FrozenProgrammePolicyV1['execution'];
  routeSnapshot: unknown;
}> {
  const input = asRecord(value, 'programme policy v5.execution');
  assertExactKeys(input, [
    'evaluator', 'protectedInputs', 'boundTaskDigest', 'routeSnapshotBlob',
    'routeSnapshotBlobDigest', 'routeSnapshotDigest',
  ], 'programme policy v5.execution');
  const evaluator = parseIdentity(input.evaluator, 'programme policy v5 evaluator identity');
  const protectedInputs = parseProtectedInputs(input.protectedInputs, config, task);
  const boundTaskDigest = parseDigest(
    input.boundTaskDigest,
    'programme policy v5 execution boundTaskDigest',
  );
  const routeSnapshotBlob = parseBlob(
    input.routeSnapshotBlob,
    'programme policy v5 routeSnapshotBlob',
    1_000_000,
  );
  const routeSnapshotBlobDigest = parseDigest(
    input.routeSnapshotBlobDigest,
    'programme policy v5 execution routeSnapshotBlobDigest',
  );
  if (sha256(routeSnapshotBlob) !== routeSnapshotBlobDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_ROUTE_SNAPSHOT_BLOB_MISMATCH');
  }
  const routeSnapshot = parsePolicyBlob(routeSnapshotBlob, 'route snapshot');
  const routeSnapshotDigest = parseDigest(
    input.routeSnapshotDigest,
    'programme policy v5 execution routeSnapshotDigest',
  );
  if (digestValue(routeSnapshot) !== routeSnapshotDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_ROUTE_SNAPSHOT_MISMATCH');
  }
  return deepFreeze({
    snapshot: {
      evaluator,
      protectedInputs,
      boundTaskDigest,
      routeSnapshotBlob,
      routeSnapshotBlobDigest,
      routeSnapshotDigest,
    },
    routeSnapshot,
  });
}

function parseProtectedInputs(
  value: unknown,
  config: HarnessConfig,
  task: AcceptanceTaskV3,
): Readonly<Record<string, string>> {
  const input = asRecord(value, 'programme policy v5 execution protectedInputs');
  const expected = [...new Set([
    ...config.requiredProtectedPaths,
    ...task.evaluatorPaths,
    'Cargo.lock',
  ])].sort();
  if (JSON.stringify(Object.keys(input).sort()) !== JSON.stringify(expected)) {
    throw new Error('HARNESS_PROGRAMME_POLICY_PROTECTED_INPUT_SET_MISMATCH');
  }
  const entries = expected.map((path) => {
    const digest = parseDigest(input[path], `programme policy v5 protected input ${path}`);
    if (digest === '0'.repeat(64)) {
      throw new Error('HARNESS_PROGRAMME_POLICY_PROTECTED_INPUT_DIGEST_INVALID');
    }
    return [path, digest] as const;
  });
  return deepFreeze(Object.fromEntries(entries));
}

export function programmePolicyFingerprint(value: unknown): string {
  return canonicalizeFrozenProgrammePolicyV1(value).fingerprint;
}

function parseControllerPolicy(value: unknown, config: HarnessConfig): Readonly<{
  snapshot: FrozenProgrammePolicyV1['controller'];
  manifest: HarnessManifest;
  task: AcceptanceTaskV3;
  build: ControllerBuildManifest;
  taskPath: string;
}> {
  const input = asRecord(value, 'programme policy v5.controller');
  assertExactKeys(input, [
    'identity', 'manifestPath', 'manifestBlob', 'manifestBlobDigest',
    'taskPath', 'taskBlob', 'taskBlobDigest', 'buildManifestPath',
    'buildManifestBlob', 'buildManifestBlobDigest', 'runtimeTreeDigest',
    'lockfileDigest', 'executionDigest',
  ], 'programme policy v5.controller');
  const identity = parseIdentity(input.identity, 'programme policy v5 controller identity');
  if (input.manifestPath !== HARNESS_MANIFEST_PATH) {
    throw new TypeError('programme policy v5 controller manifestPath is invalid');
  }
  const manifestBlob = parseBlob(input.manifestBlob, 'programme policy v5 manifestBlob');
  const manifestBlobDigest = parseDigest(
    input.manifestBlobDigest,
    'programme policy v5 manifestBlobDigest',
  );
  if (sha256(manifestBlob) !== manifestBlobDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_MANIFEST_BLOB_MISMATCH');
  }
  const manifest = parseHarnessManifest(parsePolicyBlob(manifestBlob, 'manifest'), config);
  const taskPath = selectAcceptanceTaskPath(manifest, input.taskPath);
  const taskBlob = parseBlob(input.taskBlob, 'programme policy v5 taskBlob');
  const taskBlobDigest = parseDigest(input.taskBlobDigest, 'programme policy v5 taskBlobDigest');
  if (sha256(taskBlob) !== taskBlobDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_TASK_BLOB_MISMATCH');
  }
  const task = parseAcceptanceTask(parsePolicyBlob(taskBlob, 'task'), config);
  if (task.schemaVersion !== 3) throw new Error('HARNESS_PROGRAMME_POLICY_TASK_SCHEMA_INVALID');
  if (input.buildManifestPath !== CONTROLLER_BUILD_PATH) {
    throw new TypeError('programme policy v5 controller buildManifestPath is invalid');
  }
  const buildManifestBlob = parseBlob(
    input.buildManifestBlob,
    'programme policy v5 buildManifestBlob',
  );
  const buildManifestBlobDigest = parseDigest(
    input.buildManifestBlobDigest,
    'programme policy v5 buildManifestBlobDigest',
  );
  if (sha256(buildManifestBlob) !== buildManifestBlobDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_BUILD_MANIFEST_BLOB_MISMATCH');
  }
  const build = parseControllerBuildManifest(parsePolicyBlob(buildManifestBlob, 'build manifest'));
  const runtimeTreeDigest = parseDigest(
    input.runtimeTreeDigest,
    'programme policy v5 runtimeTreeDigest',
  );
  const lockfileDigest = parseDigest(input.lockfileDigest, 'programme policy v5 lockfileDigest');
  const executionDigest = parseDigest(input.executionDigest, 'programme policy v5 executionDigest');
  if (build.harnessManifestDigest !== manifestBlobDigest
    || build.runtimeTreeDigest !== runtimeTreeDigest
    || build.lockfileDigest !== lockfileDigest) {
    throw new Error('HARNESS_PROGRAMME_POLICY_CONTROLLER_BUILD_MISMATCH');
  }
  return deepFreeze({
    snapshot: {
      identity,
      manifestPath: HARNESS_MANIFEST_PATH,
      manifestBlob,
      manifestBlobDigest,
      taskPath,
      taskBlob,
      taskBlobDigest,
      buildManifestPath: CONTROLLER_BUILD_PATH,
      buildManifestBlob,
      buildManifestBlobDigest,
      runtimeTreeDigest,
      lockfileDigest,
      executionDigest,
    },
    manifest,
    task,
    build,
    taskPath,
  });
}

function parseIdentity(value: unknown, label: string): GitIdentity {
  const input = asRecord(value, label);
  assertExactKeys(input, ['commit', 'tree'], label);
  const parseObject = (entry: unknown, label: string) => {
    const object = asNonEmptyString(entry, label);
    if (!/^[a-f0-9]{40,64}$/.test(object)) throw new TypeError(`${label} is invalid`);
    return object;
  };
  return {
    commit: parseObject(input.commit, `${label} commit`),
    tree: parseObject(input.tree, `${label} tree`),
  };
}

function parseBlob(value: unknown, label: string, maximumBytes = 10_000_000): string {
  const blob = asNonEmptyString(value, label);
  if (Buffer.byteLength(blob, 'utf8') > maximumBytes) throw new TypeError(`${label} is too large`);
  return blob;
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value)) {
    throw new TypeError(`${label} must be a lowercase SHA-256 digest`);
  }
  return value;
}

function parsePolicyBlob(value: string, label: string): unknown {
  return parseJsonWithoutDuplicateKeys(value, `programme policy v5 ${label} blob`);
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function serializePolicyJson(value: unknown, label: string): string {
  let serialized: string | undefined;
  try { serialized = JSON.stringify(value); } catch {
    throw new TypeError(`${label} is not serializable JSON`);
  }
  if (serialized === undefined) throw new TypeError(`${label} is not serializable JSON`);
  parseJsonWithoutDuplicateKeys(serialized, label);
  return `${serialized}\n`;
}

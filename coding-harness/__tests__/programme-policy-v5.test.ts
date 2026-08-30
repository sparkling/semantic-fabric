// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  bindAcceptanceTaskToRustProfile,
  parseAcceptanceTask,
} from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { parseHarnessConfig, type HarnessConfig } from '../src/contracts.js';
import { HARNESS_MANIFEST_PATH } from '../src/controller-attestation.js';
import { CONTROLLER_BUILD_PATH } from '../src/controller-build.js';
import {
  createFrozenProgrammePolicyV1,
  programmePolicyFingerprint,
  verifyFrozenProgrammePolicyV1,
  type ControllerPolicyInputs,
} from '../src/programme-policy-v5.js';
import {
  receiptArtifactKey,
  receiptGeneratedOutputKey,
  receiptMutationKey,
  receiptQeKey,
  receiptVerifierKey,
} from '../src/programme-receipt-keys.js';
import {
  PROGRAMME_RUST_COMMAND_PROFILE_V1,
  bindProgrammeTaskRuntimeV1,
  programmeCommandReceiptProjectionsV1,
} from '../src/programme-task-runtime-v1.js';
import { resolveTaskEvidencePlanV1 } from '../src/task-evidence-plan.js';
import type { RustOfflineProfile } from '../src/rust-sandbox.js';
import { PROGRAMME_V5_POST_HISTORICAL_PATHS }
  from './programme-v5-post-historical-paths.js';

const taskPath = 'coding-harness/config/issue-8-acceptance.json';
const EXPECTED_POLICY_FINGERPRINT =
  'ab6e0efe3c1ba836e1719b2ebe168a3ff27b39a45fe21e750658271f191709dd';
const HISTORICAL_POLICY_FINGERPRINT =
  '7888d16a81b048d2bd1a436047cac8ebd13d61050daeff670371140383526c3c';
const HISTORICAL_MANIFEST_DIGEST =
  'f1dbcaf5c49c45b84e0c4bb09f305b9787eaa2493f659e445eeced60324cf104';
const manifestUrl = new URL('../.harness/manifest.json', import.meta.url);
const taskUrl = new URL('../config/issue-8-acceptance.json', import.meta.url);

describe('frozen schema-v5 programme policy', () => {
  it('round-trips a self-contained schema-v3 policy with a pinned fingerprint', () => {
    const input = policyInput();
    const policy = createFrozenProgrammePolicyV1(input);
    const fingerprint = programmePolicyFingerprint(policy);
    expect(fingerprint).toBe(EXPECTED_POLICY_FINGERPRINT);
    const parsed = verifyFrozenProgrammePolicyV1(policy, EXPECTED_POLICY_FINGERPRINT);
    expect(parsed.fingerprint).toBe(EXPECTED_POLICY_FINGERPRINT);
    expect(programmePolicyFingerprint(policy)).toBe(parsed.fingerprint);
    expect(verifyFrozenProgrammePolicyV1(policy, parsed.fingerprint)).toEqual(parsed);
    expect(parsed.task.schemaVersion).toBe(3);
    expect(parsed.evidencePlan.declarationDigest).toBe(input.taskEvidencePlanDigest);
    expect(parsed.snapshot.controller.manifestBlob).toBe(input.controller.manifestBlob);
    expect(parsed.snapshot.controller.taskBlob).toBe(input.controller.taskBlob);
    for (const value of [
      parsed, parsed.snapshot, parsed.snapshot.gateContract,
      parsed.snapshot.harnessConfig, parsed.snapshot.controller,
      parsed.snapshot.execution, parsed.manifest, parsed.task, parsed.boundTask,
      parsed.build, parsed.evidencePlan,
    ]) expect(Object.isFrozen(value)).toBe(true);
  });

  it('preserves the pre-capture V5 policy fingerprint as a historical anchor', () => {
    const { manifestBlob, harnessConfig } = historicalManifest();
    expect(sha256(manifestBlob)).toBe(HISTORICAL_MANIFEST_DIGEST);
    const input = policyInput(manifestBlob, harnessConfig);
    const policy = historicalPolicy(input, harnessConfig);
    expect(programmePolicyFingerprint(policy)).toBe(HISTORICAL_POLICY_FINGERPRINT);
    expect(verifyFrozenProgrammePolicyV1(policy, HISTORICAL_POLICY_FINGERPRINT).fingerprint)
      .toBe(HISTORICAL_POLICY_FINGERPRINT);
  });

  it('keeps the declaration digest invariant across Rust runtime binding', () => {
    const input = policyInput();
    const policy = createFrozenProgrammePolicyV1(input);
    const parsed = verifyFrozenProgrammePolicyV1(policy, EXPECTED_POLICY_FINGERPRINT);
    const profile: RustOfflineProfile = Object.freeze({
      cargoExecutable: '/toolchain/bin/cargo',
      environment: PROGRAMME_RUST_COMMAND_PROFILE_V1.environment,
      readOnlyMounts: Object.freeze([]),
      isolator: Object.freeze({ isolate() {}, assertStable() {} }),
    });
    const bound = bindAcceptanceTaskToRustProfile(parsed.task, profile);

    expect(bound).toEqual(parsed.boundTask);
    expect(resolveTaskEvidencePlanV1({ task: bound, taskPath }).declarationDigest)
      .toBe(parsed.evidencePlan.declarationDigest);
    expect(programmeCommandReceiptProjectionsV1(parsed.task))
      .toEqual(programmeCommandReceiptProjectionsV1(bound));
  });

  it('projects transient repository refs out of frozen Git identities', () => {
    const input = policyInput() as any;
    input.controller.identity.ref = 'refs/heads/controller';
    input.execution.evaluator.ref = 'refs/metaharness/evaluators/test';

    const policy = createFrozenProgrammePolicyV1(input);

    expect(policy.controller.identity).toEqual({ commit: '1'.repeat(40), tree: '2'.repeat(40) });
    expect(policy.execution.evaluator).toEqual({ commit: '9'.repeat(40), tree: '8'.repeat(40) });
    expect(programmePolicyFingerprint(policy)).toBe(EXPECTED_POLICY_FINGERPRINT);

    const serialized = structuredClone(policy) as any;
    serialized.execution.evaluator.ref = 'refs/metaharness/evaluators/test';
    expect(() => verifyFrozenProgrammePolicyV1(serialized, EXPECTED_POLICY_FINGERPRINT))
      .toThrow('programme policy v5 evaluator identity has invalid keys');
  });

  it('fails closed on raw-input, gate, config, path, or derived-plan tampering', () => {
    const policy = createFrozenProgrammePolicyV1(policyInput());
    const fingerprint = EXPECTED_POLICY_FINGERPRINT;

    const rawTask = structuredClone(policy);
    rawTask.controller.taskBlob = `${rawTask.controller.taskBlob} `;
    expect(() => verifyFrozenProgrammePolicyV1(rawTask, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_TASK_BLOB_MISMATCH');

    const rehashedTask = structuredClone(policy);
    rehashedTask.controller.taskBlob = `${rehashedTask.controller.taskBlob} `;
    rehashedTask.controller.taskBlobDigest = sha256(rehashedTask.controller.taskBlob);
    rehashedTask.execution.protectedInputs[taskPath] = rehashedTask.controller.taskBlobDigest;
    expect(() => verifyFrozenProgrammePolicyV1(rehashedTask, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_FINGERPRINT_MISMATCH');

    const rawManifest = structuredClone(policy);
    rawManifest.controller.manifestBlob = `${rawManifest.controller.manifestBlob} `;
    expect(() => verifyFrozenProgrammePolicyV1(rawManifest, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_MANIFEST_BLOB_MISMATCH');

    const gate = structuredClone(policy) as any;
    gate.gateContract.acceptance.threshold = 97;
    expect(() => verifyFrozenProgrammePolicyV1(gate, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_GATE_CONTRACT_INVALID');

    const plan = structuredClone(policy) as any;
    plan.taskEvidencePlanDigest = 'f'.repeat(64);
    expect(() => verifyFrozenProgrammePolicyV1(plan, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_EVIDENCE_PLAN_MISMATCH');

    const identity = structuredClone(policy) as any;
    identity.controller.identity.commit = 'b'.repeat(40);
    expect(() => verifyFrozenProgrammePolicyV1(identity, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_FINGERPRINT_MISMATCH');

    const path = structuredClone(policy) as any;
    path.controller.taskPath = 'coding-harness/config/unlisted-acceptance.json';
    expect(() => verifyFrozenProgrammePolicyV1(path, fingerprint))
      .toThrow('HARNESS_MANIFEST_TASK_NOT_LISTED');

    const config = structuredClone(policy) as any;
    config.harnessConfig.requiredProtectedPaths.pop();
    expect(() => verifyFrozenProgrammePolicyV1(config, fingerprint))
      .toThrow('HARNESS_MANIFEST_PROTECTED_PATHS_MISMATCH');

    const unknown = { ...structuredClone(policy), extra: true };
    expect(() => verifyFrozenProgrammePolicyV1(unknown, fingerprint)).toThrow(/invalid keys/);

    const gateUndefined = structuredClone(policy) as any;
    gateUndefined.gateContract.tools.exactValues.unexpected = undefined;
    expect(() => verifyFrozenProgrammePolicyV1(gateUndefined, fingerprint)).toThrow(/invalid keys/);

    const execution = structuredClone(policy) as any;
    execution.controller.executionDigest = 'e'.repeat(64);
    expect(() => verifyFrozenProgrammePolicyV1(execution, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_FINGERPRINT_MISMATCH');

    const build = structuredClone(policy) as any;
    build.controller.runtimeTreeDigest = 'd'.repeat(64);
    expect(() => verifyFrozenProgrammePolicyV1(build, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_CONTROLLER_BUILD_MISMATCH');

    const evaluator = structuredClone(policy) as any;
    evaluator.execution.evaluator.tree = 'e'.repeat(40);
    expect(() => verifyFrozenProgrammePolicyV1(evaluator, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_FINGERPRINT_MISMATCH');

    for (const protectedPath of [
      HARNESS_MANIFEST_PATH,
      taskPath,
      CONTROLLER_BUILD_PATH,
      'coding-harness/package-lock.json',
      'Cargo.lock',
    ]) {
      const protectedInput = structuredClone(policy) as any;
      protectedInput.execution.protectedInputs[protectedPath] = 'e'.repeat(64);
      expect(() => verifyFrozenProgrammePolicyV1(protectedInput, fingerprint))
        .toThrow('HARNESS_PROGRAMME_POLICY_PROTECTED_INPUT_BINDING_MISMATCH');
    }

    const route = structuredClone(policy) as any;
    route.execution.routeSnapshotBlob = '{"historyEpoch":0,"decisions":{}}\n ';
    expect(() => verifyFrozenProgrammePolicyV1(route, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_ROUTE_SNAPSHOT_BLOB_MISMATCH');

    const boundTask = structuredClone(policy) as any;
    boundTask.execution.boundTaskDigest = 'e'.repeat(64);
    expect(() => verifyFrozenProgrammePolicyV1(boundTask, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_BOUND_TASK_MISMATCH');

    const base = policyInput();
    const v2Blob = readFileSync(taskUrl, 'utf8');
    const v2 = {
      ...base,
      controller: { ...base.controller, taskBlob: v2Blob, taskBlobDigest: sha256(v2Blob) },
    };
    expect(() => createFrozenProgrammePolicyV1(v2))
      .toThrow('HARNESS_PROGRAMME_POLICY_TASK_SCHEMA_INVALID');

    expect(() => createFrozenProgrammePolicyV1({ ...policyInput(), maxRepairs: 11 }))
      .toThrow(/cannot exceed 10/);

    const weakerConfig = structuredClone(SECURE_HARNESS_CONFIG) as any;
    weakerConfig.requiredProtectedPaths = [HARNESS_MANIFEST_PATH];
    expect(() => createFrozenProgrammePolicyV1({
      ...policyInput(), harnessConfig: weakerConfig,
    } as any)).toThrow(/invalid keys/);

    const reanchoredWeakerConfig = structuredClone(policy) as any;
    reanchoredWeakerConfig.harnessConfig.requiredProtectedPaths.pop();
    const weakenedManifest = JSON.parse(
      reanchoredWeakerConfig.controller.manifestBlob,
    ) as Record<string, any>;
    weakenedManifest.protectedPaths = reanchoredWeakerConfig.harnessConfig.requiredProtectedPaths;
    reanchoredWeakerConfig.controller.manifestBlob = `${JSON.stringify(weakenedManifest, null, 2)}\n`;
    reanchoredWeakerConfig.controller.manifestBlobDigest = sha256(
      reanchoredWeakerConfig.controller.manifestBlob,
    );
    const weakenedBuild = JSON.parse(
      reanchoredWeakerConfig.controller.buildManifestBlob,
    ) as Record<string, any>;
    weakenedBuild.harnessManifestDigest = reanchoredWeakerConfig.controller.manifestBlobDigest;
    const { runtimeTreeDigest: _oldTree, ...weakenedBuildBody } = weakenedBuild;
    weakenedBuild.runtimeTreeDigest = sha256(JSON.stringify(weakenedBuildBody));
    reanchoredWeakerConfig.controller.buildManifestBlob = `${JSON.stringify(weakenedBuild, null, 2)}\n`;
    reanchoredWeakerConfig.controller.buildManifestBlobDigest = sha256(
      reanchoredWeakerConfig.controller.buildManifestBlob,
    );
    reanchoredWeakerConfig.controller.runtimeTreeDigest = weakenedBuild.runtimeTreeDigest;
    expect(() => verifyFrozenProgrammePolicyV1(reanchoredWeakerConfig, fingerprint))
      .toThrow('HARNESS_PROGRAMME_POLICY_PROTECTED_INPUT_SET_MISMATCH');
  });

  it('rejects duplicate keys in every embedded controller JSON blob', () => {
    const task = policyInput() as any;
    task.controller.taskBlob = task.controller.taskBlob.replace(
      '"objective":',
      '"objective":"shadowed objective","objective":',
    );
    task.controller.taskBlobDigest = sha256(task.controller.taskBlob);
    expect(() => createFrozenProgrammePolicyV1(task)).toThrow(/duplicate JSON key: objective/);

    const manifest = policyInput() as any;
    manifest.controller.manifestBlob = manifest.controller.manifestBlob.replace(
      '"name":',
      '"name":"shadowed-name","name":',
    );
    manifest.controller.manifestBlobDigest = sha256(manifest.controller.manifestBlob);
    expect(() => createFrozenProgrammePolicyV1(manifest)).toThrow(/duplicate JSON key: name/);

    const build = policyInput() as any;
    build.controller.buildManifestBlob = build.controller.buildManifestBlob.replace(
      '"runtimeEntry":',
      '"runtimeEntry":"coding-harness/dist/other.js","runtimeEntry":',
    );
    build.controller.buildManifestBlobDigest = sha256(build.controller.buildManifestBlob);
    expect(() => createFrozenProgrammePolicyV1(build)).toThrow(/duplicate JSON key: runtimeEntry/);
  });

  it('freezes replay-complete final-attempt key and gate semantics', () => {
    const policy = createFrozenProgrammePolicyV1(policyInput());
    const gate = policy.gateContract;
    const attempt = 2;

    expect(receiptArtifactKey(attempt, 'crates/sf-sparql/src/unfold.rs'))
      .toBe(gate.attempts.artifacts.key
        .replace('{attempt}', String(attempt)).replace('{path}', 'crates/sf-sparql/src/unfold.rs'));
    expect(receiptVerifierKey(attempt, 'independent'))
      .toBe(gate.attempts.verifiers.key
        .replace('{attempt}', String(attempt)).replace('{stage}', 'independent'));
    expect(receiptGeneratedOutputKey(attempt, 'regression', 'workspace-tests-earl'))
      .toBe(gate.attempts.generatedOutputs.key
        .replace('{attempt}', String(attempt)).replace('{stage}', 'regression')
        .replace('{evidenceId}', 'workspace-tests-earl'));
    expect(receiptMutationKey(attempt, 'checked_prune_0001'))
      .toBe(gate.attempts.mutations.key
        .replace('{attempt}', String(attempt)).replace('{mutationId}', 'checked_prune_0001'));
    expect(receiptQeKey(attempt, 'sast'))
      .toBe(gate.attempts.qe.key
        .replace('{attempt}', String(attempt)).replace('{profile}', 'sast'));
    expect(gate.policyBindings).toContainEqual([
      'receipt.toolVersions.controllerExecutionDigest', 'policy.controller.executionDigest',
    ]);
    expect(gate.tools.keySetRule).toBe('exactly-requiredKeys');
    expect(gate.tools.exactValues.codexExecutable).toContain('#sha256:73dc5888');
    expect(gate.tools.structuredValueRules.rustRegistryLock.leadingBinding)
      .toBe('policy.task.rust.frozenLockSha256');
    expect(gate.diagnostics.pass).toEqual({
      success: true, degraded: false, exitCode: 0, scaffoldReady: true,
      hardConstraintsPassed: 'equals-hardConstraintsTotal',
    });
  });
});

function policyInput(
  manifestBlob = readFileSync(manifestUrl, 'utf8'),
  harnessConfig: HarnessConfig = SECURE_HARNESS_CONFIG,
): ControllerPolicyInputs {
  const task = JSON.parse(readFileSync(taskUrl, 'utf8')) as Record<string, any>;
  task.schemaVersion = 3;
  task.taskId = 'verifier_only_task_0001';
  task.workItem = 'completion-programme:reproducibility';
  task.candidateOracle = { mode: 'verifier-only' };
  delete task.qeProfiles;
  task.rust = { frozenLockSha256: 'a'.repeat(64) };
  task.qe = {
    profiles: [
      { profile: 'sast', collector: 'agentic-qe-sast' },
      {
        profile: 'lcov-gap', collector: 'rust-lcov',
        packageName: 'sf-conformance', testTarget: 'issue_8_binding_pruning',
      },
    ],
  };
  task.evidence = {
    requiredAdmittedPaths: ['crates/sf-sparql/src/unfold.rs'],
    generatedOutputs: [{
      stage: 'regression', evidenceId: 'workspace-tests-earl',
      commandId: 'workspace-tests',
      workspacePaths: ['tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl'],
    }],
  };
  const taskBlob = `${JSON.stringify(task, null, 2)}\n`;
  const parsedTask = parseAcceptanceTask(JSON.parse(taskBlob), harnessConfig);
  const boundTask = bindProgrammeTaskRuntimeV1(parsedTask);
  const evidencePlan = resolveTaskEvidencePlanV1({ task: boundTask, taskPath });
  const manifestBlobDigest = sha256(manifestBlob);
  const taskBlobDigest = sha256(taskBlob);
  const protectedInputs = Object.fromEntries([...new Set([
    ...harnessConfig.requiredProtectedPaths,
    ...parsedTask.evaluatorPaths,
    'Cargo.lock',
  ])].sort().map((path, index) => [path, sha256(`${index}:${path}`)]));
  protectedInputs[HARNESS_MANIFEST_PATH] = manifestBlobDigest;
  protectedInputs[taskPath] = taskBlobDigest;
  protectedInputs['Cargo.lock'] = parsedTask.rust.frozenLockSha256;
  const buildBody = {
    schemaVersion: 1,
    authority: 'development-only-no-promotion',
    runtimeEntry: 'coding-harness/dist/issue-8-program.js',
    harnessManifestDigest: manifestBlobDigest,
    lockfileDigest: '3'.repeat(64),
    outputs: { 'coding-harness/dist/issue-8-program.js': '4'.repeat(64) },
    productionFiles: { 'coding-harness/node_modules/example/index.js': '5'.repeat(64) },
  } as const;
  const build = { ...buildBody, runtimeTreeDigest: sha256(JSON.stringify(buildBody)) };
  const buildManifestBlob = `${JSON.stringify(build, null, 2)}\n`;
  protectedInputs[CONTROLLER_BUILD_PATH] = sha256(buildManifestBlob);
  protectedInputs['coding-harness/package-lock.json'] = build.lockfileDigest;
  return {
    bootstrap: {
      controllerStoreDigest: '7'.repeat(64),
      nodeDigest: '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc',
      gitDigest: '2a8c18fbf43da9f692d75474c72bea9dfd796c260b0f3dfe456376abc3bbd668',
    },
    controller: {
      identity: { commit: '1'.repeat(40), tree: '2'.repeat(40) },
      manifestPath: HARNESS_MANIFEST_PATH,
      manifestBlob,
      manifestBlobDigest,
      taskPath,
      taskBlob,
      taskBlobDigest,
      buildManifestPath: CONTROLLER_BUILD_PATH,
      buildManifestBlob,
      buildManifestBlobDigest: sha256(buildManifestBlob),
      build,
      task: parsedTask,
      executionDigest: '6'.repeat(64),
    },
    execution: {
      evaluator: { commit: '9'.repeat(40), tree: '8'.repeat(40) },
      protectedInputs,
      routeSnapshot: {
        historyEpoch: 0,
        decisions: {
          architecture: {
            runId: 'programme_run_0001',
            taskDigest: 'a'.repeat(64),
            stepKind: 'architecture',
            candidateId: 'codex-architect',
            predictedQuality: 0,
            mode: 'cold-start',
            embedding: [0, 1],
            historyEpoch: 0,
            subscriptionCostUsd: 0,
          },
        },
      },
    },
    taskEvidencePlanDigest: evidencePlan.declarationDigest,
    maxRepairs: 2,
  };
}

function historicalManifest(): { manifestBlob: string; harnessConfig: HarnessConfig } {
  const manifest = JSON.parse(readFileSync(manifestUrl, 'utf8')) as Record<string, any>;
  manifest.protectedPaths = manifest.protectedPaths.filter(
    (path: string) => !PROGRAMME_V5_POST_HISTORICAL_PATHS.has(path),
  );
  const manifestBlob = `${JSON.stringify(manifest, null, 2)}\n`;
  const harnessConfig = parseHarnessConfig({
    ...structuredClone(SECURE_HARNESS_CONFIG),
    requiredProtectedPaths: SECURE_HARNESS_CONFIG.requiredProtectedPaths
      .filter((path) => !PROGRAMME_V5_POST_HISTORICAL_PATHS.has(path)),
  });
  return { manifestBlob, harnessConfig };
}

function historicalPolicy(
  input: ControllerPolicyInputs,
  harnessConfig: HarnessConfig,
): ReturnType<typeof createFrozenProgrammePolicyV1> {
  const policy = structuredClone(createFrozenProgrammePolicyV1(policyInput())) as any;
  policy.harnessConfig = harnessConfig;
  Object.assign(policy.controller, {
    manifestBlob: input.controller.manifestBlob,
    manifestBlobDigest: input.controller.manifestBlobDigest,
    buildManifestBlob: input.controller.buildManifestBlob,
    buildManifestBlobDigest: input.controller.buildManifestBlobDigest,
    runtimeTreeDigest: input.controller.build.runtimeTreeDigest,
    lockfileDigest: input.controller.build.lockfileDigest,
  });
  policy.execution.protectedInputs = input.execution.protectedInputs;
  return verifyFrozenProgrammePolicyV1(policy, HISTORICAL_POLICY_FINGERPRINT).snapshot;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

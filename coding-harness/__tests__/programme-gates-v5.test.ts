// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parseAcceptanceTask } from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { DEVELOPMENT_AUTHORITY } from '../src/contracts.js';
import { HARNESS_MANIFEST_PATH } from '../src/controller-attestation.js';
import { CONTROLLER_BUILD_PATH } from '../src/controller-build.js';
import type { ProgrammeV5RufloEvidence } from '../src/evidence.js';
import {
  parseMetaHarnessDiagnosticSnapshot,
  type MetaHarnessDiagnosticSnapshot,
} from '../src/metaharness-diagnostics.js';
import { evaluateProgrammeGatesV5 } from '../src/programme-gates-v5.js';
import { programmeV5RufloSnapshotDigest } from '../src/programme-v5-ruflo-contract.js';
import {
  createFrozenProgrammePolicyV1,
  programmePolicyFingerprint,
  verifyFrozenProgrammePolicyV1,
  type ControllerPolicyInputs,
  type ParsedProgrammePolicyV1,
} from '../src/programme-policy-v5.js';
import {
  RED_BASELINE_RECEIPT_KEY,
  receiptArtifactKey,
  receiptGeneratedOutputKey,
  receiptMutationKey,
  receiptQeKey,
  receiptVerifierKey,
} from '../src/programme-receipt-keys.js';
import {
  programmeCommandReceiptProjectionsV1,
  type ProgrammeCommandReceiptProjectionV1,
} from '../src/programme-task-runtime-v1.js';
import {
  ReceiptChain,
  digestValue,
  type CommandEvidence,
  type Receipt,
  type ReceiptDraft,
} from '../src/receipts.js';
import { resolveTaskEvidencePlanV1 } from '../src/task-evidence-plan.js';
import { programmeV5RufloFixture } from './candidate-fixtures.js';

const taskPath = 'coding-harness/config/issue-8-acceptance.json';
const taskUrl = new URL('../config/issue-8-acceptance.json', import.meta.url);
const manifestUrl = new URL('../.harness/manifest.json', import.meta.url);
const diagnosticsUrl = new URL('../config/metaharness-diagnostics.json', import.meta.url);

interface GateInput {
  policy: ParsedProgrammePolicyV1;
  receipt: Receipt;
  diagnostics: MetaHarnessDiagnosticSnapshot;
  rufloEvidence: ProgrammeV5RufloEvidence;
}

describe('schema-v5 programme gate evaluator', () => {
  it('accepts exact replay evidence and returns one frozen digest per hard gate', () => {
    const result = evaluateProgrammeGatesV5(fixture());

    expect(result.dimensions.map(({ id }) => id)).toEqual([
      'policy-and-supply-chain-safety', 'evaluator-integrity',
      'evolution-containment', 'patched-candidate-verification',
      'dual-host-control-plane', 'reliability-and-receipts',
      'ruflo-and-qe-integration',
    ]);
    expect(result.dimensions.every(({ passed, evidenceDigest }) =>
      passed && /^[a-f0-9]{64}$/.test(evidenceDigest))).toBe(true);
    expect(result.diagnostics.map(({ target, passed }) => [target, passed])).toEqual([
      ['repository', true], ['coding-harness', true],
    ]);
    for (const value of [result, result.dimensions, ...result.dimensions,
      result.diagnostics, ...result.diagnostics]) expect(Object.isFrozen(value)).toBe(true);
  });

  const receiptCases: ReadonlyArray<readonly [string, string, (input: any) => void]> = [
    ['protected input binding', 'policy-and-supply-chain-safety', (x) => {
      x.receipt.protectedInputs[Object.keys(x.receipt.protectedInputs)[0]] = 'e'.repeat(64);
    }],
    ['tool allowlist', 'policy-and-supply-chain-safety', (x) => {
      x.receipt.toolVersions.unexpected = 'forbidden';
    }],
    ['policy fingerprint binding', 'policy-and-supply-chain-safety', (x) => {
      x.receipt.toolVersions.programmePolicyFingerprint = 'e'.repeat(64);
    }],
    ['route binding', 'policy-and-supply-chain-safety', (x) => {
      x.receipt.route.routerVersion = '@metaharness/router@9.9.9';
    }],
    ['red command projection', 'evaluator-integrity', (x) => {
      x.receipt.commands.find((c: CommandEvidence) => c.stage === 'red-baseline').argv.push('--tamper');
    }],
    ['red digest recomputation', 'evaluator-integrity', (x) => {
      x.receipt.verifierDigests[RED_BASELINE_RECEIPT_KEY] = 'e'.repeat(64);
    }],
    ['schema-v3 verifier-only candidate', 'evolution-containment', (x) => {
      x.policy.task.candidateOracle = { mode: 'exact-reference' };
    }],
    ['final build command', 'patched-candidate-verification', (x) => {
      x.receipt.commands.find((c: CommandEvidence) => c.stage === 'build').exitCode = 1;
    }],
    ['command stage allowlist', 'patched-candidate-verification', (x) => {
      x.receipt.commands.push({ ...structuredClone(x.receipt.commands[0]), stage: 'public' });
    }],
    ['artifact allowlist', 'patched-candidate-verification', (x) => {
      x.receipt.artifactDigests['attempt-0:unlisted'] = 'e'.repeat(64);
    }],
    ['verifier key allowlist', 'patched-candidate-verification', (x) => {
      x.receipt.verifierDigests['attempt-0:unknown'] = 'e'.repeat(64);
    }],
    ['mutation digest recomputation', 'patched-candidate-verification', (x) => {
      const key = Object.keys(x.receipt.verifierDigests).find((k) => k.includes(':mutation:'));
      x.receipt.verifierDigests[key] = 'e'.repeat(64);
    }],
    ['native host execution', 'dual-host-control-plane', (x) => {
      x.receipt.hosts[0].model = 'wrong-model';
    }],
    ['native reviews', 'dual-host-control-plane', (x) => {
      x.receipt.reviewDigests[1] = x.receipt.reviewDigests[0];
    }],
    ['reliability', 'reliability-and-receipts', (x) => {
      x.receipt.recovery.cancelled = true;
    }],
    ['QE digest order', 'ruflo-and-qe-integration', (x) => {
      x.receipt.coordination.agenticQeEvidenceDigests.reverse();
    }],
    ['Ruflo topology binding', 'ruflo-and-qe-integration', (x) => {
      x.receipt.toolVersions.rufloHive = 'mesh';
    }],
  ];

  it.each(receiptCases)('fails closed on %s', (_name, dimension, mutate) => {
    const input = mutableFixture();
    mutate(input);
    input.receipt = rehashReceipt(input.receipt);
    const result = evaluateProgrammeGatesV5(input);
    expect(result.dimensions.find(({ id }) => id === dimension)?.passed).toBe(false);
  });

  it('binds the embedded Ruflo evidence to route, run, task, and coordination', () => {
    const input = mutableFixture();
    input.receipt.runId = 'different_run'; input.receipt = rehashReceipt(input.receipt);
    const result = evaluateProgrammeGatesV5(input);
    expect(result.dimensions.at(-1)?.passed).toBe(false);
    const replay = mutableFixture();
    replay.receipt.issuedAt = '2026-08-25T11:59:59.000Z';
    replay.receipt = rehashReceipt(replay.receipt);
    expect(evaluateProgrammeGatesV5(replay).dimensions.at(-1)?.passed).toBe(false);
  });

  it('rejects an over-policy repair count before attempt-key expansion', () => {
    for (const repairCount of [3, Number.MAX_SAFE_INTEGER]) {
      const input = mutableFixture();
      input.receipt.recovery.repairCount = repairCount;
      input.receipt = rehashReceipt(input.receipt);
      expect(() => evaluateProgrammeGatesV5(input))
        .toThrow('HARNESS_PROGRAMME_REPAIR_COUNT_EXCEEDS_POLICY');
    }
  });

  it('binds every dimension digest to the receipt and Ruflo evidence', () => {
    const original = evaluateProgrammeGatesV5(mutableFixture());
    const changedReceipt = mutableFixture();
    changedReceipt.receipt.issuedAt = '2026-08-25T12:00:01.000Z';
    changedReceipt.receipt = rehashReceipt(changedReceipt.receipt);
    const receiptResult = evaluateProgrammeGatesV5(changedReceipt);
    const changedRuflo = mutableFixture();
    changedRuflo.rufloEvidence.taskStatus.progress = 51;
    changedRuflo.rufloEvidence.taskStatusDigest = programmeV5RufloSnapshotDigest(
      changedRuflo.rufloEvidence.taskStatus,
    );
    const rufloResult = evaluateProgrammeGatesV5(changedRuflo);

    expect(receiptResult.dimensions.every(({ passed }) => passed)).toBe(true);
    expect(rufloResult.dimensions.every(({ passed }) => passed)).toBe(true);
    expect(receiptResult.dimensions.map(({ evidenceDigest }) => evidenceDigest))
      .not.toEqual(original.dimensions.map(({ evidenceDigest }) => evidenceDigest));
    expect(rufloResult.dimensions.map(({ evidenceDigest }) => evidenceDigest))
      .not.toEqual(original.dimensions.map(({ evidenceDigest }) => evidenceDigest));
  });

  it('accepts retained prior evidence only when attempt one has every final set', () => {
    const input = repairedFixture();
    const result = evaluateProgrammeGatesV5(input);
    expect(result.dimensions.every(({ passed }) => passed)).toBe(true);
  });

  it.each([
    ['stale attempt-zero verifier substitution', (input: any) => {
      delete input.receipt.verifierDigests[receiptVerifierKey(1, 'public')];
    }],
    ['stale attempt-zero artifact substitution', (input: any) => {
      delete input.receipt.artifactDigests[
        receiptArtifactKey(1, input.policy.task.artifactPaths[0])
      ];
    }],
    ['stale attempt-zero generated-output substitution', (input: any) => {
      const evidence = input.policy.task.evidence.generatedOutputs[0];
      delete input.receipt.verifierDigests[
        receiptGeneratedOutputKey(1, evidence.stage, evidence.evidenceId)
      ];
    }],
    ['stale attempt-zero mutation substitution', (input: any) => {
      delete input.receipt.verifierDigests[
        receiptMutationKey(1, input.policy.task.commands.mutation[0].mutationId)
      ];
    }],
    ['stale attempt-zero QE substitution', (input: any) => {
      delete input.receipt.verifierDigests[
        receiptQeKey(1, input.policy.evidencePlan.requiredQeProfiles[0])
      ];
    }],
    ['future command evidence', (input: any) => {
      const future = structuredClone(input.receipt.commands.find(
        (command: CommandEvidence) => command.stage === 'build' && command.attempt === 1,
      ));
      future.attempt = 2;
      input.receipt.commands.push(future);
    }],
  ])('does not let %s satisfy repaired final gates', (_name, mutate) => {
    const input = repairedFixture();
    mutate(input);
    input.receipt = rehashReceipt(input.receipt);
    const result = evaluateProgrammeGatesV5(input);
    expect(result.dimensions.find(({ id }) =>
      id === 'patched-candidate-verification')?.passed).toBe(false);
  });

  it('accepts a declared prior-build prefix ending in failure', () => {
    const input = repairedFixture();
    const priorBuild = input.receipt.commands.filter((command) =>
      command.stage === 'build' && command.attempt === 0);
    priorBuild.at(-1)!.exitCode = 1;
    input.receipt = rehashReceipt(input.receipt);
    const result = evaluateProgrammeGatesV5(input);
    expect(result.dimensions.every(({ passed }) => passed)).toBe(true);
  });

  it('rejects diagnostic reordering at the boundary', () => {
    const input = mutableFixture();
    input.diagnostics.targets.reverse();
    input.diagnostics = rehashDiagnostics(input.diagnostics);
    expect(() => evaluateProgrammeGatesV5(input))
      .toThrow('HARNESS_PROGRAMME_DIAGNOSTIC_ORDER_INVALID');
  });

  it('projects diagnostics in contract order and fails a degraded target', () => {
    const input = mutableFixture();
    input.diagnostics.targets[0].degraded = true;
    input.diagnostics = rehashDiagnostics(input.diagnostics);
    const result = evaluateProgrammeGatesV5(input);
    expect(result.diagnostics.map(({ target, passed }) => [target, passed])).toEqual([
      ['repository', false], ['coding-harness', true],
    ]);
  });

  it('keeps generic harness-fit scores diagnostic-only', () => {
    const input = mutableFixture();
    for (const target of input.diagnostics.targets) target.harnessFit = 0;
    input.diagnostics = rehashDiagnostics(input.diagnostics);

    const result = evaluateProgrammeGatesV5(input);
    expect(result.diagnostics.map(({ harnessFit, passed }) => [harnessFit, passed]))
      .toEqual([[0, true], [0, true]]);
    expect(result.dimensions.every(({ passed }) => passed)).toBe(true);
  });
});

function fixture(): GateInput {
  const input = mutableFixture();
  return {
    ...input,
    policy: input.policy,
    receipt: input.receipt,
    diagnostics: input.diagnostics,
    rufloEvidence: input.rufloEvidence,
  };
}

function mutableFixture(): GateInput {
  const policy = policyFixture();
  const receipt = receiptFixture(policy);
  const diagnostics = parseMetaHarnessDiagnosticSnapshot(
    JSON.parse(readFileSync(diagnosticsUrl, 'utf8')),
  );
  const rufloEvidence: ProgrammeV5RufloEvidence = programmeV5RufloFixture({
    taskId: receipt.taskId, runId: receipt.runId,
    swarmId: receipt.coordination.swarmId!,
    coordinationTaskId: receipt.coordination.taskId!,
    hookIds: [...receipt.coordination.hookIds], traceIds: [...receipt.coordination.traceIds],
    routeSnapshotDigest: receipt.route.snapshotDigest, capturedAt: receipt.issuedAt,
    transactionStartedAt: receipt.route.frozenAt,
  });
  return structuredClone({ policy, receipt, diagnostics, rufloEvidence });
}

function repairedFixture(): GateInput {
  const input = mutableFixture();
  const receipt = input.receipt;
  receipt.recovery.repairCount = 1;
  receipt.patchDigest = 'c'.repeat(64);
  receipt.patchDigests.push(receipt.patchDigest);
  const finalCommands = receipt.commands
    .filter(({ stage }) => stage === 'build' || stage === 'mutation')
    .map((command) => ({ ...structuredClone(command), attempt: 1 }));
  receipt.commands.push(...finalCommands);
  for (const path of input.policy.task.artifactPaths) {
    receipt.artifactDigests[receiptArtifactKey(1, path)] = digestValue(`final:${path}`);
  }
  for (const stage of ['public', 'independent', 'regression'] as const) {
    receipt.verifierDigests[receiptVerifierKey(1, stage)] = digestValue(`final:${stage}`);
  }
  input.policy.task.evidence.generatedOutputs.forEach(({ stage, evidenceId }) => {
    receipt.verifierDigests[receiptGeneratedOutputKey(1, stage, evidenceId)] =
      digestValue(`final:${evidenceId}`);
  });
  const finalMutations = finalCommands.filter(({ stage }) => stage === 'mutation');
  input.policy.task.commands.mutation.forEach((entry, index) => {
    receipt.verifierDigests[receiptMutationKey(1, entry.mutationId)] = digestValue({
      mutationId: entry.mutationId, path: entry.path,
      searchDigest: digestValue(entry.search), replacementDigest: digestValue(entry.replacement),
      command: finalMutations[index],
    });
  });
  input.policy.evidencePlan.requiredQeProfiles.forEach((profile, index) => {
    receipt.verifierDigests[receiptQeKey(1, profile)] =
      receipt.coordination.agenticQeEvidenceDigests[index];
  });
  input.receipt = rehashReceipt(receipt);
  return input;
}

function policyFixture(): ParsedProgrammePolicyV1 {
  const taskInput = JSON.parse(readFileSync(taskUrl, 'utf8')) as Record<string, any>;
  taskInput.schemaVersion = 3;
  taskInput.taskId = 'verifier_only_task_0001';
  taskInput.workItem = 'completion-programme:reproducibility';
  taskInput.candidateOracle = { mode: 'verifier-only' };
  delete taskInput.qeProfiles;
  taskInput.rust = { frozenLockSha256: 'a'.repeat(64) };
  taskInput.qe = { profiles: [
    { profile: 'sast', collector: 'agentic-qe-sast' },
    { profile: 'lcov-gap', collector: 'rust-lcov', packageName: 'sf-conformance',
      testTarget: 'issue_8_binding_pruning' },
  ] };
  taskInput.evidence = {
    requiredAdmittedPaths: ['crates/sf-sparql/src/unfold.rs'],
    generatedOutputs: [{ stage: 'regression', evidenceId: 'workspace-tests-earl',
      commandId: 'workspace-tests',
      workspacePaths: ['tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl'] }],
  };
  const taskBlob = `${JSON.stringify(taskInput, null, 2)}\n`;
  const task = parseAcceptanceTask(taskInput, SECURE_HARNESS_CONFIG);
  if (task.schemaVersion !== 3) throw new Error('expected schema-v3 task');
  const evidencePlan = resolveTaskEvidencePlanV1({ task, taskPath });
  const manifestBlob = readFileSync(manifestUrl, 'utf8');
  const manifestBlobDigest = sha256(manifestBlob);
  const protectedInputs = Object.fromEntries([...new Set([
    ...SECURE_HARNESS_CONFIG.requiredProtectedPaths, ...task.evaluatorPaths, 'Cargo.lock',
  ])].sort().map((path, index) => [path, sha256(`${index}:${path}`)]));
  protectedInputs[HARNESS_MANIFEST_PATH] = manifestBlobDigest;
  protectedInputs[taskPath] = sha256(taskBlob);
  protectedInputs['Cargo.lock'] = task.rust.frozenLockSha256;
  const buildBody = {
    schemaVersion: 1, authority: DEVELOPMENT_AUTHORITY,
    runtimeEntry: 'coding-harness/dist/issue-8-program.js',
    harnessManifestDigest: manifestBlobDigest, lockfileDigest: '3'.repeat(64),
    outputs: { 'coding-harness/dist/issue-8-program.js': '4'.repeat(64) },
    productionFiles: { 'coding-harness/node_modules/example/index.js': '5'.repeat(64) },
  } as const;
  const build = { ...buildBody, runtimeTreeDigest: sha256(JSON.stringify(buildBody)) };
  const buildManifestBlob = `${JSON.stringify(build, null, 2)}\n`;
  protectedInputs[CONTROLLER_BUILD_PATH] = sha256(buildManifestBlob);
  protectedInputs['coding-harness/package-lock.json'] = build.lockfileDigest;
  const input: ControllerPolicyInputs = {
    bootstrap: { controllerStoreDigest: '7'.repeat(64),
      nodeDigest: '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc',
      gitDigest: '2a8c18fbf43da9f692d75474c72bea9dfd796c260b0f3dfe456376abc3bbd668' },
    controller: {
      identity: { commit: '1'.repeat(40), tree: '2'.repeat(40) },
      manifestPath: HARNESS_MANIFEST_PATH, manifestBlob, manifestBlobDigest,
      taskPath, taskBlob, taskBlobDigest: sha256(taskBlob),
      buildManifestPath: CONTROLLER_BUILD_PATH, buildManifestBlob,
      buildManifestBlobDigest: sha256(buildManifestBlob), build, task,
      executionDigest: '6'.repeat(64),
    },
    execution: {
      evaluator: { commit: '9'.repeat(40), tree: '8'.repeat(40) }, protectedInputs,
      routeSnapshot: { historyEpoch: 0, decisions: {} },
    },
    taskEvidencePlanDigest: evidencePlan.declarationDigest, maxRepairs: 2,
  };
  const snapshot = createFrozenProgrammePolicyV1(input);
  return verifyFrozenProgrammePolicyV1(snapshot, programmePolicyFingerprint(snapshot));
}

function receiptFixture(policy: ParsedProgrammePolicyV1): Receipt {
  const gate = policy.snapshot.gateContract;
  const candidate = { commit: 'c'.repeat(40), tree: 'd'.repeat(40) };
  const projections = programmeCommandReceiptProjectionsV1(policy.task);
  const commands = projections.filter(({ stage }) =>
    stage === 'red-baseline' || stage === 'build' || stage === 'mutation')
    .map((projection) => commandEvidence(projection, candidate.tree, policy.snapshot.execution.evaluator.tree));
  const red = commands.filter(({ stage }) => stage === 'red-baseline');
  const mutation = commands.filter(({ stage }) => stage === 'mutation');
  const verifierDigests: Record<string, string> = {
    [RED_BASELINE_RECEIPT_KEY]: digestValue({
      expected: policy.task.redBaseline.expected, commands: red,
    }),
    [receiptVerifierKey(0, 'public')]: digestValue('public'),
    [receiptVerifierKey(0, 'independent')]: digestValue('independent'),
    [receiptVerifierKey(0, 'regression')]: digestValue('regression'),
  };
  policy.task.evidence.generatedOutputs.forEach(({ stage, evidenceId }) => {
    verifierDigests[receiptGeneratedOutputKey(0, stage, evidenceId)] = digestValue(evidenceId);
  });
  policy.task.commands.mutation.forEach((entry, index) => {
    verifierDigests[receiptMutationKey(0, entry.mutationId)] = digestValue({
      mutationId: entry.mutationId, path: entry.path,
      searchDigest: digestValue(entry.search), replacementDigest: digestValue(entry.replacement),
      command: mutation[index],
    });
  });
  const qeDigests = policy.evidencePlan.requiredQeProfiles.map((profile) => digestValue(profile));
  policy.evidencePlan.requiredQeProfiles.forEach((profile, index) => {
    verifierDigests[receiptQeKey(0, profile)] = qeDigests[index];
  });
  const toolVersions = toolFixture(policy);
  const draft: ReceiptDraft = {
    schemaVersion: 3, runId: 'programme_run_0001', taskId: policy.task.taskId,
    step: 'candidate-transaction', status: 'pass', failureCode: null,
    authority: DEVELOPMENT_AUTHORITY, issuedAt: '2026-08-25T12:00:00.000Z',
    identities: { controller: policy.snapshot.controller.identity,
      baseline: policy.task.baseline, evaluator: policy.snapshot.execution.evaluator, candidate },
    protectedInputs: { ...policy.snapshot.execution.protectedInputs },
    route: { snapshotDigest: policy.snapshot.execution.routeSnapshotDigest,
      frozenAt: '2026-08-25T11:59:00.000Z', routerVersion: gate.route.routerVersion },
    hosts: gate.nativeControlPlane.hosts.map((host) => ({ ...host })),
    admittedPaths: [...policy.task.evidence.requiredAdmittedPaths],
    patchDigest: 'b'.repeat(64), patchDigests: ['b'.repeat(64)], toolVersions, commands,
    artifactDigests: Object.fromEntries(policy.task.artifactPaths.map((path) =>
      [receiptArtifactKey(0, path), digestValue(path)])),
    verifierDigests, critiqueDigests: [digestValue('critique')],
    reviewDigests: [digestValue('codex review'), digestValue('claude review')],
    recovery: { retryCount: 0, breakerState: 'closed', cancelled: false, repairCount: 0 },
    coordination: { swarmId: 'programme_swarm_0001', taskId: 'programme_coordination_0001',
      hookIds: ['hook-1'], traceIds: ['trace-1'], agenticQeEvidenceDigests: qeDigests,
      nativeEvidenceDigests: ['native-1', 'native-2', 'native-3', 'native-4'].map(digestValue),
      nativeRuntimeEvidenceDigest: digestValue('native-runtime') },
  };
  return new ReceiptChain().append(draft);
}

function commandEvidence(
  projection: ProgrammeCommandReceiptProjectionV1,
  candidateTree: string,
  evaluatorTree: string,
): CommandEvidence {
  const red = projection.stage === 'red-baseline';
  return { stage: projection.stage as CommandEvidence['stage'], attempt: 0,
    candidateTree: red ? evaluatorTree : candidateTree,
    tool: projection.tool, executable: projection.executable, argv: [...projection.argv],
    cwd: projection.cwd, exitCode: red || projection.stage === 'mutation' ? 101 : 0,
    signal: null, durationMs: 1, stdoutDigest: digestValue(`${projection.commandId}:stdout`),
    stderrDigest: digestValue(`${projection.commandId}:stderr`), timedOut: false,
    cancelled: false, outputLimitExceeded: false, spawnErrorDigest: null };
}

function toolFixture(policy: ParsedProgrammePolicyV1): Record<string, string> {
  const gate = policy.snapshot.gateContract;
  const tools = Object.fromEntries(gate.tools.requiredKeys.map((key) => [key, digestValue(key)]));
  Object.assign(tools, gate.tools.exactValues, {
    bootstrapControllerStoreDigest: policy.snapshot.bootstrap.controllerStoreDigest,
    bootstrapBuildManifestDigest: policy.snapshot.controller.buildManifestBlobDigest,
    bootstrapRuntimeTreeDigest: policy.snapshot.controller.runtimeTreeDigest,
    controllerExecutionDigest: policy.snapshot.controller.executionDigest,
    controllerBuildManifestDigest: policy.snapshot.controller.buildManifestBlobDigest,
    controllerRuntimeTreeDigest: policy.snapshot.controller.runtimeTreeDigest,
    controllerManifestDigest: policy.snapshot.controller.manifestBlobDigest,
    controllerTaskPath: policy.snapshot.controller.taskPath,
    controllerTaskPathDigest: digestValue(policy.snapshot.controller.taskPath),
    controllerTaskDigest: policy.snapshot.controller.taskBlobDigest,
    taskEvidencePlanDigest: policy.evidencePlan.declarationDigest,
    boundTaskDigest: policy.snapshot.execution.boundTaskDigest,
    programmePolicyFingerprint: policy.fingerprint,
    frozenCargoLockDigest: policy.task.rust.frozenLockSha256,
    rustToolchainClosure: `${gate.tools.structuredValueRules.rustToolchainClosure.leadingValue}:1:1`,
    rustRegistryBootstrapSnapshot: `${digestValue('registry-bootstrap')}:1:1`,
    rustRegistryClosure: `${gate.tools.structuredValueRules.rustRegistryClosure.leadingValue}:1:1`,
    rustRegistryLock: `${policy.task.rust.frozenLockSha256}:1:1`,
    rustRegistrySelection: `x86_64-unknown-linux-gnu:${digestValue('registry-selection')}`,
    rufloHive: 'hierarchical', rufloConsensus: 'raft',
  });
  return tools;
}

function rehashReceipt(receipt: Receipt): Receipt {
  const { digest: _digest, ...body } = receipt;
  return { ...body, digest: digestValue(body) };
}

function rehashDiagnostics(snapshot: MetaHarnessDiagnosticSnapshot): MetaHarnessDiagnosticSnapshot {
  const { digest: _digest, ...body } = snapshot;
  return { ...body, digest: digestValue(body) };
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

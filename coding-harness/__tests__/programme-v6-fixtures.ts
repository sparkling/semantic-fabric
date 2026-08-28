// SPDX-License-Identifier: MIT
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { parseAcceptanceTask } from '../src/acceptance-task.js';
import {
  completeCandidateRepairTransitionReset, createCandidateRepairTransitionDraft,
  sealCandidateRepairTransitions,
  type CandidateRepairTransition,
} from '../src/candidate-repair-transition.js';
import {
  createCandidateTransactionEvidenceV1, type CandidateTransactionEvidenceV1,
} from '../src/candidate-transaction-evidence-v1.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { DEVELOPMENT_AUTHORITY } from '../src/contracts.js';
import { HARNESS_MANIFEST_PATH } from '../src/controller-attestation.js';
import { CONTROLLER_BUILD_PATH, parseControllerBuildManifest } from '../src/controller-build.js';
import {
  parseMetaHarnessDiagnosticSnapshot, type MetaHarnessDiagnosticSnapshot,
} from '../src/metaharness-diagnostics.js';
import type { NativeRuntimeEvidenceV2 } from '../src/native-runtime-evidence-v2.js';
import {
  createFrozenProgrammePolicyV1, programmePolicyFingerprint, verifyFrozenProgrammePolicyV1,
  type ControllerPolicyInputs,
} from '../src/programme-policy-v5.js';
import {
  createFrozenProgrammePolicyV2, programmePolicyV2Fingerprint, verifyFrozenProgrammePolicyV2,
  type ParsedProgrammePolicyV2,
} from '../src/programme-policy-v6.js';
import type { ProgrammeV5RufloEvidence } from '../src/programme-v5-ruflo-contract.js';
import {
  RED_BASELINE_RECEIPT_KEY, receiptArtifactKey, receiptGeneratedOutputKey,
  receiptMutationKey, receiptQeKey, receiptVerifierKey,
} from '../src/programme-receipt-keys.js';
import {
  programmeCommandReceiptProjectionsV1, type ProgrammeCommandReceiptProjectionV1,
} from '../src/programme-task-runtime-v1.js';
import {
  ReceiptChain, digestValue, type CommandEvidence, type GitIdentity, type Receipt,
  type ReceiptDraft,
} from '../src/receipts.js';
import { resolveTaskEvidencePlanV1 } from '../src/task-evidence-plan.js';
import { diagnosticSnapshot, programmeV5RufloFixture } from './candidate-fixtures.js';
export type RepairDisposition = 'none' | 'not-started' | 'failed' | 'passed';
export interface ProgrammeV6Fixture {
  readonly policy: ParsedProgrammePolicyV2;
  receipt: Receipt;
  readonly diagnostics: MetaHarnessDiagnosticSnapshot;
  readonly rufloEvidence: ProgrammeV5RufloEvidence;
  candidateTransactionEvidence: CandidateTransactionEvidenceV1;
  readonly nativeRuntimeEvidence: NativeRuntimeEvidenceV2;
  readonly repairTransitions: readonly CandidateRepairTransition[];
}
const TASK_PATH = 'coding-harness/config/programme-v5-acceptance.json';
const TASK_URL = new URL('../config/programme-v5-acceptance.json', import.meta.url);
const MANIFEST_URL = new URL('../.harness/manifest.json', import.meta.url);
const BUILD_URL = new URL('../.harness/controller-build.json', import.meta.url);
const BASE_POLICY = basePolicyFixture();
const FROZEN_V2 = createFrozenProgrammePolicyV2(BASE_POLICY.snapshot, BASE_POLICY.fingerprint);
const POLICY_V2 = verifyFrozenProgrammePolicyV2(
  FROZEN_V2,
  programmePolicyV2Fingerprint(FROZEN_V2),
);
export function programmeV6Fixture(disposition: RepairDisposition = 'none'): ProgrammeV6Fixture {
  const repaired = disposition !== 'none';
  const finalAttempt = repaired ? 1 : 0;
  const evaluator = POLICY_V2.base.snapshot.execution.evaluator;
  const sourceCandidate = identity('a');
  const transitionSource = disposition === 'not-started' ? evaluator : sourceCandidate;
  const finalCandidate = identity('d');
  const patchDigests = repaired ? [digest('b'), digest('c')] : [digest('b')];
  const nativeRuntimeEvidence = nativeEvidence({
    repaired,
    evaluator,
    sourceCandidate: transitionSource,
    finalCandidate,
    taskId: POLICY_V2.base.task.taskId,
    runId: 'programme_v6_run_0001',
    patchDigests,
  });
  const repairTransitions = repaired
    ? transitionsFor(disposition, transitionSource, evaluator, nativeRuntimeEvidence, patchDigests)
    : [];
  const receipt = receiptFixture({
    disposition,
    finalAttempt,
    evaluator,
    sourceCandidate: transitionSource,
    finalCandidate,
    patchDigests,
    nativeRuntimeEvidence,
  });
  const candidateTransactionEvidence = createCandidateTransactionEvidenceV1({
    receipt,
    nativeRuntimeEvidence,
    repairTransitions,
  });
  return {
    policy: POLICY_V2,
    receipt,
    diagnostics: parseMetaHarnessDiagnosticSnapshot(diagnosticSnapshot),
    rufloEvidence: rufloEvidenceFor(receipt),
    candidateTransactionEvidence,
    nativeRuntimeEvidence,
    repairTransitions,
  };
}
export function rehashReceipt(receipt: Receipt): Receipt {
  const { digest: _digest, ...body } = receipt;
  return { ...body, digest: digestValue(body) };
}
export function rebindCandidateEvidence(fixture: ProgrammeV6Fixture): void {
  fixture.candidateTransactionEvidence = createCandidateTransactionEvidenceV1({
    receipt: fixture.receipt,
    nativeRuntimeEvidence: fixture.nativeRuntimeEvidence,
    repairTransitions: fixture.repairTransitions,
  });
}
export function declaredBuildCommands(
  fixture: ProgrammeV6Fixture, attempt: number, candidateTree: string,
): CommandEvidence[] {
  return programmeCommandReceiptProjectionsV1(fixture.policy.base.task)
    .filter(({ stage }) => stage === 'build')
    .map((projection) => commandEvidence(projection, attempt, candidateTree));
}
export function rehashCandidateEvidence(value: CandidateTransactionEvidenceV1): void {
  const mutable = value as unknown as Record<string, any>;
  mutable.repairTransitionsDigest = digestValue(mutable.repairTransitions);
  const { evidenceDigest: _digest, ...body } = mutable;
  mutable.evidenceDigest = digestValue(body);
}
function basePolicyFixture() {
  const taskBlob = readFileSync(TASK_URL, 'utf8');
  const task = parseAcceptanceTask(JSON.parse(taskBlob), SECURE_HARNESS_CONFIG);
  if (task.schemaVersion !== 3) throw new Error('expected schema-v3 task');
  const manifestBlob = readFileSync(MANIFEST_URL, 'utf8');
  const buildManifestBlob = readFileSync(BUILD_URL, 'utf8');
  const build = parseControllerBuildManifest(JSON.parse(buildManifestBlob));
  const evidencePlan = resolveTaskEvidencePlanV1({ task, taskPath: TASK_PATH });
  const protectedInputs = Object.fromEntries([...new Set([
    ...SECURE_HARNESS_CONFIG.requiredProtectedPaths,
    ...task.evaluatorPaths,
    'Cargo.lock',
  ])].sort().map((path, index) => [path, sha256(`${index}:${path}`)]));
  protectedInputs[HARNESS_MANIFEST_PATH] = sha256(manifestBlob);
  protectedInputs[TASK_PATH] = sha256(taskBlob);
  protectedInputs[CONTROLLER_BUILD_PATH] = sha256(buildManifestBlob);
  protectedInputs['coding-harness/package-lock.json'] = build.lockfileDigest;
  protectedInputs['Cargo.lock'] = task.rust.frozenLockSha256;
  const input: ControllerPolicyInputs = {
    bootstrap: {
      controllerStoreDigest: digest('7'),
      nodeDigest: '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc',
      gitDigest: '2a8c18fbf43da9f692d75474c72bea9dfd796c260b0f3dfe456376abc3bbd668',
    },
    controller: {
      identity: identity('1'),
      manifestPath: HARNESS_MANIFEST_PATH,
      manifestBlob,
      manifestBlobDigest: sha256(manifestBlob),
      taskPath: TASK_PATH,
      taskBlob,
      taskBlobDigest: sha256(taskBlob),
      buildManifestPath: CONTROLLER_BUILD_PATH,
      buildManifestBlob,
      buildManifestBlobDigest: sha256(buildManifestBlob),
      build,
      task,
      executionDigest: digest('6'),
    },
    execution: {
      evaluator: identity('9'),
      protectedInputs,
      routeSnapshot: { historyEpoch: 0, decisions: {} },
    },
    taskEvidencePlanDigest: evidencePlan.declarationDigest,
    maxRepairs: 2,
  };
  const snapshot = createFrozenProgrammePolicyV1(input);
  return verifyFrozenProgrammePolicyV1(snapshot, programmePolicyFingerprint(snapshot));
}
function receiptFixture(input: Readonly<{
  disposition: RepairDisposition;
  finalAttempt: number;
  evaluator: GitIdentity;
  sourceCandidate: GitIdentity;
  finalCandidate: GitIdentity;
  patchDigests: readonly string[];
  nativeRuntimeEvidence: NativeRuntimeEvidenceV2;
}>): Receipt {
  const policy = POLICY_V2.base;
  const projections = programmeCommandReceiptProjectionsV1(policy.task);
  const red = projections.filter(({ stage }) => stage === 'red-baseline')
    .map((projection) => commandEvidence(projection, 0, input.evaluator.tree));
  const priorBuild = input.disposition === 'failed' || input.disposition === 'passed'
    ? projections.filter(({ stage }) => stage === 'build').map((projection) =>
      commandEvidence(projection, 0, input.sourceCandidate.tree))
    : [];
  if (input.disposition === 'failed') priorBuild.at(-1)!.exitCode = 1;
  const finalBuild = projections.filter(({ stage }) => stage === 'build')
    .map((projection) => commandEvidence(projection, input.finalAttempt, input.finalCandidate.tree));
  const finalMutation = projections.filter(({ stage }) => stage === 'mutation')
    .map((projection) => commandEvidence(projection, input.finalAttempt, input.finalCandidate.tree));
  const commands = [...red, ...priorBuild, ...finalBuild, ...finalMutation];
  const verifierDigests = finalVerifierDigests(policy, input.finalAttempt, red, finalMutation);
  if (input.disposition === 'passed') {
    for (const stage of ['public', 'independent', 'regression'] as const) {
      verifierDigests[receiptVerifierKey(0, stage)] = digestValue(`prior:${stage}`);
    }
    for (const { stage, evidenceId } of policy.task.evidence.generatedOutputs) {
      verifierDigests[receiptGeneratedOutputKey(0, stage, evidenceId)] =
        digestValue(`prior:${evidenceId}`);
    }
  }
  const artifactDigests: Record<string, string> = Object.fromEntries(
    policy.task.artifactPaths.map((path) =>
      [receiptArtifactKey(input.finalAttempt, path), digestValue(`final:${path}`)]),
  );
  if (input.disposition === 'failed' || input.disposition === 'passed') {
    for (const path of policy.task.artifactPaths) {
      artifactDigests[receiptArtifactKey(0, path)] = digestValue(`prior:${path}`);
    }
  }
  const qeDigests = policy.evidencePlan.requiredQeProfiles.map((profile) => digestValue(profile));
  policy.evidencePlan.requiredQeProfiles.forEach((profile, index) => {
    verifierDigests[receiptQeKey(input.finalAttempt, profile)] = qeDigests[index];
  });
  const gate = policy.snapshot.gateContract;
  const draft: ReceiptDraft = {
    schemaVersion: 3,
    runId: input.nativeRuntimeEvidence.runId,
    taskId: policy.task.taskId,
    step: 'candidate-transaction',
    status: 'pass',
    failureCode: null,
    authority: DEVELOPMENT_AUTHORITY,
    issuedAt: '2026-08-25T12:00:00.000Z',
    identities: {
      controller: policy.snapshot.controller.identity,
      baseline: policy.task.baseline,
      evaluator: input.evaluator,
      candidate: input.finalCandidate,
    },
    protectedInputs: { ...policy.snapshot.execution.protectedInputs },
    route: {
      snapshotDigest: policy.snapshot.execution.routeSnapshotDigest,
      frozenAt: '2026-08-25T11:59:00.000Z',
      routerVersion: gate.route.routerVersion,
    },
    hosts: gate.nativeControlPlane.hosts.map((host) => ({ ...host })),
    admittedPaths: [...policy.task.evidence.requiredAdmittedPaths],
    patchDigest: input.patchDigests.at(-1)!,
    patchDigests: [...input.patchDigests],
    toolVersions: toolFixture(policy),
    commands,
    artifactDigests,
    verifierDigests,
    critiqueDigests: [digestValue('critique')],
    reviewDigests: [digestValue('codex review'), digestValue('claude review')],
    recovery: {
      retryCount: 0,
      breakerState: 'closed',
      cancelled: false,
      repairCount: input.finalAttempt,
    },
    coordination: {
      swarmId: 'programme_v6_swarm_0001',
      taskId: 'programme_v6_coordination_0001',
      hookIds: ['hook-1'],
      traceIds: ['trace-1'],
      agenticQeEvidenceDigests: qeDigests,
      nativeEvidenceDigests: [
        ...input.nativeRuntimeEvidence.hosts.map(digestValue),
        ...input.nativeRuntimeEvidence.invocations.map(digestValue),
      ],
      nativeRuntimeEvidenceDigest: digestValue(input.nativeRuntimeEvidence),
    },
  };
  return new ReceiptChain().append(draft);
}
function finalVerifierDigests(
  policy: typeof POLICY_V2.base,
  attempt: number,
  red: readonly CommandEvidence[],
  mutations: readonly CommandEvidence[],
): Record<string, string> {
  const values: Record<string, string> = {
    [RED_BASELINE_RECEIPT_KEY]: digestValue({ expected: policy.task.redBaseline.expected, commands: red }),
    [receiptVerifierKey(attempt, 'public')]: digestValue('public'),
    [receiptVerifierKey(attempt, 'independent')]: digestValue('independent'),
    [receiptVerifierKey(attempt, 'regression')]: digestValue('regression'),
  };
  policy.task.evidence.generatedOutputs.forEach(({ stage, evidenceId }) => {
    values[receiptGeneratedOutputKey(attempt, stage, evidenceId)] = digestValue(evidenceId);
  });
  policy.task.commands.mutation.forEach((entry, index) => {
    values[receiptMutationKey(attempt, entry.mutationId)] = digestValue({
      mutationId: entry.mutationId,
      path: entry.path,
      searchDigest: digestValue(entry.search),
      replacementDigest: digestValue(entry.replacement),
      command: mutations[index],
    });
  });
  return values;
}
function commandEvidence(
  projection: ProgrammeCommandReceiptProjectionV1,
  attempt: number,
  candidateTree: string,
): CommandEvidence {
  return {
    stage: projection.stage as CommandEvidence['stage'],
    attempt,
    candidateTree,
    tool: projection.tool,
    executable: projection.executable,
    argv: [...projection.argv],
    cwd: projection.cwd,
    exitCode: projection.stage === 'red-baseline' || projection.stage === 'mutation' ? 101 : 0,
    signal: null,
    durationMs: 1,
    stdoutDigest: digestValue(`${projection.commandId}:stdout:${attempt}`),
    stderrDigest: digestValue(`${projection.commandId}:stderr:${attempt}`),
    timedOut: false,
    cancelled: false,
    outputLimitExceeded: false,
    spawnErrorDigest: null,
  };
}
function nativeEvidence(input: Readonly<{
  repaired: boolean;
  evaluator: GitIdentity;
  sourceCandidate: GitIdentity;
  finalCandidate: GitIdentity;
  taskId: string;
  runId: string;
  patchDigests: readonly string[];
}>): NativeRuntimeEvidenceV2 {
  const hosts = POLICY_V2.base.snapshot.gateContract.nativeControlPlane.hosts;
  const host = (index: 0 | 1) => ({
    host: hosts[index].host,
    model: hosts[index].model,
    authentication: index === 0 ? 'chatgpt-subscription' as const : 'claude-subscription' as const,
    clientVersion: hosts[index].clientVersion,
    executablePath: index === 0 ? '/tools/codex' : '/tools/claude',
    executableDigest: digest(index === 0 ? '1' : '2'),
    preflightDigest: digest(index === 0 ? '3' : '4'),
    credentialCapability: 'invocation-private-copy' as const,
    hostCredentialPathMounted: false as const,
  });
  const invocations = [
    invocation('architecture-codex', 'codex', 'architecture', input.evaluator.tree, null),
    invocation('architecture-claude', 'claude-code', 'architecture', input.evaluator.tree, null),
    invocation('implementation-codex', 'codex', 'implementation', input.evaluator.tree, input.patchDigests[0]),
    ...(input.repaired ? [
      invocation('repair-claude', 'claude-code', 'repair', input.sourceCandidate.tree, input.patchDigests[1]),
    ] : []),
    invocation('review-codex', 'codex', 'review', input.finalCandidate.tree, null),
    invocation('review-claude', 'claude-code', 'review', input.finalCandidate.tree, null),
  ];
  return {
    schemaVersion: 2,
    source: 'trusted-native-runtime',
    taskId: input.taskId,
    runId: input.runId,
    hosts: [host(0), host(1)],
    invocations,
  };
}

function invocation(
  invocationId: string,
  host: 'codex' | 'claude-code',
  operation: 'architecture' | 'implementation' | 'repair' | 'review',
  candidateTree: string,
  patchPayloadSha256: string | null,
): NativeRuntimeEvidenceV2['invocations'][number] {
  const expected = POLICY_V2.base.snapshot.gateContract.nativeControlPlane.hosts
    .find((entry) => entry.host === host)!;
  return {
    invocationId,
    host,
    model: expected.model,
    operation,
    candidateTree,
    environmentDigest: digest('5'),
    outputDigest: digestValue(`output:${invocationId}`),
    patchPayloadSha256,
    exitCode: 0,
    network: {
      enforcement: 'origin-pinned-process-boundary',
      mechanism: 'test-firewall',
      pinnedOrigins: host === 'codex'
        ? ['https://api.openai.com', 'https://chatgpt.com']
        : ['https://api.anthropic.com', 'https://claude.ai'],
      allowedConnections: 1,
      deniedConnections: 0,
      connectDigest: digest('6'),
    },
    filesystem: {
      enforcement: 'os-filesystem-namespace',
      mechanism: 'test-namespace',
      workspaceRootDigest: digest('7'),
      mountManifestDigest: digest('8'),
      configurationMaskDigest: digest('a'),
      outputChannelDigest: digest('9'),
      hostFileConfidentiality: true,
      emptyPrivateHome: true,
      privateEphemeralHome: true,
      hostRootMounted: false,
      hostCredentialPathMounted: false,
      gitMetadataMasked: true,
    },
    resources: {
      enforcement: 'systemd-cgroup-v2',
      mechanism: 'systemd-transient-service',
      limitsDigest: digest('b'),
    },
  };
}

function transitionsFor(
  disposition: Exclude<RepairDisposition, 'none'>,
  sourceCandidate: GitIdentity,
  evaluator: GitIdentity,
  native: NativeRuntimeEvidenceV2,
  patches: readonly string[],
): readonly CandidateRepairTransition[] {
  const trigger = disposition === 'not-started' ? 'patch-admission'
    : disposition === 'failed' ? 'build' : 'verification';
  const drafts = [createCandidateRepairTransitionDraft({
    fromAttempt: 0,
    phase: disposition === 'not-started' ? 'pre-admission' : 'post-admission',
    trigger,
    sourcePatchDigest: patches[0],
    replacementPatchDigest: patches[1],
    sourceCandidate,
    repairResetIdentity: disposition === 'not-started' ? evaluator : null,
    reasons: [`${trigger} rejected`],
    repairInvocationId: 'repair-claude',
  })];
  completeCandidateRepairTransitionReset(drafts, 1, evaluator, patches[1]);
  return sealCandidateRepairTransitions(drafts, native);
}
function toolFixture(policy: typeof POLICY_V2.base): Record<string, string> {
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
    rufloHive: 'hierarchical',
    rufloConsensus: 'raft',
  });
  return tools;
}

function rufloEvidenceFor(receipt: Receipt): ProgrammeV5RufloEvidence {
  return programmeV5RufloFixture({
    taskId: receipt.taskId,
    runId: receipt.runId,
    swarmId: receipt.coordination.swarmId!,
    coordinationTaskId: receipt.coordination.taskId!,
    hookIds: receipt.coordination.hookIds,
    traceIds: receipt.coordination.traceIds,
    routeSnapshotDigest: receipt.route.snapshotDigest,
    capturedAt: receipt.issuedAt,
    transactionStartedAt: receipt.route.frozenAt,
  });
}

function identity(character: string): GitIdentity {
  return { commit: character.repeat(40), tree: character.repeat(40) };
}

function digest(character: string): string {
  return character.repeat(64);
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

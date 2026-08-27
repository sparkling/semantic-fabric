// SPDX-License-Identifier: MIT

import type { AcceptanceTaskV3 } from './acceptance-task.js';
import { SHA256_PATTERN, deepFreeze } from './contracts.js';
import { parseRufloEvidence, type RufloEvidence } from './evidence.js';
import {
  parseMetaHarnessDiagnosticSnapshot,
  type MetaHarnessDiagnosticSnapshot,
  type MetaHarnessDiagnosticTarget,
} from './metaharness-diagnostics.js';
import type {
  ProgrammeDimensionId,
  UpstreamDiagnosticEvidence,
} from './programme-acceptance.js';
import type { ProgrammeGateContractV1 } from './programme-gate-contract-v1.js';
import type { ParsedProgrammePolicyV1 } from './programme-policy-v5.js';
import {
  RED_BASELINE_RECEIPT_KEY,
  receiptArtifactKey,
  receiptGeneratedOutputKey,
  receiptMutationKey,
  receiptQeKey,
  receiptVerifierKey,
} from './programme-receipt-keys.js';
import {
  bindProgrammeTaskRuntimeV1,
  programmeBoundTaskDigestV1,
  programmeCommandReceiptProjectionsV1,
  type ProgrammeCommandReceiptProjectionV1,
} from './programme-task-runtime-v1.js';
import { digestValue, type CommandEvidence, type Receipt } from './receipts.js';
import { resolveTaskEvidencePlanV1 } from './task-evidence-plan.js';
const GENESIS_DIGEST = '0'.repeat(64);
const VERIFIER_STAGES = ['public', 'independent', 'regression'] as const;
export interface ProgrammeDimensionGateV5 {
  readonly id: ProgrammeDimensionId; readonly passed: boolean; readonly evidenceDigest: string;
}

export interface ProgrammeDiagnosticGateV5 extends UpstreamDiagnosticEvidence {
  readonly passed: boolean;
}
export interface ProgrammeGateEvaluationV5 {
  readonly dimensions: readonly ProgrammeDimensionGateV5[];
  readonly diagnostics: readonly ProgrammeDiagnosticGateV5[];
}
export function evaluateProgrammeGatesV5(input: Readonly<{
  policy: ParsedProgrammePolicyV1;
  receipt: Receipt;
  diagnostics: MetaHarnessDiagnosticSnapshot;
  rufloEvidence: RufloEvidence;
}>): ProgrammeGateEvaluationV5 {
  const { policy, receipt } = input;
  const snapshot = policy.snapshot;
  const gate = snapshot.gateContract;
  if (!nonnegativeInteger(receipt.recovery.repairCount)
    || receipt.recovery.repairCount > gate.attempts.maximumRepairs) {
    throw new Error('HARNESS_PROGRAMME_REPAIR_COUNT_EXCEEDS_POLICY');
  }
  const ruflo = parseRufloEvidence(input.rufloEvidence);
  const diagnostics = parseMetaHarnessDiagnosticSnapshot(input.diagnostics);
  assertDiagnosticOrder(diagnostics, gate);

  const receiptIntegrity = validReceiptIntegrity(receipt, gate);
  const policyBindings = validPolicyBindings(policy, receipt);
  const protectedInputs = sameRecord(receipt.protectedInputs, snapshot.execution.protectedInputs)
    && Object.values(receipt.protectedInputs).every(validDigest);
  const tools = validTools(policy, receipt);
  const route = receipt.route.routerVersion === gate.route.routerVersion
    && validDigest(receipt.route.snapshotDigest)
    && receipt.route.snapshotDigest === snapshot.execution.routeSnapshotDigest
    && digestValue(policy.routeSnapshot) === snapshot.execution.routeSnapshotDigest
    && canonicalTimestamp(receipt.route.frozenAt);
  const commands = evaluateCommands(policy.task, receipt);
  const keys = evaluateEvidenceKeys(policy, receipt, commands.finalMutationCommands);
  const evolution = validEvolutionPolicy(policy);
  const candidate = validCandidate(policy.task, receipt);
  const native = validNativeControlPlane(receipt, gate);
  const reliability = receiptIntegrity
    && receipt.recovery.cancelled === gate.reliability.cancelled
    && receipt.recovery.breakerState === gate.reliability.breakerState
    && nonnegativeInteger(receipt.recovery.retryCount)
    && commands.completed && commands.attemptHistory;
  const rufloGate = validRufloBinding(receipt, ruflo)
    && route
    && nonempty(receipt.toolVersions.rufloHive)
    && nonempty(receipt.toolVersions.rufloConsensus)
    && keys.qe;

  const checks: Record<ProgrammeDimensionId, Readonly<Record<string, boolean>>> = {
    'policy-and-supply-chain-safety': { policyBindings, protectedInputs, tools, route },
    'evaluator-integrity': {
      identities: validIdentities(policy, receipt),
      redCommands: commands.red,
      redDigest: keys.red,
      independentVerifier: keys.finalVerifiers,
    },
    'evolution-containment': { verifierOnlyTask: evolution },
    'patched-candidate-verification': {
      candidate,
      commandProjections: commands.allDeclared, finalBuild: commands.finalBuild,
      finalMutation: commands.finalMutation, artifacts: keys.artifacts,
      verifierAllowlist: keys.allowlist, finalVerifiers: keys.finalVerifiers,
      generatedOutputs: keys.generated, mutations: keys.mutations,
    },
    'dual-host-control-plane': { native },
    'reliability-and-receipts': { reliability },
    'ruflo-and-qe-integration': { ruflo: rufloGate, qe: keys.qe },
  };
  const replayBinding = {
    receiptDigest: receipt.digest, policyFingerprint: policy.fingerprint,
    rufloEvidenceDigest: digestValue(ruflo),
  };
  const dimensions = gate.acceptance.dimensions.map(({ id }) => {
    const evidence = { schemaVersion: 1, id, ...replayBinding, checks: checks[id] };
    return {
      id,
      passed: Object.values(checks[id]).every(Boolean),
      evidenceDigest: digestValue(evidence),
    };
  });
  return deepFreeze({
    dimensions,
    diagnostics: gate.diagnostics.targets.map((expected) =>
      diagnosticGate(diagnostics.targets.find(({ target }) => target === expected.target)!, expected)),
  });
}
function validReceiptIntegrity(receipt: Receipt, gate: ProgrammeGateContractV1): boolean {
  const { digest, ...body } = receipt;
  return receipt.schemaVersion === gate.receiptSchemaVersion
    && receipt.sequence === gate.receipt.sequence
    && receipt.previousDigest === gate.receipt.previousDigest
    && validDigest(digest) && digestValue(body) === digest
    && receipt.step === gate.receipt.step
    && receipt.status === gate.receipt.status
    && receipt.failureCode === gate.receipt.failureCode
    && receipt.authority === gate.receipt.authority
    && canonicalTimestamp(receipt.issuedAt)
    && nonnegativeInteger(receipt.recovery.repairCount)
    && receipt.recovery.repairCount <= gate.attempts.maximumRepairs
    && receipt.patchDigests.length === receipt.recovery.repairCount + 1
    && receipt.patchDigests.every(validDigest)
    && receipt.patchDigest !== null && validDigest(receipt.patchDigest)
    && receipt.patchDigests.at(-1) === receipt.patchDigest;
}

function validPolicyBindings(policy: ParsedProgrammePolicyV1, receipt: Receipt): boolean {
  const { snapshot } = policy;
  const tools = receipt.toolVersions;
  return receipt.taskId === policy.task.taskId
    && tools.bootstrapControllerStoreDigest === snapshot.bootstrap.controllerStoreDigest
    && tools.bootstrapBuildManifestDigest === snapshot.controller.buildManifestBlobDigest
    && tools.bootstrapRuntimeTreeDigest === snapshot.controller.runtimeTreeDigest
    && tools.bootstrapNodeDigest === snapshot.bootstrap.nodeDigest
    && tools.bootstrapGitDigest === snapshot.bootstrap.gitDigest
    && tools.controllerExecutionDigest === snapshot.controller.executionDigest
    && tools.controllerBuildManifestDigest === snapshot.controller.buildManifestBlobDigest
    && tools.controllerRuntimeTreeDigest === snapshot.controller.runtimeTreeDigest
    && tools.controllerManifestDigest === snapshot.controller.manifestBlobDigest
    && tools.controllerTaskPath === snapshot.controller.taskPath
    && tools.controllerTaskPathDigest === digestValue(snapshot.controller.taskPath)
    && tools.controllerTaskDigest === snapshot.controller.taskBlobDigest
    && tools.taskEvidencePlanDigest === snapshot.taskEvidencePlanDigest
    && tools.boundTaskDigest === snapshot.execution.boundTaskDigest
    && tools.programmePolicyFingerprint === policy.fingerprint
    && tools.frozenCargoLockDigest === policy.task.rust.frozenLockSha256;
}

function validTools(policy: ParsedProgrammePolicyV1, receipt: Receipt): boolean {
  const gate = policy.snapshot.gateContract.tools;
  const tools = receipt.toolVersions;
  const exactKeys = sameStrings(Object.keys(tools).sort(), [...gate.requiredKeys].sort());
  const exactValues = Object.entries(gate.exactValues)
    .every(([key, value]) => tools[key] === value);
  const digestValues = gate.digestValueKeys.every((key) => validDigest(tools[key]));
  const nonemptyValues = gate.nonEmptyValueKeys.every((key) => nonempty(tools[key]));
  const rules = gate.structuredValueRules;
  return exactKeys && exactValues && digestValues && nonemptyValues
    && structuredTriple(tools.rustToolchainClosure, rules.rustToolchainClosure.leadingValue)
    && structuredTriple(tools.rustRegistryBootstrapSnapshot, undefined, true)
    && structuredTriple(tools.rustRegistryClosure, rules.rustRegistryClosure.leadingValue)
    && structuredTriple(tools.rustRegistryLock, policy.task.rust.frozenLockSha256)
    && structuredPair(tools.rustRegistrySelection, rules.rustRegistrySelection.leadingValue);
}

function validIdentities(policy: ParsedProgrammePolicyV1, receipt: Receipt): boolean {
  return sameValue(receipt.identities.controller, policy.snapshot.controller.identity)
    && sameValue(receipt.identities.baseline, policy.task.baseline)
    && sameValue(receipt.identities.evaluator, policy.snapshot.execution.evaluator)
    && /^[a-f0-9]{40,64}$/.test(receipt.identities.candidate.commit)
    && /^[a-f0-9]{40,64}$/.test(receipt.identities.candidate.tree)
    && receipt.identities.candidate.tree !== receipt.identities.evaluator.tree;
}

function validEvolutionPolicy(policy: ParsedProgrammePolicyV1): boolean {
  try {
    const plan = resolveTaskEvidencePlanV1({
      task: policy.boundTask,
      taskPath: policy.snapshot.controller.taskPath,
    });
    return policy.task.schemaVersion === policy.snapshot.gateContract.taskSchemaVersion
      && policy.task.candidateOracle.mode === 'verifier-only'
      && policy.boundTask.schemaVersion === 3
      && policy.boundTask.candidateOracle.mode === 'verifier-only'
      && programmeBoundTaskDigestV1(policy.task) === policy.snapshot.execution.boundTaskDigest
      && sameValue(policy.boundTask, bindProgrammeTaskRuntimeV1(policy.task))
      && plan.declarationDigest === policy.snapshot.taskEvidencePlanDigest
      && sameValue(plan, policy.evidencePlan);
  } catch {
    return false;
  }
}

function validCandidate(task: AcceptanceTaskV3, receipt: Receipt): boolean {
  return receipt.status === 'pass'
    && task.candidateOracle.mode === 'verifier-only'
    && sameStrings(receipt.admittedPaths, task.evidence.requiredAdmittedPaths)
    && receipt.patchDigest !== null && validDigest(receipt.patchDigest);
}

interface CommandEvaluation {
  red: boolean; finalBuild: boolean; finalMutation: boolean;
  allDeclared: boolean; completed: boolean; attemptHistory: boolean;
  finalMutationCommands: readonly CommandEvidence[];
}

function evaluateCommands(task: AcceptanceTaskV3, receipt: Receipt): CommandEvaluation {
  const projections = programmeCommandReceiptProjectionsV1(task);
  const expected = (stage: ProgrammeCommandReceiptProjectionV1['stage']) =>
    projections.filter((entry) => entry.stage === stage);
  const finalAttempt = receipt.recovery.repairCount;
  const red = receipt.commands.filter(({ stage }) => stage === 'red-baseline');
  const finalBuild = receipt.commands.filter((command) =>
    command.stage === 'build' && command.attempt === finalAttempt);
  const finalMutation = receipt.commands.filter((command) =>
    command.stage === 'mutation' && command.attempt === finalAttempt);
  const completed = receipt.commands.every(normalCompletion);
  const allDeclared = receipt.commands.every((command) =>
    nonnegativeInteger(command.attempt) && command.attempt <= finalAttempt
    && (command.stage === 'red-baseline' || command.stage === 'build' || command.stage === 'mutation')
    && expected(command.stage).some((projection) => sameProjection(command, projection)));
  return {
    red: sameCommandMultiset(red, expected('red-baseline'))
      && red.every((command) => command.attempt === 0
        && command.candidateTree === receipt.identities.evaluator.tree
        && command.exitCode === 101 && normalCompletion(command)),
    finalBuild: sameCommandMultiset(finalBuild, expected('build'))
      && finalBuild.every((command) => command.candidateTree === receipt.identities.candidate.tree
        && command.exitCode === 0 && normalCompletion(command)),
    finalMutation: sameCommandMultiset(finalMutation, expected('mutation'))
      && finalMutation.every((command) => command.candidateTree === receipt.identities.candidate.tree
        && command.exitCode === 101 && normalCompletion(command)),
    allDeclared,
    completed,
    attemptHistory: priorBuildHistory(receipt, expected('build')),
    finalMutationCommands: matchCommands(finalMutation, expected('mutation')) ?? [],
  };
}
function priorBuildHistory(
  receipt: Receipt,
  expected: readonly ProgrammeCommandReceiptProjectionV1[],
): boolean {
  for (let attempt = 0; attempt < receipt.recovery.repairCount; attempt += 1) {
    const actual = receipt.commands.filter((command) =>
      command.stage === 'build' && command.attempt === attempt);
    if (actual.length === 0 || actual.length > expected.length
      || actual.some((command, index) => !sameProjection(command, expected[index]))) return false;
    const completeSuccess = actual.length === expected.length
      && actual.every((command) => command.exitCode === 0);
    const lastExit = actual.at(-1)?.exitCode;
    const failedPrefix = actual.slice(0, -1).every((command) => command.exitCode === 0)
      && Number.isSafeInteger(lastExit) && lastExit !== 0;
    if (!completeSuccess && !failedPrefix) return false;
  }
  return true;
}
function evaluateEvidenceKeys(
  policy: ParsedProgrammePolicyV1,
  receipt: Receipt,
  finalMutations: readonly CommandEvidence[],
) {
  const task = policy.task;
  const finalAttempt = receipt.recovery.repairCount;
  const artifactAllowed = new Set<string>();
  const verifierAllowed = new Set<string>([RED_BASELINE_RECEIPT_KEY]);
  for (let attempt = 0; attempt <= finalAttempt; attempt += 1) {
    task.artifactPaths.forEach((path) => artifactAllowed.add(receiptArtifactKey(attempt, path)));
    VERIFIER_STAGES.forEach((stage) => verifierAllowed.add(receiptVerifierKey(attempt, stage)));
    task.evidence.generatedOutputs.forEach(({ stage, evidenceId }) =>
      verifierAllowed.add(receiptGeneratedOutputKey(attempt, stage, evidenceId)));
    task.commands.mutation.forEach(({ mutationId }) =>
      verifierAllowed.add(receiptMutationKey(attempt, mutationId)));
    policy.evidencePlan.requiredQeProfiles.forEach((profile) =>
      verifierAllowed.add(receiptQeKey(attempt, profile)));
  }
  const finalArtifacts = task.artifactPaths.map((path) => receiptArtifactKey(finalAttempt, path));
  const finalVerifiers = VERIFIER_STAGES.map((stage) => receiptVerifierKey(finalAttempt, stage));
  const generated = task.evidence.generatedOutputs.map(({ stage, evidenceId }) =>
    receiptGeneratedOutputKey(finalAttempt, stage, evidenceId));
  const mutationKeys = task.commands.mutation.map(({ mutationId }) =>
    receiptMutationKey(finalAttempt, mutationId));
  const qeKeys = policy.evidencePlan.requiredQeProfiles.map((profile) =>
    receiptQeKey(finalAttempt, profile));
  const redCommands = receipt.commands.filter(({ stage }) => stage === 'red-baseline');
  const red = receipt.verifierDigests[RED_BASELINE_RECEIPT_KEY] === digestValue({
    expected: task.redBaseline.expected, commands: redCommands,
  });
  const mutations = mutationKeys.every((key, index) => {
    const entry = task.commands.mutation[index];
    const command = finalMutations[index];
    return command !== undefined && receipt.verifierDigests[key] === digestValue({
      mutationId: entry.mutationId,
      path: entry.path,
      searchDigest: digestValue(entry.search),
      replacementDigest: digestValue(entry.replacement),
      command,
    });
  });
  const allVerifierDigests = Object.values(receipt.verifierDigests).every(validDigest);
  const allArtifactDigests = Object.values(receipt.artifactDigests).every(validDigest);
  return {
    red,
    artifacts: exactAllowed(receipt.artifactDigests, artifactAllowed, finalArtifacts)
      && allArtifactDigests && finalArtifacts.length >= 1,
    allowlist: exactAllowed(receipt.verifierDigests, verifierAllowed, [
      RED_BASELINE_RECEIPT_KEY, ...finalVerifiers, ...generated, ...mutationKeys, ...qeKeys,
    ]) && allVerifierDigests,
    finalVerifiers: finalVerifiers.every((key) => validDigest(receipt.verifierDigests[key])),
    generated: generated.every((key) => validDigest(receipt.verifierDigests[key])),
    mutations,
    qe: qeKeys.every((key) => validDigest(receipt.verifierDigests[key]))
      && sameStrings(receipt.coordination.agenticQeEvidenceDigests,
        qeKeys.map((key) => receipt.verifierDigests[key])),
  };
}

function validNativeControlPlane(receipt: Receipt, gate: ProgrammeGateContractV1): boolean {
  const native = gate.nativeControlPlane;
  return sameValue(receipt.hosts, native.hosts)
    && receipt.hosts[0]?.clientVersion === receipt.toolVersions.codex
    && receipt.hosts[1]?.clientVersion === receipt.toolVersions.claude
    && receipt.critiqueDigests.length >= native.minimumCritiqueDigests
    && receipt.critiqueDigests.every(validDigest)
    && receipt.reviewDigests.length === native.finalReviewDigests
    && uniqueDigests(receipt.reviewDigests)
    && receipt.coordination.nativeEvidenceDigests.length >= native.minimumNativeEvidenceDigests
    && uniqueDigests(receipt.coordination.nativeEvidenceDigests)
    && receipt.coordination.nativeRuntimeEvidenceDigest !== null
    && validDigest(receipt.coordination.nativeRuntimeEvidenceDigest);
}

function validRufloBinding(receipt: Receipt, ruflo: RufloEvidence): boolean {
  return ruflo.taskId === receipt.taskId
    && ruflo.runId === receipt.runId
    && ruflo.routeSnapshotDigest === receipt.route.snapshotDigest
    && ruflo.swarmId === receipt.coordination.swarmId
    && ruflo.coordinationTaskId === receipt.coordination.taskId
    && sameStrings(ruflo.hookIds, receipt.coordination.hookIds)
    && sameStrings(ruflo.traceIds, receipt.coordination.traceIds);
}

function assertDiagnosticOrder(
  diagnostics: MetaHarnessDiagnosticSnapshot,
  gate: ProgrammeGateContractV1,
): void {
  if (!sameStrings(
    diagnostics.targets.map(({ target }) => target),
    gate.diagnostics.targets.map(({ target }) => target),
  )) throw new Error('HARNESS_PROGRAMME_DIAGNOSTIC_ORDER_INVALID');
}

function diagnosticGate(
  target: MetaHarnessDiagnosticTarget,
  expected: ProgrammeGateContractV1['diagnostics']['targets'][number],
): ProgrammeDiagnosticGateV5 {
  const evidence: UpstreamDiagnosticEvidence = {
    target: target.target,
    implementation: 'metaharness@0.3.2',
    success: target.success,
    degraded: target.degraded,
    exitCode: target.exitCode,
    scaffoldReady: target.scaffoldReady,
    hardConstraintsPassed: target.hardConstraintsPassed,
    hardConstraintsTotal: target.hardConstraintsTotal,
    harnessFit: target.harnessFit,
    evidenceDigest: digestValue(target),
  };
  return {
    ...evidence,
    passed: target.target === expected.target
      && target.repositoryPath === expected.repositoryPath
      && target.schema === expected.schema
      && target.hardConstraintsTotal === expected.hardConstraintsTotal
      && target.success && !target.degraded && target.exitCode === 0
      && target.scaffoldReady
      && target.hardConstraintsPassed === target.hardConstraintsTotal,
  };
}

function sameCommandMultiset(
  actual: readonly CommandEvidence[],
  expected: readonly ProgrammeCommandReceiptProjectionV1[],
): boolean {
  return matchCommands(actual, expected) !== null;
}

function matchCommands(
  actual: readonly CommandEvidence[],
  expected: readonly ProgrammeCommandReceiptProjectionV1[],
): CommandEvidence[] | null {
  if (actual.length !== expected.length) return null;
  const unused = [...actual];
  const ordered: CommandEvidence[] = [];
  for (const projection of expected) {
    const index = unused.findIndex((command) => sameProjection(command, projection));
    if (index < 0) return null;
    ordered.push(unused.splice(index, 1)[0]);
  }
  return ordered;
}

function sameProjection(
  command: CommandEvidence,
  expected: ProgrammeCommandReceiptProjectionV1,
): boolean {
  return command.stage === expected.stage && command.tool === expected.tool
    && command.executable === expected.executable && command.cwd === expected.cwd
    && sameStrings(command.argv, expected.argv);
}

function normalCompletion(command: CommandEvidence): boolean {
  return command.signal === null && !command.timedOut && !command.cancelled
    && !command.outputLimitExceeded && command.spawnErrorDigest === null;
}

function exactAllowed(
  values: Readonly<Record<string, string>>,
  allowed: ReadonlySet<string>,
  required: readonly string[],
): boolean {
  const keys = Object.keys(values);
  return keys.every((key) => allowed.has(key)) && required.every((key) => key in values);
}

function structuredTriple(value: string, leading?: string, leadingDigest = false): boolean {
  const [first, second, third, extra] = String(value).split(':');
  return extra === undefined && (leadingDigest ? validDigest(first) : first === leading)
    && positiveIntegerText(second) && positiveIntegerText(third);
}

function structuredPair(value: string, leading: string): boolean {
  const [first, second, extra] = String(value).split(':');
  return extra === undefined && first === leading && validDigest(second);
}

function positiveIntegerText(value: string): boolean {
  return /^[1-9][0-9]*$/.test(value) && Number.isSafeInteger(Number(value));
}

function validDigest(value: unknown): value is string {
  return typeof value === 'string' && SHA256_PATTERN.test(value) && value !== GENESIS_DIGEST;
}

function uniqueDigests(values: readonly string[]): boolean {
  return values.every(validDigest) && new Set(values).size === values.length;
}

function nonnegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function nonempty(value: unknown): value is string {
  return typeof value === 'string' && value.trim() !== '';
}

function canonicalTimestamp(value: string): boolean {
  const date = new Date(value);
  return Number.isFinite(date.valueOf()) && date.toISOString() === value;
}

function sameStrings(actual: readonly string[], expected: readonly string[]): boolean {
  return actual.length === expected.length && actual.every((value, index) => value === expected[index]);
}

function sameRecord(
  actual: Readonly<Record<string, string>>,
  expected: Readonly<Record<string, string>>,
): boolean {
  return sameStrings(Object.keys(actual).sort(), Object.keys(expected).sort())
    && Object.entries(expected).every(([key, value]) => actual[key] === value);
}

function sameValue(actual: unknown, expected: unknown): boolean {
  return digestValue(actual) === digestValue(expected);
}

// SPDX-License-Identifier: MIT
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parseAcceptanceTask } from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { parseHarnessConfig, type HarnessConfig } from '../src/contracts.js';
import { HARNESS_MANIFEST_PATH } from '../src/controller-attestation.js';
import { CONTROLLER_BUILD_PATH } from '../src/controller-build.js';
import type { ProgrammeV5RufloEvidence } from '../src/evidence.js';
import { METAHARNESS_DIAGNOSTICS_PATH } from '../src/metaharness-diagnostics.js';
import {
  createProgrammeEnvelopeV5,
  finalizeProgrammeOutcomeV5,
  parseProgrammeEnvelopeV5,
  serializeProgrammeEnvelopeV5,
  type ProgrammeEnvelopeV5,
} from '../src/programme-envelope-v5.js';
import { parseProgrammeEnvelope, serializeProgrammeEnvelope } from '../src/programme-envelope.js';
import {
  createFrozenProgrammePolicyV1,
  programmePolicyFingerprint,
  verifyFrozenProgrammePolicyV1,
  type ControllerPolicyInputs,
  type FrozenProgrammePolicyV1,
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
  bindProgrammeTaskRuntimeV1,
  programmeCommandReceiptProjectionsV1,
  type ProgrammeCommandReceiptProjectionV1,
} from '../src/programme-task-runtime-v1.js';
import { digestValue, ReceiptChain, type Receipt, type ReceiptStatus } from '../src/receipts.js';
import { resolveTaskEvidencePlanV1 } from '../src/task-evidence-plan.js';
import { diagnosticBlob, diagnosticBlobDigest, programmeV5RufloFixture }
  from './candidate-fixtures.js';
import { PROGRAMME_V5_POST_HISTORICAL_PATHS }
  from './programme-v5-post-historical-paths.js';
const taskPath = 'coding-harness/config/issue-8-acceptance.json';
const POLICY_FINGERPRINT = 'aa136fe4b027d39667669b17553a63e0155bcde28d36859f0dead245c23138e2';
const ACCEPTANCE_DIGEST = 'a18d6c18397c1e145b75cf88c015f9a646694bbe6c72e398447609ed2be8ffce';
const ENVELOPE_DIGEST = '3a6807125d22ac51f43b9f6d1a5453aa982bd8262eb8aee63886e5ab590aa045';
const HISTORICAL_POLICY_FINGERPRINT = '3f6481bd336a59bbda3e9f475adb88551f1650d0be55b0e398c1ec384fcfe59d';
const HISTORICAL_ACCEPTANCE_DIGEST = '480103f3d9876b67e4a1bb2a48909240b4ca0d14b0a3917d2bb20db757b402ee';
const HISTORICAL_ENVELOPE_DIGEST = '7b3de3ef1b02c6b4558bed6203a09b2f730a2df30e0b02c6bb45235901bc2031';

describe('strict schema-v5 programme envelope', () => {
  it('round-trips one receipt against independent policy and envelope anchors', () => {
    const fixture = envelopeFixture('pass');
    expect(programmePolicyFingerprint(fixture.policy)).toBe(POLICY_FINGERPRINT);
    const created = createProgrammeEnvelopeV5(fixture.input, POLICY_FINGERPRINT);
    const serialized = serializeProgrammeEnvelopeV5(created, POLICY_FINGERPRINT);
    const parsed = parseProgrammeEnvelopeV5(serialized, POLICY_FINGERPRINT);

    expect(parsed).toEqual(created);
    expect(parseProgrammeEnvelope(serialized, {
      schemaVersion: 5, policyFingerprint: POLICY_FINGERPRINT,
    })).toEqual(created);
    expect(serializeProgrammeEnvelope(created, {
      schemaVersion: 5, policyFingerprint: POLICY_FINGERPRINT,
    })).toBe(serialized);
    expect(parsed.envelopeDigest).toBe(ENVELOPE_DIGEST);
    expect(parsed.programmeAcceptanceDigest).toBe(ACCEPTANCE_DIGEST);
    expect(parsed.receiptChain.receipts).toHaveLength(1);
    expect(parsed.programmeAcceptance).toMatchObject({
      receiptDigest: fixture.receipt.digest,
      score: 100,
      status: 'ACCEPTED',
      fitnessEligible: false,
    });
    for (const value of [
      parsed, parsed.policy, parsed.rufloEvidence, parsed.receiptChain,
      parsed.receiptChain.receipts, parsed.programmeAcceptance,
    ]) expect(Object.isFrozen(value)).toBe(true);
  });

  it('preserves the pre-capture V5 policy, acceptance, and envelope anchors', () => {
    const { manifestBlob, harnessConfig } = historicalManifest();
    const policy = policyFixture(manifestBlob, harnessConfig);
    const fixture = envelopeFixture('pass', true, policy);
    expect(programmePolicyFingerprint(policy)).toBe(HISTORICAL_POLICY_FINGERPRINT);
    const envelope = createProgrammeEnvelopeV5(fixture.input, HISTORICAL_POLICY_FINGERPRINT);
    expect(envelope.programmeAcceptanceDigest).toBe(HISTORICAL_ACCEPTANCE_DIGEST);
    expect(envelope.envelopeDigest).toBe(HISTORICAL_ENVELOPE_DIGEST);
    expect(parseProgrammeEnvelopeV5(
      serializeProgrammeEnvelopeV5(envelope, HISTORICAL_POLICY_FINGERPRINT),
      HISTORICAL_POLICY_FINGERPRINT,
    )).toEqual(envelope);
  });

  it('requires the external fingerprint on create, parse, serialize, and finalize', () => {
    const fixture = envelopeFixture('fail');
    const envelope = createProgrammeEnvelopeV5(fixture.input, POLICY_FINGERPRINT);
    const serialized = JSON.stringify(envelope);
    const wrong = 'f'.repeat(64);

    expect(() => createProgrammeEnvelopeV5(fixture.input, wrong)).toThrow(/FINGERPRINT/);
    expect(() => parseProgrammeEnvelopeV5(serialized, wrong)).toThrow(/FINGERPRINT/);
    expect(() => serializeProgrammeEnvelopeV5(envelope, wrong)).toThrow(/FINGERPRINT/);
    expect(() => finalizeProgrammeOutcomeV5({
      expectedPolicyFingerprint: wrong,
      transactionStatus: 'fail',
      transactionReason: 'HARNESS_TRANSACTION_FAILED',
      envelope,
    })).toThrow(/FINGERPRINT/);
    expect(() => (createProgrammeEnvelopeV5 as any)(fixture.input)).toThrow(/ANCHOR/);
    expect(() => (parseProgrammeEnvelopeV5 as any)(serialized)).toThrow(/ANCHOR/);
    expect(() => (serializeProgrammeEnvelopeV5 as any)(envelope)).toThrow(/ANCHOR/);
    expect(() => finalizeProgrammeOutcomeV5({
      transactionStatus: 'fail', transactionReason: 'HARNESS_TRANSACTION_FAILED', envelope,
    } as any)).toThrow();
  });

  it('rejects an internally rehashed policy self-anchor', () => {
    const fixture = envelopeFixture('fail');
    const envelope = createProgrammeEnvelopeV5(fixture.input, POLICY_FINGERPRINT);
    const attack = structuredClone(envelope) as any;
    attack.policy.controller.identity.commit = 'e'.repeat(40);
    attack.policyFingerprint = programmePolicyFingerprint(attack.policy);
    rehashEnvelope(attack);

    expect(attack.policyFingerprint).not.toBe(POLICY_FINGERPRINT);
    expect(() => parseProgrammeEnvelopeV5(JSON.stringify(attack), POLICY_FINGERPRINT))
      .toThrow(/FINGERPRINT/);
  });

  it('recomputes every embedded evidence and digest layer', () => {
    const fixture = envelopeFixture('fail');
    const envelope = createProgrammeEnvelopeV5(fixture.input, POLICY_FINGERPRINT);
    const attacks: Array<(value: any) => void> = [
      (value) => { value.policyFingerprint = 'e'.repeat(64); },
      (value) => { value.rufloEvidence.swarmId = 'tampered-swarm'; },
      (value) => { value.rufloEvidenceDigest = 'e'.repeat(64); },
      (value) => { value.receiptChain.receipts[0].runId = 'tampered-run'; },
      (value) => { value.receiptChain.schemaVersion = 2; },
      (value) => { value.receiptChain.receipts = []; },
      (value) => { value.receiptChain.receipts.push(value.receiptChain.receipts[0]); },
      (value) => { value.diagnosticBlob += ' '; },
      (value) => { value.diagnosticBlobDigest = 'e'.repeat(64); },
      (value) => { value.programmeAcceptance.score += 1; },
      (value) => { value.programmeAcceptanceDigest = 'e'.repeat(64); },
      (value) => { value.envelopeDigest = 'e'.repeat(64); },
      (value) => { value.authority = 'promotion'; },
      (value) => { value.extra = true; },
    ];
    for (const attack of attacks) {
      const tampered = structuredClone(envelope) as any;
      attack(tampered);
      expect(() => parseProgrammeEnvelopeV5(JSON.stringify(tampered), POLICY_FINGERPRINT)).toThrow();
    }

    const rescored = structuredClone(envelope) as any;
    rescored.programmeAcceptance.dimensions[0].verifiedPoints = 1;
    rescored.programmeAcceptanceDigest = digestValue(rescored.programmeAcceptance);
    rehashEnvelope(rescored);
    expect(() => parseProgrammeEnvelopeV5(JSON.stringify(rescored), POLICY_FINGERPRINT)).toThrow();
  });

  it('rejects schema downgrade and escaped or nested duplicate keys', () => {
    const fixture = envelopeFixture('fail');
    const envelope = createProgrammeEnvelopeV5(fixture.input, POLICY_FINGERPRINT);
    const serialized = serializeProgrammeEnvelopeV5(envelope, POLICY_FINGERPRINT);
    const duplicates = [
      serialized.replace('"schemaVersion": 5', '"schemaVersion":5,"schema\\u0056ersion":5'),
      serialized.replace('"policyId":', '"policyId":"shadow","policy\\u0049d":'),
      serialized.replace('"runId":', '"runId":"shadow","run\\u0049d":'),
    ];
    for (const duplicate of duplicates) {
      expect(() => parseProgrammeEnvelopeV5(duplicate, POLICY_FINGERPRINT)).toThrow(/duplicate/);
    }
    const downgraded = structuredClone(envelope) as any;
    downgraded.schemaVersion = 4;
    expect(() => parseProgrammeEnvelopeV5(JSON.stringify(downgraded), POLICY_FINGERPRINT)).toThrow();

    const duplicateDiagnostic = fixture.input.diagnosticBlob.replace(
      '"source":',
      '"source":"shadow","source":',
    );
    expect(() => createProgrammeEnvelopeV5({
      ...fixture.input, diagnosticBlob: duplicateDiagnostic,
    }, POLICY_FINGERPRINT)).toThrow(/duplicate/);
  });

  it('revalidates final outcomes and gates a passing receipt with rejected assessment', () => {
    const accepted = envelopeFixture('pass');
    const acceptedEnvelope = createProgrammeEnvelopeV5(accepted.input, POLICY_FINGERPRINT);
    expect(finalizeProgrammeOutcomeV5({
      expectedPolicyFingerprint: POLICY_FINGERPRINT,
      transactionStatus: 'pass', transactionReason: null, envelope: acceptedEnvelope,
    })).toEqual({ status: 'pass', reason: null });

    const failed = envelopeFixture('fail');
    const failedEnvelope = createProgrammeEnvelopeV5(failed.input, POLICY_FINGERPRINT);
    expect(finalizeProgrammeOutcomeV5({
      expectedPolicyFingerprint: POLICY_FINGERPRINT,
      transactionStatus: 'fail',
      transactionReason: 'HARNESS_TRANSACTION_FAILED: detail',
      envelope: failedEnvelope,
    })).toEqual({ status: 'fail', reason: 'HARNESS_TRANSACTION_FAILED' });

    const passing = envelopeFixture('pass', false);
    const passingEnvelope = createProgrammeEnvelopeV5(passing.input, POLICY_FINGERPRINT);
    expect(passingEnvelope.programmeAcceptance.status).toBe('REJECTED');
    expect(finalizeProgrammeOutcomeV5({
      expectedPolicyFingerprint: POLICY_FINGERPRINT,
      transactionStatus: 'pass', transactionReason: null, envelope: passingEnvelope,
    })).toEqual({ status: 'gated', reason: 'HARNESS_PROGRAMME_ACCEPTANCE_REJECTED' });
    expect(() => finalizeProgrammeOutcomeV5({
      expectedPolicyFingerprint: POLICY_FINGERPRINT,
      transactionStatus: 'pass', transactionReason: 'unexpected', envelope: passingEnvelope,
    })).toThrow();
  });
});

function envelopeFixture(
  status: ReceiptStatus,
  gateValid = true,
  policy = policyFixture(),
) {
  const rufloEvidence = programmeV5RufloFixture({
    taskId: 'verifier_only_task_0001', runId: 'programme-run-0001',
    swarmId: 'swarm-0001', coordinationTaskId: 'coordination-task-0001',
    hookIds: ['hook-route-0001'], traceIds: ['trace-0001'],
    routeSnapshotDigest: policy.execution.routeSnapshotDigest,
    capturedAt: '2026-08-27T08:00:00.000Z',
  });
  const receipt = receiptFixture(policy, rufloEvidence, status, gateValid);
  return {
    policy,
    receipt,
    input: { policy, rufloEvidence, receipt, diagnosticBlob },
  };
}

function receiptFixture(
  policy: FrozenProgrammePolicyV1,
  ruflo: ProgrammeV5RufloEvidence,
  status: ReceiptStatus,
  gateValid: boolean,
): Receipt {
  const task = parseAcceptanceTask(JSON.parse(policy.controller.taskBlob), SECURE_HARNESS_CONFIG);
  if (task.schemaVersion !== 3) throw new Error('test task must be schema v3');
  const bound = bindProgrammeTaskRuntimeV1(task);
  const candidate = { commit: 'c'.repeat(40), tree: 'd'.repeat(40) };
  const pass = status === 'pass';
  const projections = programmeCommandReceiptProjectionsV1(bound);
  const commands = pass ? projections.filter(({ stage }) =>
    stage === 'red-baseline' || stage === 'build' || stage === 'mutation')
    .map((projection) => commandReceipt(
      projection, candidate.tree, policy.execution.evaluator.tree,
    )) : [];
  const red = commands.filter(({ stage }) => stage === 'red-baseline');
  const mutation = commands.filter(({ stage }) => stage === 'mutation');
  const qeDigests = bound.qe.profiles.map(({ profile }) => digestValue(profile));
  const verifiers: Record<string, string> = pass ? {
    [RED_BASELINE_RECEIPT_KEY]: digestValue({ expected: bound.redBaseline.expected, commands: red }),
    [receiptVerifierKey(0, 'public')]: digestValue('public'),
    [receiptVerifierKey(0, 'independent')]: digestValue('independent'),
    [receiptVerifierKey(0, 'regression')]: digestValue('regression'),
  } : {};
  if (pass) {
    bound.evidence.generatedOutputs.forEach(({ stage, evidenceId }) => {
      verifiers[receiptGeneratedOutputKey(0, stage, evidenceId)] = digestValue(evidenceId);
    });
    bound.commands.mutation.forEach((entry, index) => {
      verifiers[receiptMutationKey(0, entry.mutationId)] = digestValue({
        mutationId: entry.mutationId, path: entry.path,
        searchDigest: digestValue(entry.search), replacementDigest: digestValue(entry.replacement),
        command: mutation[index],
      });
    });
    bound.qe.profiles.forEach(({ profile }, index) => {
      verifiers[receiptQeKey(0, profile)] = qeDigests[index]!;
    });
  }
  const chain = new ReceiptChain();
  return chain.append({
    schemaVersion: 3, runId: ruflo.runId, taskId: task.taskId,
    step: 'candidate-transaction', status,
    failureCode: pass ? null : 'HARNESS_TRANSACTION_FAILED',
    authority: 'development-only-no-promotion', issuedAt: '2026-08-27T08:01:00.000Z',
    identities: {
      controller: policy.controller.identity, baseline: task.baseline,
      evaluator: policy.execution.evaluator, candidate,
    },
    protectedInputs: { ...policy.execution.protectedInputs },
    route: {
      snapshotDigest: policy.execution.routeSnapshotDigest,
      frozenAt: '2026-08-27T08:00:00.000Z', routerVersion: '@metaharness/router@0.4.0',
    },
    hosts: pass ? policy.gateContract.nativeControlPlane.hosts.map((host) => ({ ...host })) : [],
    admittedPaths: pass ? [...task.evidence.requiredAdmittedPaths] : [],
    patchDigest: pass ? digest('8') : null, patchDigests: pass ? [digest('8')] : [],
    toolVersions: pass ? {
      ...toolFixture(policy, bound),
      ...(gateValid ? {} : { bootstrapSource: 'wrong' }),
    } : {},
    commands,
    artifactDigests: pass ? Object.fromEntries(
      task.artifactPaths.map((path) => [receiptArtifactKey(0, path), digestValue(path)]),
    ) : {},
    verifierDigests: verifiers, critiqueDigests: pass ? [digest('a')] : [],
    reviewDigests: pass ? [digest('b'), digest('c')] : [],
    recovery: { retryCount: 0, breakerState: 'closed', cancelled: false, repairCount: 0 },
    coordination: {
      swarmId: ruflo.swarmId, taskId: ruflo.coordinationTaskId,
      hookIds: [...ruflo.hookIds], traceIds: [...ruflo.traceIds],
      agenticQeEvidenceDigests: pass ? qeDigests : [],
      nativeEvidenceDigests: pass ? ['native-1', 'native-2', 'native-3', 'native-4'].map(digestValue) : [],
      nativeRuntimeEvidenceDigest: pass ? digestValue('native-runtime') : null,
    },
  });
}

function commandReceipt(
  projection: ProgrammeCommandReceiptProjectionV1,
  candidateTree: string,
  evaluatorTree: string,
) {
  const red = projection.stage === 'red-baseline';
  return {
    stage: projection.stage as 'red-baseline' | 'build' | 'mutation', attempt: 0,
    candidateTree: red ? evaluatorTree : candidateTree,
    tool: projection.tool, executable: projection.executable,
    argv: [...projection.argv], cwd: projection.cwd,
    exitCode: red || projection.stage === 'mutation' ? 101 : 0,
    signal: null, durationMs: 1,
    stdoutDigest: digestValue(`${projection.commandId}:stdout`),
    stderrDigest: digestValue(`${projection.commandId}:stderr`), timedOut: false,
    cancelled: false, outputLimitExceeded: false, spawnErrorDigest: null,
  };
}

function toolFixture(
  policy: FrozenProgrammePolicyV1,
  task: ReturnType<typeof bindProgrammeTaskRuntimeV1>,
): Record<string, string> {
  const gate = policy.gateContract;
  const tools = Object.fromEntries(gate.tools.requiredKeys.map((key) => [key, digestValue(key)]));
  Object.assign(tools, gate.tools.exactValues, {
    bootstrapControllerStoreDigest: policy.bootstrap.controllerStoreDigest,
    bootstrapBuildManifestDigest: policy.controller.buildManifestBlobDigest,
    bootstrapRuntimeTreeDigest: policy.controller.runtimeTreeDigest,
    controllerExecutionDigest: policy.controller.executionDigest,
    controllerBuildManifestDigest: policy.controller.buildManifestBlobDigest,
    controllerRuntimeTreeDigest: policy.controller.runtimeTreeDigest,
    controllerManifestDigest: policy.controller.manifestBlobDigest,
    controllerTaskPath: policy.controller.taskPath,
    controllerTaskPathDigest: digestValue(policy.controller.taskPath),
    controllerTaskDigest: policy.controller.taskBlobDigest,
    taskEvidencePlanDigest: policy.taskEvidencePlanDigest,
    boundTaskDigest: policy.execution.boundTaskDigest,
    programmePolicyFingerprint: programmePolicyFingerprint(policy),
    frozenCargoLockDigest: task.rust.frozenLockSha256,
    rustToolchainClosure: `${gate.tools.structuredValueRules.rustToolchainClosure.leadingValue}:1:1`,
    rustRegistryBootstrapSnapshot: `${digestValue('registry-bootstrap')}:1:1`,
    rustRegistryClosure: `${gate.tools.structuredValueRules.rustRegistryClosure.leadingValue}:1:1`,
    rustRegistryLock: `${task.rust.frozenLockSha256}:1:1`,
    rustRegistrySelection: `x86_64-unknown-linux-gnu:${digestValue('registry-selection')}`,
    rufloHive: 'hierarchical', rufloConsensus: 'raft',
  });
  return tools;
}

function policyFixture(
  manifestBlob = readFileSync(new URL('../.harness/manifest.json', import.meta.url), 'utf8'),
  harnessConfig: HarnessConfig = SECURE_HARNESS_CONFIG,
): FrozenProgrammePolicyV1 {
  const raw = JSON.parse(readFileSync(new URL('../config/issue-8-acceptance.json', import.meta.url), 'utf8')) as any;
  Object.assign(raw, {
    schemaVersion: 3, taskId: 'verifier_only_task_0001',
    workItem: 'completion-programme:reproducibility', candidateOracle: { mode: 'verifier-only' },
    rust: { frozenLockSha256: 'a'.repeat(64) },
    qe: { profiles: [
      { profile: 'sast', collector: 'agentic-qe-sast' },
      { profile: 'lcov-gap', collector: 'rust-lcov', packageName: 'sf-conformance', testTarget: 'issue_8_binding_pruning' },
    ] },
    evidence: {
      requiredAdmittedPaths: ['crates/sf-sparql/src/unfold.rs'],
      generatedOutputs: [{
        stage: 'regression', evidenceId: 'workspace-tests-earl', commandId: 'workspace-tests',
        workspacePaths: ['tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl'],
      }],
    },
  });
  delete raw.qeProfiles;
  const taskBlob = `${JSON.stringify(raw, null, 2)}\n`;
  const task = parseAcceptanceTask(JSON.parse(taskBlob), harnessConfig);
  if (task.schemaVersion !== 3) throw new Error('test task must be schema v3');
  const manifestDigest = sha256(manifestBlob);
  const taskDigest = sha256(taskBlob);
  const protectedInputs = Object.fromEntries([...new Set([
    ...harnessConfig.requiredProtectedPaths, ...task.evaluatorPaths, 'Cargo.lock',
  ])].sort().map((path, index) => [path, sha256(`${index}:${path}`)]));
  const buildBody = {
    schemaVersion: 1, authority: 'development-only-no-promotion',
    runtimeEntry: 'coding-harness/dist/issue-8-program.js', harnessManifestDigest: manifestDigest,
    lockfileDigest: '3'.repeat(64),
    outputs: { 'coding-harness/dist/issue-8-program.js': '4'.repeat(64) },
    productionFiles: { 'coding-harness/node_modules/example/index.js': '5'.repeat(64) },
  } as const;
  const build = { ...buildBody, runtimeTreeDigest: sha256(JSON.stringify(buildBody)) };
  const buildManifestBlob = `${JSON.stringify(build, null, 2)}\n`;
  Object.assign(protectedInputs, {
    [HARNESS_MANIFEST_PATH]: manifestDigest, [taskPath]: taskDigest,
    [CONTROLLER_BUILD_PATH]: sha256(buildManifestBlob),
    'coding-harness/package-lock.json': build.lockfileDigest,
    'Cargo.lock': task.rust.frozenLockSha256,
    [METAHARNESS_DIAGNOSTICS_PATH]: diagnosticBlobDigest,
  });
  const plan = resolveTaskEvidencePlanV1({ task: bindProgrammeTaskRuntimeV1(task), taskPath });
  const input: ControllerPolicyInputs = {
    bootstrap: {
      controllerStoreDigest: '7'.repeat(64),
      nodeDigest: '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc',
      gitDigest: '2a8c18fbf43da9f692d75474c72bea9dfd796c260b0f3dfe456376abc3bbd668',
    },
    controller: {
      identity: { commit: '1'.repeat(40), tree: '2'.repeat(40) },
      manifestPath: HARNESS_MANIFEST_PATH, manifestBlob, manifestBlobDigest: manifestDigest,
      taskPath, taskBlob, taskBlobDigest: taskDigest,
      buildManifestPath: CONTROLLER_BUILD_PATH, buildManifestBlob,
      buildManifestBlobDigest: sha256(buildManifestBlob), build, task, executionDigest: '6'.repeat(64),
    },
    execution: {
      evaluator: { commit: '9'.repeat(40), tree: '8'.repeat(40) }, protectedInputs,
      routeSnapshot: { historyEpoch: 0, decisions: {} },
    },
    taskEvidencePlanDigest: plan.declarationDigest, maxRepairs: 2,
  };
  if (harnessConfig === SECURE_HARNESS_CONFIG) return createFrozenProgrammePolicyV1(input);
  const policy = structuredClone(policyFixture()) as any;
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

function historicalManifest(): { manifestBlob: string; harnessConfig: HarnessConfig } {
  const manifest = JSON.parse(readFileSync(
    new URL('../.harness/manifest.json', import.meta.url), 'utf8',
  )) as Record<string, any>;
  manifest.protectedPaths = manifest.protectedPaths.filter(
    (path: string) => !PROGRAMME_V5_POST_HISTORICAL_PATHS.has(path),
  );
  const manifestBlob = `${JSON.stringify(manifest, null, 2)}\n`;
  return { manifestBlob, harnessConfig: parseHarnessConfig({
    ...structuredClone(SECURE_HARNESS_CONFIG),
    requiredProtectedPaths: SECURE_HARNESS_CONFIG.requiredProtectedPaths
      .filter((path) => !PROGRAMME_V5_POST_HISTORICAL_PATHS.has(path)),
  }) };
}

function rehashEnvelope(value: any): void {
  const { envelopeDigest: _old, ...body } = value;
  value.envelopeDigest = digestValue(body);
}
function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}
function digest(character: string): string {
  return character.repeat(64);
}

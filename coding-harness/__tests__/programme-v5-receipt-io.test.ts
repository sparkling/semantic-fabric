// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync, existsSync, linkSync, lstatSync, mkdirSync, mkdtempSync, readFileSync,
  rmSync, symlinkSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { parseAcceptanceTask } from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { CONTROLLER_BUILD_PATH } from '../src/controller-build.js';
import { HARNESS_MANIFEST_PATH } from '../src/controller-attestation.js';
import { canonicalProgrammePolicyJson } from '../src/programme-v5-driver-support.js';
import {
  claimProgrammeV5Execution, type ProgrammeV5PolicyReviewReceipt,
} from '../src/programme-v5-policy-anchor.js';
import {
  createFrozenProgrammePolicyV1, type ControllerPolicyInputs,
} from '../src/programme-policy-v5.js';
import { prepareTrustedProgrammeV5 } from '../src/programme-v5-program.js';
import {
  programmeV5ArtifactPath, programmeV5AuthorityClaimPath,
  readProgrammeV5PrivateArtifact,
  writeProgrammeV5PrivateArtifact,
} from '../src/programme-v5-receipt-io.js';
import type { ProgrammeV5Invocation } from '../src/programme-v5-program-runtime.js';
import { bindProgrammeTaskRuntimeV1 } from '../src/programme-task-runtime-v1.js';
import { resolveTaskEvidencePlanV1 } from '../src/task-evidence-plan.js';

const roots: string[] = [];
const RUN_ID = 'programme_v5_receipt_io';

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme-v5 private receipt IO', () => {
  it('writes, fsyncs, stable-reads, and refuses reuse or hard links', () => {
    const repository = fixtureRepository();
    const path = programmeV5ArtifactPath(repository, RUN_ID, 'execution-claim');
    writeProgrammeV5PrivateArtifact(repository, path, '{"claim":true}\n', 1_000);

    expect(readProgrammeV5PrivateArtifact(repository, path, 1_000)).toBe('{"claim":true}\n');
    expect(lstatSync(path).mode & 0o777).toBe(0o600);
    expect(() => writeProgrammeV5PrivateArtifact(repository, path, '{}\n', 1_000))
      .toThrow('HARNESS_PROGRAMME_V5_RECEIPT_EXISTS');

    const linked = programmeV5ArtifactPath(repository, RUN_ID, 'replay');
    linkSync(path, linked);
    expect(() => readProgrammeV5PrivateArtifact(repository, path, 1_000))
      .toThrow('HARNESS_PROGRAMME_V5_ARTIFACT_INVALID');
  });

  it('rejects an ancestor symlink before creating anything outside the repository', () => {
    const repository = fixtureRepository();
    const outside = temporary('programme-v5-receipt-outside-');
    symlinkSync(outside, join(repository, 'coding-harness', '.metaharness'));
    const path = join(
      repository, 'coding-harness', '.metaharness', 'runs', `${RUN_ID}.policy-review.json`,
    );

    expect(() => writeProgrammeV5PrivateArtifact(repository, path, '{}\n'))
      .toThrow('HARNESS_PROGRAMME_V5_RESULT_ROOT_INVALID');
    expect(existsSync(join(outside, 'runs'))).toBe(false);
  });

  it('treats a dangling output symlink as an existing fail-closed artifact', () => {
    const repository = fixtureRepository();
    const seed = programmeV5ArtifactPath(repository, 'programme_v5_seed_run', 'replay');
    writeProgrammeV5PrivateArtifact(repository, seed, '{}\n');
    const path = programmeV5ArtifactPath(repository, RUN_ID, 'replay');
    const outside = join(temporary('programme-v5-dangling-target-'), 'target.json');
    symlinkSync(outside, path);

    expect(() => writeProgrammeV5PrivateArtifact(repository, path, '{}\n'))
      .toThrow('HARNESS_PROGRAMME_V5_RECEIPT_EXISTS');
    expect(existsSync(outside)).toBe(false);
  });

  it('claims one execution exactly once before model work can begin', () => {
    const repository = fixtureRepository();
    const invocation = executionInvocation(repository);
    const review = policyReviewReceipt();
    const authority = temporary('programme-v5-claim-authority-');
    const first = claimProgrammeV5Execution(invocation, review, authority);

    expect(first.path).toBe(programmeV5AuthorityClaimPath(
      sha256(canonicalClaimKey(invocation, review)), authority,
    ));
    expect(first.digest).toMatch(/^[a-f0-9]{64}$/);
    expect(lstatSync(join(authority, 'programme-v5-claims')).mode & 0o777).toBe(0o700);
    expect(lstatSync(first.path).mode & 0o777).toBe(0o600);
    expect(JSON.parse(readFileSync(first.path, 'utf8'))).toMatchObject({
      operation: 'programme-v5-execution-claim',
      runId: RUN_ID,
      claimDigest: first.digest,
    });
    expect(() => claimProgrammeV5Execution(invocation, review, authority))
      .toThrow('HARNESS_PROGRAMME_V5_RECEIPT_EXISTS');
    expect(() => claimProgrammeV5Execution(
      executionInvocation(fixtureRepository()), review, authority,
    )).toThrow('HARNESS_PROGRAMME_V5_RECEIPT_EXISTS');
  });

  it('rejects a tampered policy-review digest before creating an execution claim', async () => {
    const repository = fixtureRepository();
    const store = temporary('programme-v5-receipt-store-');
    const authority = temporary('programme-v5-claim-authority-');
    const runId = 'programme_v5_tampered_review';
    const policy = policyFixture(runId);
    const policyBlob = canonicalProgrammePolicyJson(policy);
    const policyFingerprint = sha256(policyBlob);
    const reviewPath = programmeV5ArtifactPath(repository, runId, 'policy-review');
    const body = {
      schemaVersion: 1,
      authority: 'development-only-no-promotion',
      operation: 'programme-v5-policy-review',
      controllerCommit: policy.controller.identity.commit,
      taskPath: policy.controller.taskPath,
      runId,
      swarmId: 'programme_v5_swarm',
      coordinationTaskId: 'programme_v5_task',
      hiveId: 'hierarchical',
      consensusId: 'raft',
      controllerStoreDigest: policy.bootstrap.controllerStoreDigest,
      buildManifestDigest: policy.controller.buildManifestBlobDigest,
      runtimeTreeDigest: policy.controller.runtimeTreeDigest,
      nodeDigest: policy.bootstrap.nodeDigest,
      gitDigest: policy.bootstrap.gitDigest,
      policyFingerprint,
      policyBlob,
    } as const;
    writeProgrammeV5PrivateArtifact(repository, reviewPath, `${JSON.stringify({
      ...body,
      policyReviewReceiptDigest: 'f'.repeat(64),
    })}\n`);
    const bootstrap = {
      schemaVersion: 3,
      source: 'verified-packed-private-runtime',
      controllerCommit: body.controllerCommit,
      taskPath: body.taskPath,
      controllerStoreDigest: body.controllerStoreDigest,
      buildManifestDigest: body.buildManifestDigest,
      runtimeTreeDigest: body.runtimeTreeDigest,
      nodeDigest: body.nodeDigest,
      gitDigest: body.gitDigest,
    } as const;
    const args = [
      '--repository', repository, '--controller-store', store,
      '--controller-commit', body.controllerCommit, '--run-id', runId,
      '--swarm-id', 'programme_v5_swarm', '--coordination-task-id', 'programme_v5_task',
      '--hive-id', 'hierarchical', '--consensus-id', 'raft', '--task-path', body.taskPath,
      '--expected-policy-fingerprint', policyFingerprint,
      '--policy-review-receipt', reviewPath,
    ];

    await expect(prepareTrustedProgrammeV5(args, bootstrap, authority))
      .rejects.toThrow('HARNESS_PROGRAMME_V5_POLICY_REVIEW_RECEIPT_INVALID');
    expect(existsSync(join(authority, 'programme-v5-claims'))).toBe(false);
  });
});

function fixtureRepository(): string {
  const repository = temporary('programme-v5-receipt-repository-');
  mkdirSync(join(repository, 'coding-harness'), { mode: 0o755 });
  return repository;
}

function executionInvocation(repositoryRoot: string): ProgrammeV5Invocation {
  const taskPath = 'coding-harness/config/programme-v5-acceptance.json';
  return {
    repositoryRoot,
    controllerStore: temporary('programme-v5-receipt-store-'),
    controllerCommit: 'a'.repeat(40),
    taskPath,
    runId: RUN_ID,
    swarmId: 'programme_v5_swarm',
    coordinationTaskId: 'programme_v5_task',
    hiveId: 'hierarchical',
    consensusId: 'raft',
    policyReviewReceipt: programmeV5ArtifactPath(repositoryRoot, RUN_ID, 'policy-review'),
    expectedPolicy: {
      controllerCommit: 'a'.repeat(40),
      taskPath,
      fingerprint: 'f'.repeat(64),
    },
  };
}

function policyReviewReceipt(): ProgrammeV5PolicyReviewReceipt {
  return {
    schemaVersion: 1,
    authority: 'development-only-no-promotion',
    operation: 'programme-v5-policy-review',
    controllerCommit: 'a'.repeat(40),
    taskPath: 'coding-harness/config/programme-v5-acceptance.json',
    runId: RUN_ID,
    swarmId: 'programme_v5_swarm',
    coordinationTaskId: 'programme_v5_task',
    hiveId: 'hierarchical',
    consensusId: 'raft',
    controllerStoreDigest: '1'.repeat(64),
    buildManifestDigest: '2'.repeat(64),
    runtimeTreeDigest: '3'.repeat(64),
    nodeDigest: '4'.repeat(64),
    gitDigest: '5'.repeat(64),
    policyFingerprint: 'f'.repeat(64),
    policyBlob: '{}',
    policyReviewReceiptDigest: '6'.repeat(64),
  };
}

function policyFixture(runId: string) {
  const taskPath = 'coding-harness/config/issue-8-acceptance.json';
  const manifestBlob = readFileSync(
    new URL('../.harness/manifest.json', import.meta.url), 'utf8',
  );
  const taskInput = JSON.parse(readFileSync(
    new URL('../config/issue-8-acceptance.json', import.meta.url), 'utf8',
  )) as Record<string, any>;
  Object.assign(taskInput, {
    schemaVersion: 3,
    taskId: 'verifier_only_task_0001',
    workItem: 'completion-programme:reproducibility',
    candidateOracle: { mode: 'verifier-only' },
    rust: { frozenLockSha256: 'a'.repeat(64) },
    qe: { profiles: [
      { profile: 'sast', collector: 'agentic-qe-sast' },
      {
        profile: 'lcov-gap', collector: 'rust-lcov', packageName: 'sf-conformance',
        testTarget: 'issue_8_binding_pruning',
      },
    ] },
    evidence: {
      requiredAdmittedPaths: ['crates/sf-sparql/src/unfold.rs'],
      generatedOutputs: [{
        stage: 'regression', evidenceId: 'workspace-tests-earl',
        commandId: 'workspace-tests',
        workspacePaths: ['tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl'],
      }],
    },
  });
  delete taskInput.qeProfiles;
  const taskBlob = `${JSON.stringify(taskInput, null, 2)}\n`;
  const task = parseAcceptanceTask(taskInput, SECURE_HARNESS_CONFIG);
  if (task.schemaVersion !== 3) throw new Error('test task must be schema v3');
  const manifestBlobDigest = sha256(manifestBlob);
  const taskBlobDigest = sha256(taskBlob);
  const protectedInputs = Object.fromEntries([...new Set([
    ...SECURE_HARNESS_CONFIG.requiredProtectedPaths, ...task.evaluatorPaths, 'Cargo.lock',
  ])].sort().map((path, index) => [path, sha256(`${index}:${path}`)]));
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
  Object.assign(protectedInputs, {
    [HARNESS_MANIFEST_PATH]: manifestBlobDigest,
    [taskPath]: taskBlobDigest,
    [CONTROLLER_BUILD_PATH]: sha256(buildManifestBlob),
    'coding-harness/package-lock.json': build.lockfileDigest,
    'Cargo.lock': task.rust.frozenLockSha256,
  });
  const evidencePlan = resolveTaskEvidencePlanV1({
    task: bindProgrammeTaskRuntimeV1(task), taskPath,
  });
  const input: ControllerPolicyInputs = {
    bootstrap: {
      controllerStoreDigest: '7'.repeat(64),
      nodeDigest: '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc',
      gitDigest: '2a8c18fbf43da9f692d75474c72bea9dfd796c260b0f3dfe456376abc3bbd668',
    },
    controller: {
      identity: { commit: 'a'.repeat(40), tree: 'b'.repeat(40) },
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
      task,
      executionDigest: '8'.repeat(64),
    },
    execution: {
      evaluator: { commit: 'c'.repeat(40), tree: 'd'.repeat(40) },
      protectedInputs,
      routeSnapshot: {
        historyEpoch: 0,
        decisions: Object.fromEntries(['architecture', 'implementation', 'repair'].map(
          (step) => [step, { runId, stepKind: step }],
        )),
      },
    },
    taskEvidencePlanDigest: evidencePlan.declarationDigest,
    maxRepairs: 2,
  };
  return createFrozenProgrammePolicyV1(input);
}

function temporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  chmodSync(root, 0o700);
  roots.push(root);
  return root;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function canonicalClaimKey(
  invocation: ProgrammeV5Invocation,
  receipt: ProgrammeV5PolicyReviewReceipt,
): string {
  return canonicalProgrammePolicyJson({
    schemaVersion: 1,
    authority: 'programme-v5-local-subscription-host',
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    runId: invocation.runId,
    swarmId: invocation.swarmId,
    coordinationTaskId: invocation.coordinationTaskId,
    policyReviewReceiptDigest: receipt.policyReviewReceiptDigest,
  });
}

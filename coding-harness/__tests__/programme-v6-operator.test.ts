// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import {
  assertProgrammeV6ChildStatus, createPackedControllerStore,
  parsePolicyReviewReceipt, parseProgrammeV6ExecutionSummary,
  parseProgrammeV6ReplaySummary,
  programmeV6OperatorExitCode,
} from '../scripts/run-programme-v6.mjs';

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const operatorPath = resolve(repositoryRoot, 'coding-harness/scripts/run-programme-v6.mjs');
const controllerCommit = gitText(['rev-parse', 'HEAD']);
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme-v6 operator', () => {
  it('propagates recorded execution rejection but treats verified replay separately', () => {
    expect(programmeV6OperatorExitCode('execute', { status: 'pass' })).toBe(0);
    expect(programmeV6OperatorExitCode('execute', { status: 'gated' })).toBe(1);
    expect(programmeV6OperatorExitCode('execute', { status: 'fail' })).toBe(1);
    expect(programmeV6OperatorExitCode('replay', { recordedStatus: 'gated' })).toBe(0);
    expect(() => assertProgrammeV6ChildStatus('execute', 1, { status: 'pass' }))
      .toThrow('HARNESS_OPERATOR_CHILD_STATUS_MISMATCH');
    expect(() => assertProgrammeV6ChildStatus('execute', 0, { status: 'gated' }))
      .toThrow('HARNESS_OPERATOR_CHILD_STATUS_MISMATCH');
    expect(() => assertProgrammeV6ChildStatus('execute', 0, { status: 'pass' })).not.toThrow();
  });

  it('keys candidate evidence to transaction status, not the final outcome', () => {
    const policyFingerprint = '1'.repeat(64);
    const execution = {
      status: 'gated', transactionStatus: 'pass',
      reason: 'HARNESS_PROGRAMME_ACCEPTANCE_REJECTED',
      receiptDigest: '2'.repeat(64),
      candidateTransactionEvidenceDigest: '3'.repeat(64),
      programmeAcceptanceDigest: '4'.repeat(64), envelopeDigest: '5'.repeat(64),
      policyFingerprint, executionClaimDigest: '6'.repeat(64),
      launchReceiptDigest: '7'.repeat(64),
    };
    expect(parseProgrammeV6ExecutionSummary(
      `${JSON.stringify(execution)}\n`, policyFingerprint,
    )).toEqual(execution);
    expect(() => parseProgrammeV6ExecutionSummary(`${JSON.stringify({
      ...execution, candidateTransactionEvidenceDigest: null,
    })}\n`, policyFingerprint)).toThrow('HARNESS_OPERATOR_EXECUTION_SUMMARY_INVALID');

    const invocation = {
      receiptPath: '/receipt', expectedPolicyFingerprint: policyFingerprint,
      controllerCommit, taskPath: 'coding-harness/config/programme-v5-acceptance.json',
    };
    const basePolicyFingerprint = 'b'.repeat(64);
    const replay = {
      verificationStatus: 'verified', transactionStatus: 'gated',
      recordedStatus: 'gated', recordedReason: 'HARNESS_TRANSACTION_FAILED',
      receiptPath: invocation.receiptPath, replayReceiptDigest: '8'.repeat(64),
      receiptDigest: '9'.repeat(64), envelopeDigest: 'a'.repeat(64), policyFingerprint,
      basePolicyFingerprint, candidateTransactionEvidenceDigest: null,
      executionClaimDigest: 'c'.repeat(64), launchReceiptDigest: '',
    };
    replay.launchReceiptDigest = replayLaunchDigest(replay, invocation);
    expect(parseProgrammeV6ReplaySummary(
      `${JSON.stringify(replay)}\n`, invocation, basePolicyFingerprint,
    )).toEqual(replay);
    expect(() => parseProgrammeV6ReplaySummary(`${JSON.stringify({
      ...replay, candidateTransactionEvidenceDigest: 'b'.repeat(64),
    })}\n`, invocation, basePolicyFingerprint))
      .toThrow('HARNESS_OPERATOR_REPLAY_SUMMARY_INVALID');
    expect(() => parseProgrammeV6ReplaySummary(`${JSON.stringify({
      ...replay, launchReceiptDigest: 'd'.repeat(64),
    })}\n`, invocation, basePolicyFingerprint))
      .toThrow('HARNESS_OPERATOR_REPLAY_SUMMARY_INVALID');
  });

  it('requires a clean outer Node process before parsing an operation', () => {
    const runtime = temporary('programme-v6-operator-process-');
    const base = [
      '-i', 'LANG=C.UTF-8', `XDG_RUNTIME_DIR=${runtime}`,
      `DBUS_SESSION_BUS_ADDRESS=unix:path=${runtime}/bus`,
    ];
    const previousUmask = process.umask(0o077);
    try {
      const clean = spawnSync('/usr/bin/env', [
        ...base, '/usr/bin/node', '--no-addons', '--disable-proto=throw', operatorPath,
      ], { encoding: 'utf8' });
      expect(JSON.parse(clean.stderr)).toEqual({
        status: 'error', reason: 'HARNESS_OPERATOR_ARGUMENTS_INVALID',
      });
      const contaminated = spawnSync('/usr/bin/env', [
        ...base, 'NODE_OPTIONS=--trace-warnings', '/usr/bin/node',
        '--no-addons', '--disable-proto=throw', operatorPath,
      ], { encoding: 'utf8' });
      expect(JSON.parse(contaminated.stderr)).toEqual({
        status: 'error', reason: 'HARNESS_OPERATOR_ENVIRONMENT_INVALID',
      });
    } finally { process.umask(previousUmask); }
  });

  it('recreates a byte-identical packed controller store for one commit', () => {
    const runtimeRoot = temporary('programme-v6-operator-runtime-');
    const first = createPackedControllerStore({ repositoryRoot, runtimeRoot, controllerCommit });
    const second = createPackedControllerStore({ repositoryRoot, runtimeRoot, controllerCommit });
    try {
      expect(first.path).not.toBe(second.path);
      expect(first.digest).toMatch(/^[a-f0-9]{64}$/);
      expect(second.digest).toBe(first.digest);
    } finally {
      first.cleanup();
      second.cleanup();
    }
    expect(existsSync(first.path)).toBe(false);
    expect(existsSync(second.path)).toBe(false);
  }, 15_000);

  it('verifies the exact replay receipt and its external bindings', () => {
    const taskPath = 'coding-harness/config/programme-v5-acceptance.json';
    const baseGateContract = { schemaVersion: 1 };
    const basePolicy = {
      schemaVersion: 1,
      policyId: 'semantic-fabric-programme-v5-policy-v1',
      authority: 'development-only-no-promotion',
      gateContract: baseGateContract,
      bootstrap: {
        controllerStoreDigest: '1'.repeat(64),
        gitDigest: '5'.repeat(64),
        nodeDigest: '4'.repeat(64),
      },
      controller: {
        buildManifestBlobDigest: '2'.repeat(64),
        identity: { commit: controllerCommit },
        runtimeTreeDigest: '3'.repeat(64),
        taskPath,
      },
      execution: {
        routeSnapshotBlob: JSON.stringify({
          historyEpoch: 0,
          decisions: Object.fromEntries(['architecture', 'implementation', 'repair'].map(
            (step) => [step, { runId: 'programme_v6_operator_run', stepKind: step }],
          )),
        }),
        protectedInputs: {
          'README.md': '6'.repeat(64),
          'coding-harness/src/config.ts': '7'.repeat(64),
        },
      },
    };
    const policy = {
      schemaVersion: 2,
      policyId: 'semantic-fabric-programme-v6-policy-v2',
      authority: 'development-only-no-promotion',
      basePolicy,
      basePolicyFingerprint: sha256(canonical(basePolicy)),
      gateContract: {
        schemaVersion: 2,
        contractId: 'semantic-fabric-programme-gate-contract-v2',
        authority: 'development-only-no-promotion',
        authoritativeReplaySemantics: true,
        baseGateContract,
        baseGateContractDigest: sha256(canonical(baseGateContract)),
        attempts: {},
        nativeEvidence: {},
        envelope: {
          schemaVersion: 6,
          policyFingerprintBinding: 'externally-supplied-v2-anchor',
          candidateEvidenceDigestBinding:
            'envelope.candidateTransactionEvidenceDigest-equals-candidateTransactionEvidence.evidenceDigest',
          baseReceiptAndDiagnosticSemantics: 'programme-gate-contract-v1-unchanged',
        },
        evaluation: {},
      },
    };
    const policyBlob = canonical(policy);
    const body = {
      schemaVersion: 1,
      authority: 'development-only-no-promotion',
      operation: 'programme-v6-policy-review',
      controllerCommit,
      taskPath,
      runId: 'programme_v6_operator_run',
      swarmId: 'programme_v6_swarm',
      coordinationTaskId: 'programme_v6_task',
      hiveId: 'hierarchical',
      consensusId: 'raft',
      controllerStoreDigest: '1'.repeat(64),
      buildManifestDigest: '2'.repeat(64),
      runtimeTreeDigest: '3'.repeat(64),
      nodeDigest: '4'.repeat(64),
      gitDigest: '5'.repeat(64),
      policyFingerprint: sha256(policyBlob),
      policyBlob,
    };
    const receipt = {
      ...body,
      policyReviewReceiptDigest: sha256(canonical(body)),
    };
    const expected = {
      controllerCommit,
      taskPath: body.taskPath,
      runId: body.runId,
      swarmId: body.swarmId,
      coordinationTaskId: body.coordinationTaskId,
      hiveId: body.hiveId,
      consensusId: body.consensusId,
    };

    expect(parsePolicyReviewReceipt(`${JSON.stringify(receipt)}\n`, expected)).toEqual(receipt);
    expect(() => parsePolicyReviewReceipt(JSON.stringify(receipt), expected))
      .toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    const downgradedPolicyBlob = canonical({
      ...policy,
      gateContract: {
        ...policy.gateContract,
        envelope: { ...policy.gateContract.envelope, schemaVersion: 5 },
      },
    });
    const downgradedBody = {
      ...body,
      policyBlob: downgradedPolicyBlob,
      policyFingerprint: sha256(downgradedPolicyBlob),
    };
    expect(() => parsePolicyReviewReceipt(`${JSON.stringify({
      ...downgradedBody,
      policyReviewReceiptDigest: sha256(canonical(downgradedBody)),
    })}\n`, expected)).toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    const legacyBody = { ...body, operation: 'programme-v5-policy-review' };
    expect(() => parsePolicyReviewReceipt(`${JSON.stringify({
      ...legacyBody,
      policyReviewReceiptDigest: sha256(canonical(legacyBody)),
    })}\n`, expected)).toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    expect(() => parsePolicyReviewReceipt(`${JSON.stringify({
      ...receipt,
      runId: 'different_run',
    })}\n`, expected)).toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    expect(() => parsePolicyReviewReceipt(`${JSON.stringify({
      ...receipt,
      extra: true,
    })}\n`, expected)).toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    expect(() => parsePolicyReviewReceipt(
      `${JSON.stringify(receipt)
        .replace('"schemaVersion":1', '"schemaVersion":1,"schemaVersion":1')}\n`,
      expected,
    )).toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    const remintedBody = {
      ...body,
      buildManifestDigest: '6'.repeat(64),
    };
    const reminted = {
      ...remintedBody,
      policyReviewReceiptDigest: sha256(canonical(remintedBody)),
    };
    expect(() => parsePolicyReviewReceipt(`${JSON.stringify(reminted)}\n`, expected))
      .toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    const substitutedBody = { ...body, swarmId: 'programme_v6_other_swarm' };
    expect(() => parsePolicyReviewReceipt(`${JSON.stringify({
      ...substitutedBody,
      policyReviewReceiptDigest: sha256(canonical(substitutedBody)),
    })}\n`, expected)).toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
  });
});

function temporary(prefix: string): string {
  const root = mkdtempSync(resolve(tmpdir(), prefix));
  chmodSync(root, 0o700);
  roots.push(root);
  return root;
}

function gitText(args: readonly string[]): string {
  const result = spawnSync('/usr/bin/git', args, {
    cwd: repositoryRoot,
    encoding: 'utf8',
    env: {
      PATH: '/usr/bin:/bin',
      HOME: '/nonexistent',
      LANG: 'C',
      LC_ALL: 'C',
      GIT_CONFIG_NOSYSTEM: '1',
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_ATTR_NOSYSTEM: '1',
      GIT_NO_REPLACE_OBJECTS: '1',
      GIT_NO_LAZY_FETCH: '1',
    },
  });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}

function canonical(value: unknown): string {
  if (Array.isArray(value)) return JSON.stringify(value.map(canonicalValue));
  return JSON.stringify(canonicalValue(value));
}

function canonicalValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalValue);
  if (value !== null && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    return Object.fromEntries(Object.keys(record).sort()
      .map((key) => [key, canonicalValue(record[key])]));
  }
  return value;
}

function sha256(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

function replayLaunchDigest(
  replay: Record<string, unknown>, invocation: Record<string, unknown>,
): string {
  return sha256(canonical({
    schemaVersion: 1,
    domain: 'semantic-fabric/programme-v6/replay-launch/v1',
    operation: 'programme-v6-replay-launch',
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    outerPolicyFingerprint: replay.policyFingerprint,
    basePolicyFingerprint: replay.basePolicyFingerprint,
    envelopeDigest: replay.envelopeDigest,
    transactionStatus: replay.transactionStatus,
    receiptDigest: replay.receiptDigest,
    candidateTransactionEvidenceDigest: replay.candidateTransactionEvidenceDigest,
    executionClaimDigest: replay.executionClaimDigest,
  }));
}

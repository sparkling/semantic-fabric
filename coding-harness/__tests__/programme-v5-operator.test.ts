// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import {
  assertProgrammeV5ChildStatus, createPackedControllerStore,
  parsePolicyReviewReceipt,
  programmeV5OperatorExitCode,
} from '../scripts/run-programme-v5.mjs';

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const operatorPath = resolve(repositoryRoot, 'coding-harness/scripts/run-programme-v5.mjs');
const controllerCommit = gitText(['rev-parse', 'HEAD']);
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme-v5 operator', () => {
  it('propagates recorded execution rejection but treats verified replay separately', () => {
    expect(programmeV5OperatorExitCode('execute', { status: 'pass' })).toBe(0);
    expect(programmeV5OperatorExitCode('execute', { status: 'gated' })).toBe(1);
    expect(programmeV5OperatorExitCode('execute', { status: 'fail' })).toBe(1);
    expect(programmeV5OperatorExitCode('replay', { recordedStatus: 'gated' })).toBe(0);
    expect(() => assertProgrammeV5ChildStatus('execute', 1, { status: 'pass' }))
      .toThrow('HARNESS_OPERATOR_CHILD_STATUS_MISMATCH');
    expect(() => assertProgrammeV5ChildStatus('execute', 0, { status: 'gated' }))
      .toThrow('HARNESS_OPERATOR_CHILD_STATUS_MISMATCH');
    expect(() => assertProgrammeV5ChildStatus('execute', 0, { status: 'pass' })).not.toThrow();
  });

  it('requires a clean outer Node process before parsing an operation', () => {
    const runtime = temporary('programme-v5-operator-process-');
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
    const runtimeRoot = temporary('programme-v5-operator-runtime-');
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
    const policyBlob = canonical({
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
            (step) => [step, { runId: 'programme_v5_operator_run', stepKind: step }],
          )),
        }),
      },
    });
    const body = {
      schemaVersion: 1,
      authority: 'development-only-no-promotion',
      operation: 'programme-v5-policy-review',
      controllerCommit,
      taskPath,
      runId: 'programme_v5_operator_run',
      swarmId: 'programme_v5_swarm',
      coordinationTaskId: 'programme_v5_task',
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

    expect(parsePolicyReviewReceipt(JSON.stringify(receipt), expected)).toEqual(receipt);
    expect(() => parsePolicyReviewReceipt(JSON.stringify({
      ...receipt,
      runId: 'different_run',
    }), expected)).toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    expect(() => parsePolicyReviewReceipt(JSON.stringify({
      ...receipt,
      extra: true,
    }), expected)).toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    expect(() => parsePolicyReviewReceipt(
      JSON.stringify(receipt).replace('"schemaVersion":1', '"schemaVersion":1,"schemaVersion":1'),
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
    expect(() => parsePolicyReviewReceipt(JSON.stringify(reminted), expected))
      .toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
    const substitutedBody = { ...body, swarmId: 'programme_v5_other_swarm' };
    expect(() => parsePolicyReviewReceipt(JSON.stringify({
      ...substitutedBody,
      policyReviewReceiptDigest: sha256(canonical(substitutedBody)),
    }), expected)).toThrow('HARNESS_OPERATOR_POLICY_RECEIPT_INVALID');
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
    return Object.fromEntries(Object.entries(value)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, entry]) => [key, canonicalValue(entry)]));
  }
  return value;
}

function sha256(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

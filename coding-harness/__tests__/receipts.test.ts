// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import { ReceiptChain, digestValue, parseReceiptDraft } from '../src/receipts.js';

const gitCommit = 'a'.repeat(40);
const gitTree = 'b'.repeat(40);
const timestamp = '2026-08-25T00:00:00.000Z';

function draft(step = 'build') {
  return {
    schemaVersion: 2,
    runId: 'run-0001',
    taskId: 'task-0001',
    step,
    status: 'fail',
    authority: 'development-only-no-promotion',
    issuedAt: timestamp,
    identities: {
      controller: { commit: gitCommit, tree: gitTree },
      baseline: { commit: gitCommit, tree: gitTree },
      evaluator: { commit: gitCommit, tree: gitTree },
      candidate: { commit: gitCommit, tree: gitTree },
    },
    protectedInputs: { 'protected.txt': digestValue('protected') },
    route: {
      snapshotDigest: digestValue('route'),
      frozenAt: timestamp,
      routerVersion: '0.4.0',
    },
    hosts: [{
      host: 'codex',
      model: 'gpt-native',
      role: 'implementer',
      clientVersion: '1.0.0',
      authClass: 'native-openai-subscription',
      subscriptionCostUsd: 0,
    }],
    admittedPaths: ['src/change.ts'],
    patchDigest: digestValue('patch'),
    patchDigests: [digestValue('patch')],
    toolVersions: { node: process.version },
    commands: [{
      stage: 'build',
      attempt: 0,
      candidateTree: gitTree,
      tool: 'node',
      executable: 'node',
      argv: ['--version'],
      cwd: '.',
      exitCode: 0,
      signal: null,
      durationMs: 10,
      stdoutDigest: digestValue('stdout'),
      stderrDigest: digestValue('stderr'),
      timedOut: false,
      cancelled: false,
      outputLimitExceeded: false,
      spawnErrorDigest: null,
    }],
    artifactDigests: { build: digestValue('artifact') },
    verifierDigests: { unit: digestValue('verifier') },
    critiqueDigests: [digestValue('critique')],
    reviewDigests: [digestValue('review')],
    recovery: {
      retryCount: 0,
      breakerState: 'closed',
      cancelled: false,
      repairCount: 0,
    },
    coordination: {
      swarmId: 'swarm-1',
      taskId: 'ruflo-task-1',
      hookIds: ['route-1'],
      traceIds: ['trace-1'],
      agenticQeEvidenceDigests: [digestValue('qe')],
      nativeEvidenceDigests: [digestValue('native')],
      nativeRuntimeEvidenceDigest: null,
    },
  };
}

function passDraft() {
  const value = draft('candidate-transaction');
  const candidateTree = 'c'.repeat(40);
  value.status = 'pass';
  value.identities.candidate = { commit: 'd'.repeat(40), tree: candidateTree };
  value.hosts = [
    value.hosts[0],
    {
      host: 'claude-code', model: 'claude-native', role: 'reviewer', clientVersion: '1.0.0',
      authClass: 'native-anthropic-subscription', subscriptionCostUsd: 0,
    },
  ];
  const command = value.commands[0];
  value.commands = [
    { ...command, stage: 'red-baseline', candidateTree: gitTree, exitCode: 101 },
    { ...command, stage: 'build', candidateTree, exitCode: 0 },
    { ...command, stage: 'mutation', candidateTree, exitCode: 101 },
  ];
  value.verifierDigests = {
    'red-baseline': digestValue('red'), 'attempt-0:mutation': digestValue('mutation'),
    'attempt-0:public': digestValue('public'),
    'attempt-0:independent': digestValue('independent'),
    'attempt-0:regression': digestValue('regression'),
  };
  value.reviewDigests = [digestValue('codex-review'), digestValue('claude-review')];
  value.coordination.nativeEvidenceDigests = ['host-a', 'host-b', 'invoke-a', 'invoke-b']
    .map(digestValue);
  value.coordination.nativeRuntimeEvidenceDigest = digestValue('native-runtime');
  return value;
}

describe('strict receipt schema', () => {
  it('requires fixed development-only authority and rejects unknown fields', () => {
    expect(() => parseReceiptDraft({ ...draft(), authority: 'publish' })).toThrow(/promotion authority/);
    expect(() => parseReceiptDraft({ ...draft(), extra: true })).toThrow(/invalid keys/);
  });

  it('rejects a structurally hashed pass without candidate transaction evidence', () => {
    expect(() => parseReceiptDraft({ ...draft(), status: 'pass' })).toThrow(
      'HARNESS_PASS_RECEIPT_EVIDENCE_INCOMPLETE',
    );
  });

  it('rejects contradictory or duplicated pass evidence', () => {
    expect(parseReceiptDraft(passDraft()).status).toBe('pass');
    for (const invalid of [
      { recovery: { ...passDraft().recovery, cancelled: true } },
      { recovery: { ...passDraft().recovery, breakerState: 'open' } },
      { patchDigests: [...passDraft().patchDigests, digestValue('extra-patch')] },
      { reviewDigests: Array(2).fill(digestValue('same-review')) },
      { coordination: {
        ...passDraft().coordination,
        nativeEvidenceDigests: Array(4).fill(digestValue('same-native')),
      } },
    ]) {
      expect(() => parseReceiptDraft({ ...passDraft(), ...invalid })).toThrow(
        'HARNESS_PASS_RECEIPT_EVIDENCE_INCOMPLETE',
      );
    }
  });

  it('binds native host authentication and rejects indirect gateways', () => {
    const invalid = draft();
    invalid.hosts[0].model = 'openrouter/gpt';
    expect(() => parseReceiptDraft(invalid)).toThrow(/indirect gateway/);

    const wrongAuth = draft();
    wrongAuth.hosts[0].authClass = 'native-anthropic-subscription';
    expect(() => parseReceiptDraft(wrongAuth)).toThrow(/does not match/);
  });
});

describe('receipt chain', () => {
  it('rejects legacy v1 chains after the evidence-shape version bump', () => {
    expect(() => ReceiptChain.import(JSON.stringify({ schemaVersion: 1, receipts: [] })))
      .toThrow(/schemaVersion 2/);
  });

  it('chains canonical receipts and round-trips verified evidence', () => {
    const chain = new ReceiptChain();
    const first = chain.append(draft('build'));
    const second = chain.append(draft('verify'));
    expect(first.previousDigest).toBe('0'.repeat(64));
    expect(second.previousDigest).toBe(first.digest);
    expect(chain.verify()).toEqual({ ok: true });

    const restored = ReceiptChain.import(chain.export());
    expect(restored.verify()).toEqual({ ok: true });
    expect(restored.headDigest).toBe(chain.headDigest);
  });

  it('detects body tampering during import', () => {
    const chain = new ReceiptChain();
    chain.append(draft());
    const exported = JSON.parse(chain.export());
    exported.receipts[0].status = 'gated';
    expect(() => ReceiptChain.import(JSON.stringify(exported))).toThrow(/digest does not match body/);
  });

  it('does not expose mutable receipt entries', () => {
    const chain = new ReceiptChain();
    const receipt = chain.append(draft());
    expect(Object.isFrozen(receipt)).toBe(true);
    expect(() => { (receipt as { step: string }).step = 'tampered'; }).toThrow();
    expect(chain.verify()).toEqual({ ok: true });
  });
});

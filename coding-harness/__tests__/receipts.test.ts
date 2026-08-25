// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import { ReceiptChain, digestValue, parseReceiptDraft } from '../src/receipts.js';

const gitCommit = 'a'.repeat(40);
const gitTree = 'b'.repeat(40);
const timestamp = '2026-08-25T00:00:00.000Z';

function draft(step = 'build') {
  return {
    schemaVersion: 1,
    runId: 'run-0001',
    taskId: 'task-0001',
    step,
    status: 'pass',
    authority: 'development-only-no-promotion',
    issuedAt: timestamp,
    identities: {
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
    toolVersions: { node: process.version },
    commands: [{
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
      agenticQeEvidenceDigests: [digestValue('qe')],
    },
  };
}

describe('strict receipt schema', () => {
  it('requires fixed development-only authority and rejects unknown fields', () => {
    expect(() => parseReceiptDraft({ ...draft(), authority: 'publish' })).toThrow(/promotion authority/);
    expect(() => parseReceiptDraft({ ...draft(), extra: true })).toThrow(/invalid keys/);
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
    exported.receipts[0].status = 'fail';
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

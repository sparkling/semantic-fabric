// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  bindExternalEvidence,
  parseAgenticQeEvidence,
  parseRufloEvidence,
} from '../src/evidence.js';

const digest = (character: string) => character.repeat(64);

const rufloEvidence = {
  schemaVersion: 1,
  source: 'ruflo-coordination-ledger',
  taskId: 'task-0001',
  runId: 'run-0001',
  swarmId: 'swarm-0001',
  coordinationTaskId: 'coordination-0001',
  hookIds: ['hook-route-0001', 'hook-post-0001'],
  traceIds: ['trace-0001'],
  routeSnapshotDigest: digest('a'),
  authoritative: false,
  capturedAt: '2026-08-25T12:00:00.000Z',
};

const qeEvidence = {
  schemaVersion: 1,
  source: 'agentic-qe-local-profile',
  profile: 'sast',
  taskId: 'task-0001',
  runId: 'run-0001',
  candidateTree: 'b'.repeat(40),
  commandDigest: digest('c'),
  outputDigest: digest('d'),
  providerVariablesStripped: true,
  authoritative: false,
  capturedAt: '2026-08-25T12:01:00.000Z',
};

describe('external coordination evidence', () => {
  it('accepts strict, non-authoritative Ruflo and task-bound QE evidence', () => {
    const bound = bindExternalEvidence({
      taskId: 'task-0001',
      runId: 'run-0001',
      candidateTree: 'b'.repeat(40),
      ruflo: rufloEvidence,
      qe: [qeEvidence],
    });

    expect(bound.ruflo.swarmId).toBe('swarm-0001');
    expect(bound.qe.map(({ profile }) => profile)).toEqual(['sast']);
    expect(bound.qeDigests).toHaveLength(1);
    expect(bound.qeDigests[0]).toMatch(/^[a-f0-9]{64}$/);
  });

  it('rejects unknown fields, authority claims, provider leakage, and stale identities', () => {
    expect(() => parseRufloEvidence({ ...rufloEvidence, extra: true })).toThrow(/invalid keys/);
    expect(() => parseRufloEvidence({ ...rufloEvidence, authoritative: true })).toThrow(/non-authoritative/);
    expect(() => parseAgenticQeEvidence({
      ...qeEvidence,
      providerVariablesStripped: false,
    })).toThrow(/provider variables/);
    expect(() => bindExternalEvidence({
      taskId: 'task-0001',
      runId: 'run-0001',
      candidateTree: 'e'.repeat(40),
      ruflo: rufloEvidence,
      qe: [qeEvidence],
    })).toThrow(/CANDIDATE_IDENTITY/);
  });

  it('admits only the named local QE profiles', () => {
    expect(() => parseAgenticQeEvidence({
      ...qeEvidence,
      profile: 'ai-autofix',
    })).toThrow(/profile/);
  });
});

// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  bindExternalEvidence,
  parseAgenticQeEvidence,
  parseProgrammeV5RufloEvidence,
  parseRufloEvidence,
  validProgrammeV5RufloBinding,
} from '../src/evidence.js';
import {
  PROGRAMME_V5_RUFLO_CLI_IDENTITY,
  PROGRAMME_V5_RUFLO_CLI_IDENTITY_V3,
  PROGRAMME_V5_RUFLO_MCP_IDENTITY,
  programmeV5RufloCaptureBindingDigest,
  programmeV5RufloRequests,
  programmeV5RufloSnapshotDigest,
} from '../src/programme-v5-ruflo-contract.js';

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

const taskStatus = {
  taskId: 'coordination-0001',
  type: 'feature',
  description: 'Run the schema-v5 programme',
  status: 'in_progress' as const,
  progress: 40,
  priority: 'critical' as const,
  assignedTo: ['codex-root', 'ruflo-evidence'],
  tags: ['schema-v5', 'h0c'],
  createdAt: '2026-08-25T11:58:00.000Z',
  startedAt: '2026-08-25T11:59:00.000Z',
  completedAt: null,
  result: null,
};

const swarmStatus = {
  swarmId: 'swarm-0001',
  status: 'running' as const,
  topology: 'hierarchical' as const,
  maxAgents: 4,
  agentCount: 2,
  taskCount: 1,
  config: {
    topology: 'hierarchical' as const,
    maxAgents: 4,
    strategy: 'specialized' as const,
    communicationProtocol: 'message-bus' as const,
    autoScaling: false,
    consensusMechanism: 'raft' as const,
  },
  createdAt: '2026-08-25T11:57:00.000Z',
  updatedAt: '2026-08-25T12:00:00.000Z',
};

const captureBinding = {
  captureNonce: digest('9'),
  transactionStartedAt: '2026-08-25T12:00:00.000Z',
  taskId: 'task-0001',
  runId: 'run-0001',
  routeSnapshotDigest: digest('a'),
  swarmId: 'swarm-0001',
  coordinationTaskId: 'coordination-0001',
};

const rufloEvidenceV2 = {
  schemaVersion: 2,
  source: 'ruflo-coordination-ledger',
  taskId: 'task-0001',
  runId: 'run-0001',
  swarmId: 'swarm-0001',
  coordinationTaskId: 'coordination-0001',
  hookIds: ['hook-route-0001'],
  traceIds: ['trace-0001'],
  routeSnapshotDigest: digest('a'),
  captureNonce: captureBinding.captureNonce,
  transactionStartedAt: captureBinding.transactionStartedAt,
  captureBindingDigest: programmeV5RufloCaptureBindingDigest(captureBinding),
  mcp: PROGRAMME_V5_RUFLO_MCP_IDENTITY,
  cli: PROGRAMME_V5_RUFLO_CLI_IDENTITY,
  ...programmeV5RufloRequests('coordination-0001', 'swarm-0001'),
  taskStatus,
  taskStatusDigest: programmeV5RufloSnapshotDigest(taskStatus),
  swarmStatus,
  swarmStatusDigest: programmeV5RufloSnapshotDigest(swarmStatus),
  providerVariablesStripped: true,
  authoritative: false,
  capturedAt: '2026-08-25T12:00:01.000Z',
};
const rufloEvidenceV3 = {
  ...rufloEvidenceV2,
  schemaVersion: 3,
  cli: PROGRAMME_V5_RUFLO_CLI_IDENTITY_V3,
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

  it('preserves schemas 1 and 2 while admitting relocatable-source schema 3', () => {
    expect(parseRufloEvidence(rufloEvidence)).toEqual(rufloEvidence);
    expect(parseRufloEvidence(rufloEvidenceV2).schemaVersion).toBe(2);
    expect(parseRufloEvidence(rufloEvidenceV3).schemaVersion).toBe(3);
    const parsed = parseProgrammeV5RufloEvidence(rufloEvidenceV2);
    expect(parsed.taskStatus.taskId).toBe('coordination-0001');
    expect(parsed.swarmStatus.config).toMatchObject({
      topology: 'hierarchical', strategy: 'specialized', consensusMechanism: 'raft',
    });
    expect(Object.isFrozen(parsed.taskStatus)).toBe(true);
    expect(parseProgrammeV5RufloEvidence(rufloEvidenceV3).cli.entryPath)
      .toBe('/runtime/package/bin/mcp-server.js');
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, cli: PROGRAMME_V5_RUFLO_CLI_IDENTITY_V3,
    })).toThrow(/invalid keys/);
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV3, cli: PROGRAMME_V5_RUFLO_CLI_IDENTITY,
    })).toThrow(/invalid keys/);
    expect(() => parseProgrammeV5RufloEvidence(rufloEvidence)).toThrow(/invalid keys/);
  });

  it('rejects synthetic, stale, misbound, and structurally loose schema-2/3 evidence', () => {
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, taskStatus: { ...taskStatus, status: 'pending' },
    })).toThrow(/TASK_STATUS/);
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2,
      swarmStatus: {
        ...swarmStatus,
        config: { ...swarmStatus.config, consensusMechanism: 'majority' },
      },
    })).toThrow(/SWARM_CONFIG/);
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2,
      taskStatusRequest: {
        ...rufloEvidenceV2.taskStatusRequest,
        params: { name: 'task_status', arguments: { taskId: 'coordination-0002' } },
      },
    })).toThrow(/REQUEST_BINDING/);
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, taskStatusDigest: digest('f'),
    })).toThrow(/STATUS_DIGEST/);
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, hookIds: ['hook-route-0001', 'hook-route-0001'],
    })).toThrow(/duplicates/);
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, taskStatus: { ...taskStatus, extra: true },
    })).toThrow(/invalid keys/);
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, authoritative: true,
    })).toThrow(/BOUNDARY/);
  });

  it('requires a nonce-bound capture inside the transaction freshness window', () => {
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, captureNonce: digest('8'),
    })).toThrow(/CAPTURE_BINDING/);
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, capturedAt: '2026-08-25T12:01:00.001Z',
    })).toThrow(/FRESHNESS/);
    const futureTask = { ...taskStatus, startedAt: '2026-08-25T12:00:02.000Z' };
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, taskStatus: futureTask,
      taskStatusDigest: programmeV5RufloSnapshotDigest(futureTask),
    })).toThrow(/FRESHNESS/);
    const futureSwarm = { ...swarmStatus, updatedAt: '2026-08-25T12:00:02.000Z' };
    expect(() => parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, swarmStatus: futureSwarm,
      swarmStatusDigest: programmeV5RufloSnapshotDigest(futureSwarm),
    })).toThrow(/FRESHNESS/);
  });

  it('allows long-lived active task and swarm state captured by a fresh transaction', () => {
    const longTask = {
      ...taskStatus, createdAt: '2020-01-01T00:00:00.000Z', startedAt: '2020-01-01T00:01:00.000Z',
    };
    const longSwarm = {
      ...swarmStatus, createdAt: '2020-01-01T00:00:00.000Z', updatedAt: '2020-01-01T00:01:00.000Z',
    };
    expect(parseProgrammeV5RufloEvidence({
      ...rufloEvidenceV2, taskStatus: longTask, swarmStatus: longSwarm,
      taskStatusDigest: programmeV5RufloSnapshotDigest(longTask),
      swarmStatusDigest: programmeV5RufloSnapshotDigest(longSwarm),
    }).capturedAt).toBe(rufloEvidenceV2.capturedAt);
  });

  it('binds every programme identity and ordered coordination identifier', () => {
    const expected = {
      taskId: 'task-0001', runId: 'run-0001', routeSnapshotDigest: digest('a'),
      swarmId: 'swarm-0001', coordinationTaskId: 'coordination-0001',
      hookIds: ['hook-route-0001'], traceIds: ['trace-0001'],
      transactionStartedAt: captureBinding.transactionStartedAt,
      receiptIssuedAt: '2026-08-25T12:00:02.000Z',
    };
    expect(validProgrammeV5RufloBinding(rufloEvidenceV2, expected)).toBe(true);
    expect(validProgrammeV5RufloBinding(rufloEvidenceV2, {
      ...expected, routeSnapshotDigest: digest('b'),
    })).toBe(false);
    expect(validProgrammeV5RufloBinding(rufloEvidenceV2, {
      ...expected, traceIds: ['trace-other-0001'],
    })).toBe(false);
    expect(validProgrammeV5RufloBinding(rufloEvidenceV2, {
      ...expected, receiptIssuedAt: '2026-08-25T12:00:00.500Z',
    })).toBe(false);
    expect(validProgrammeV5RufloBinding(rufloEvidenceV2, {
      ...expected, transactionStartedAt: '2026-08-25T11:59:59.000Z',
    })).toBe(false);
  });
});

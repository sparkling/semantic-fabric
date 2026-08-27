// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { chmod, link, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  PROGRAMME_V5_RUFLO_CLI_IDENTITY,
  PROGRAMME_V5_RUFLO_NODE_IDENTITY,
  collectProgrammeV5RufloEvidence,
  stableProgrammeV5RufloFileDigest,
} from '../src/programme-v5-ruflo.js';

describe('programme v5 local Ruflo MCP collector', () => {
  it('queries exact persisted task and swarm IDs through the pinned local stdio server', async () => {
    const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-ruflo-'));
    const taskId = 'coordination-test-0001';
    const swarmId = 'swarm-test-0001';
    const now = new Date().toISOString();
    const originalOpenRouter = process.env.OPENROUTER_API_KEY;
    const originalOpenAi = process.env.OPENAI_API_KEY;
    try {
      await mkdir(join(root, '.claude-flow', 'tasks'), { recursive: true });
      await mkdir(join(root, '.claude-flow', 'swarm'), { recursive: true });
      await writeFile(join(root, '.claude-flow', 'tasks', 'store.json'), JSON.stringify({
        version: '3.0.0',
        tasks: {
          [taskId]: {
            taskId,
            type: 'feature',
            description: 'Exercise the real pinned stdio MCP collector',
            priority: 'critical',
            status: 'in_progress',
            progress: 25,
            assignedTo: ['collector-test'],
            tags: ['schema-v5'],
            createdAt: now,
            startedAt: now,
            completedAt: null,
          },
        },
      }));
      await writeFile(join(root, '.claude-flow', 'swarm', 'swarm-state.json'), JSON.stringify({
        version: '3.0.0',
        swarms: {
          [swarmId]: {
            swarmId,
            topology: 'hierarchical',
            maxAgents: 4,
            status: 'running',
            agents: [],
            tasks: [taskId],
            config: {
              topology: 'hierarchical',
              maxAgents: 4,
              strategy: 'specialized',
              communicationProtocol: 'message-bus',
              autoScaling: false,
              consensusMechanism: 'raft',
            },
            createdAt: now,
            updatedAt: now,
          },
        },
      }));
      process.env.OPENROUTER_API_KEY = 'must-not-cross-the-collector-boundary';
      process.env.OPENAI_API_KEY = 'must-not-cross-the-collector-boundary';

      const evidence = await collectProgrammeV5RufloEvidence({
        repositoryRoot: root,
        taskId: 'programme-task-0001',
        runId: 'programme-run-0001',
        routeSnapshotDigest: 'a'.repeat(64),
        captureNonce: 'b'.repeat(64),
        transactionStartedAt: now,
        swarmId,
        coordinationTaskId: taskId,
        hookIds: ['hook-route-0001'],
        traceIds: ['trace-run-0001'],
        timeoutMs: 20_000,
      });

      expect(evidence.schemaVersion).toBe(2);
      expect(evidence.captureNonce).toBe('b'.repeat(64));
      expect(evidence.transactionStartedAt).toBe(now);
      expect(evidence.captureBindingDigest).toMatch(/^[a-f0-9]{64}$/);
      expect(evidence.cli).toEqual(PROGRAMME_V5_RUFLO_CLI_IDENTITY);
      expect(evidence.cli).toMatchObject({
        nodePath: PROGRAMME_V5_RUFLO_NODE_IDENTITY.path,
        nodeDigest: PROGRAMME_V5_RUFLO_NODE_IDENTITY.digest,
      });
      expect(evidence.taskStatus).toMatchObject({ taskId, status: 'in_progress' });
      expect(evidence.swarmStatus).toMatchObject({
        swarmId,
        status: 'running',
        topology: 'hierarchical',
        config: { strategy: 'specialized', consensusMechanism: 'raft' },
      });
      expect(evidence.taskStatusRequest.params.arguments).toEqual({ taskId });
      expect(evidence.swarmStatusRequest.params.arguments).toEqual({ swarmId });
      expect(evidence.taskStatusDigest).toMatch(/^[a-f0-9]{64}$/);
      expect(evidence.swarmStatusDigest).toMatch(/^[a-f0-9]{64}$/);
      expect(evidence.providerVariablesStripped).toBe(true);
      expect(evidence.authoritative).toBe(false);
    } finally {
      if (originalOpenRouter === undefined) delete process.env.OPENROUTER_API_KEY;
      else process.env.OPENROUTER_API_KEY = originalOpenRouter;
      if (originalOpenAi === undefined) delete process.env.OPENAI_API_KEY;
      else process.env.OPENAI_API_KEY = originalOpenAi;
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('rejects writable and multiply linked local runtime files', async () => {
    const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-ruflo-file-'));
    const file = join(root, 'entry.js');
    try {
      await writeFile(file, 'process.exit(0);\n', { mode: 0o700 });
      expect(stableProgrammeV5RufloFileDigest(file, true)).toBe(
        createHash('sha256').update('process.exit(0);\n').digest('hex'),
      );
      await chmod(file, 0o720);
      expect(() => stableProgrammeV5RufloFileDigest(file, true)).toThrow(/LOCAL_FILE_UNTRUSTED/);
      await chmod(file, 0o700);
      await link(file, join(root, 'entry-hardlink.js'));
      expect(() => stableProgrammeV5RufloFileDigest(file, true)).toThrow(/LOCAL_FILE_UNTRUSTED/);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});

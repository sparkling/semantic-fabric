// SPDX-License-Identifier: MIT

import {
  existsSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { checkEvolutionEligibility } from '../src/config.js';
import { parseHarnessConfig, parseTaskContract } from '../src/contracts.js';
import {
  HarnessPolicy,
  assertProtectedInputSnapshot,
  auditMutableOutputs,
  captureProtectedInputs,
  listTrackedPaths,
  verifyProtectedInputs,
} from '../src/policy.js';
import { createTask, createTestConfig, initializeGitWorkspace } from './helpers.js';

const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function workspace(): string {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-policy-'));
  temporaryRoots.push(root);
  initializeGitWorkspace(root, { 'protected.txt': 'oracle\n', 'read.txt': 'source\n' });
  return root;
}

describe('strict contracts', () => {
  it('rejects unknown fields, traversal, overlap, and undeclared commands', () => {
    const root = workspace();
    const config = createTestConfig();
    const valid = {
      schemaVersion: 1,
      taskId: 'task-0001',
      runId: 'run-0001',
      workspaceRoot: root,
      readablePaths: ['read.txt'],
      mutablePaths: ['new.txt'],
      protectedPaths: ['protected.txt'],
      tools: ['git'],
      commands: [],
      network: { mode: 'offline', allowedOrigins: [] },
      authority: 'development-only-no-promotion',
    };
    expect(() => parseTaskContract({ ...valid, surprise: true }, config)).toThrow(/invalid keys/);
    expect(() => parseTaskContract({ ...valid, mutablePaths: ['../escape'] }, config)).toThrow(/traversal/);
    expect(() => parseTaskContract({ ...valid, mutablePaths: ['protected.txt'] }, config)).toThrow(/overlaps/);
    expect(() => parseTaskContract({
      ...valid,
      commands: [{
        tool: 'node', executable: 'node', argv: ['--version'], cwd: '.', env: {},
        timeoutMs: 100, maxOutputBytes: 100,
      }],
    }, config)).toThrow(/not declared/);
  });

  it('rejects private registry config and preserves the 5+5 evolution threshold', () => {
    const config = createTestConfig();
    expect(() => parseHarnessConfig({ ...config, approvedRegistry: 'http://10.0.0.2:4873/' })).toThrow(/HTTPS origin/);
    expect(() => parseHarnessConfig({ ...config, approvedRegistry: 'https://packages.example.org/' })).toThrow(
      /registry.npmjs.org/,
    );
    expect(checkEvolutionEligibility(['train001'], ['holdout1']).eligible).toBe(false);
    expect(checkEvolutionEligibility(
      ['train001', 'train002', 'train003', 'train004', 'train005'],
      ['holdout1', 'holdout2', 'holdout3', 'holdout4', 'holdout5'],
    ).eligible).toBe(true);
    expect(checkEvolutionEligibility(
      ['shared00', 'train002', 'train003', 'train004', 'train005'],
      ['shared00', 'holdout2', 'holdout3', 'holdout4', 'holdout5'],
    ).eligible).toBe(false);
  });
});

describe('five policy gates', () => {
  it('admits only a declared mutable path, tool, offline action, and fixed authority', () => {
    const root = workspace();
    const config = createTestConfig();
    const policy = new HarnessPolicy(createTask(root, config), config);
    const decision = policy.evaluate({
      kind: 'write',
      tool: 'apply_patch',
      path: 'new.txt',
      origin: null,
      authority: 'development-only-no-promotion',
    });
    expect(decision.allow).toBe(true);
    expect(Object.keys(decision.gates).sort()).toEqual(
      ['authority', 'network', 'path', 'protectedInput', 'tool'].sort(),
    );
    expect(Object.values(decision.gates).every((gate) => gate.allow)).toBe(true);
  });

  it('fails closed for protected writes, undeclared tools, network, and promotion', () => {
    const root = workspace();
    const config = createTestConfig();
    const policy = new HarnessPolicy(createTask(root, config), config);
    const protectedWrite = policy.evaluate({
      kind: 'write', tool: 'apply_patch', path: 'protected.txt', origin: null,
      authority: 'development-only-no-promotion',
    });
    expect(protectedWrite.allow).toBe(false);
    expect(protectedWrite.gates.path.allow).toBe(false);
    expect(protectedWrite.gates.protectedInput.allow).toBe(false);

    const network = policy.evaluate({
      kind: 'network', tool: 'curl', path: null, origin: 'https://api.openai.com',
      authority: 'promotion-authority',
    });
    expect(network.allow).toBe(false);
    expect(network.gates.tool.allow).toBe(false);
    expect(network.gates.network.allow).toBe(false);
    expect(network.gates.authority.allow).toBe(false);

    const promotion = policy.evaluate({
      kind: 'promote', tool: 'publish', path: null, origin: null,
      authority: 'development-only-no-promotion',
    });
    expect(promotion.gates.authority.allow).toBe(false);
  });

  it('rejects symlinks and hardlinks instead of guessing their boundary', () => {
    const root = workspace();
    symlinkSync('read.txt', join(root, 'link.txt'));
    linkSync(join(root, 'protected.txt'), join(root, 'hardlink.txt'));
    const config = createTestConfig();
    const symlinkPolicy = new HarnessPolicy(createTask(root, config, { readablePaths: ['link.txt'] }), config);
    expect(symlinkPolicy.evaluate({
      kind: 'read', tool: 'read_file', path: 'link.txt', origin: null,
      authority: 'development-only-no-promotion',
    }).gates.path.allow).toBe(false);

    const hardlinkPolicy = new HarnessPolicy(createTask(root, config, { mutablePaths: ['hardlink.txt'] }), config);
    expect(hardlinkPolicy.evaluate({
      kind: 'write', tool: 'write_file', path: 'hardlink.txt', origin: null,
      authority: 'development-only-no-promotion',
    }).gates.path.allow).toBe(false);
  });
});

describe('protected and mutable evidence', () => {
  it('requires the exact declared protected path set and valid digests', () => {
    const root = workspace();
    const config = createTestConfig();
    const task = createTask(root, config);
    expect(() => assertProtectedInputSnapshot(task, {})).toThrow(/PATH_SET_MISMATCH/);
    expect(() => assertProtectedInputSnapshot(task, { 'protected.txt': 'not-a-digest' })).toThrow(
      /DIGEST_INVALID/,
    );
  });

  it('digests only explicit tracked protected paths and detects changes', async () => {
    const root = workspace();
    const config = createTestConfig();
    const task = createTask(root, config);
    const snapshot = await captureProtectedInputs(task, config);
    expect(Object.keys(snapshot)).toEqual(['protected.txt']);
    expect(snapshot['protected.txt']).toMatch(/^[a-f0-9]{64}$/);
    writeFileSync(join(root, 'protected.txt'), 'tampered\n');
    expect((await verifyProtectedInputs(task, config, snapshot)).allow).toBe(false);
  });

  it('hard-fails when a listed protected input is untracked', async () => {
    const root = workspace();
    writeFileSync(join(root, 'untracked.txt'), 'not indexed\n');
    const config = createTestConfig(['untracked.txt']);
    const task = createTask(root, config, { protectedPaths: ['untracked.txt'] });
    await expect(captureProtectedInputs(task, config)).rejects.toThrow(/not tracked/);
  });

  it('uses the pinned Git executable when PATH begins with a fake git', async () => {
    const root = workspace();
    const fakeBin = join(root, 'fake-bin');
    const sentinel = join(root, 'fake-git-invoked');
    mkdirSync(fakeBin, { mode: 0o700 });
    writeFileSync(
      join(fakeBin, 'git'),
      '#!/bin/sh\nprintf invoked > fake-git-invoked\nexit 99\n',
      { mode: 0o700 },
    );
    const originalPath = process.env.PATH;
    process.env.PATH = `${fakeBin}:${originalPath ?? ''}`;
    try {
      const config = createTestConfig();
      const task = createTask(root, config);
      await expect(captureProtectedInputs(task, config)).resolves.toHaveProperty('protected.txt');
      await expect(listTrackedPaths(task, config)).resolves.toContain('protected.txt');
      expect(existsSync(sentinel)).toBe(false);
    } finally {
      if (originalPath === undefined) delete process.env.PATH;
      else process.env.PATH = originalPath;
    }
  });

  it('rejects newly-created files over 500 lines', async () => {
    const root = workspace();
    const config = createTestConfig();
    const task = createTask(root, config);
    const tracked = await listTrackedPaths(task, config);
    writeFileSync(join(root, 'new.txt'), 'line\n'.repeat(501));
    const decision = auditMutableOutputs(task, config, tracked);
    expect(decision.allow).toBe(false);
    expect(decision.reasons.join(' ')).toMatch(/exceeds 500 lines/);
  });
});

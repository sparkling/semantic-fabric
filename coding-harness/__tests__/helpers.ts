// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import type { HarnessConfig, TaskContract } from '../src/contracts.js';
import { parseHarnessConfig, parseTaskContract } from '../src/contracts.js';

export function createTestConfig(requiredProtectedPaths = ['protected.txt']): HarnessConfig {
  return parseHarnessConfig({
    schemaVersion: 1,
    authority: 'development-only-no-promotion',
    approvedRegistry: 'https://registry.npmjs.org/',
    firstPartyOrigins: ['https://api.openai.com', 'https://api.anthropic.com'],
    allowedTools: ['read_file', 'write_file', 'apply_patch', 'git', 'node', 'npm'],
    requiredProtectedPaths,
    environment: {
      allow: [
        'PATH', 'HOME', 'TMPDIR', 'LANG', 'SAFE_FLAG', 'CARGO_HOME', 'CARGO_INCREMENTAL',
        'CARGO_NET_OFFLINE', 'CARGO_TARGET_DIR', 'OPENAI_API_KEY',
        'OPENROUTER_API_KEY', 'HTTP_PROXY', 'OPENAI_BASE_URL',
      ],
      denyExact: ['OPENAI_API_KEY', 'OPENROUTER_API_KEY', 'HTTP_PROXY'],
      denyPrefixes: ['OPENROUTER_'],
      denySuffixes: ['_BASE_URL', '_PROXY'],
    },
    limits: {
      maxTimeoutMs: 3_000,
      maxOutputBytes: 100_000,
      maxNewFileLines: 500,
      terminationGraceMs: 25,
    },
    evolution: {
      minimumTrainingTasks: 5,
      minimumSealedHoldouts: 5,
    },
  });
}

export function createTask(
  workspaceRoot: string,
  config: HarnessConfig,
  overrides: Partial<Record<keyof TaskContract, unknown>> = {},
): TaskContract {
  return parseTaskContract({
    schemaVersion: 1,
    taskId: 'task-0001',
    runId: 'run-0001',
    workspaceRoot,
    readablePaths: ['read.txt'],
    mutablePaths: ['new.txt'],
    protectedPaths: config.requiredProtectedPaths,
    tools: ['read_file', 'write_file', 'apply_patch', 'git', 'node', 'npm'],
    commands: [],
    network: { mode: 'offline', allowedOrigins: [] },
    authority: 'development-only-no-promotion',
    ...overrides,
  }, config);
}

export function initializeGitWorkspace(root: string, files: Record<string, string>): void {
  for (const [path, content] of Object.entries(files)) {
    const slash = path.lastIndexOf('/');
    if (slash !== -1) mkdirSync(`${root}/${path.slice(0, slash)}`, { recursive: true });
    writeFileSync(`${root}/${path}`, content);
  }
  runGit(root, ['init', '--quiet']);
  runGit(root, ['config', 'user.email', 'harness@example.invalid']);
  runGit(root, ['config', 'user.name', 'Harness Test']);
  runGit(root, ['add', '--', ...Object.keys(files)]);
}

function runGit(root: string, argv: string[]): void {
  const result = spawnSync('git', ['-C', root, ...argv], { encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr || `git ${argv[0]} failed`);
}

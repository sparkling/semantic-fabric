// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  assertConfigurationGitSnapshot,
  captureConfigurationGitSnapshot,
  configurationFileProvenance,
} from '../src/effective-config-git.js';

const roots: string[] = [];
const originalPath = process.env.PATH;
const originalIndex = process.env.GIT_INDEX_FILE;
const GIT_ENVIRONMENT = Object.freeze({
  PATH: '/usr/bin:/bin',
  HOME: '/nonexistent',
  LANG: 'C',
  LC_ALL: 'C',
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_CONFIG_GLOBAL: '/dev/null',
});

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  restoreEnvironment('PATH', originalPath);
  restoreEnvironment('GIT_INDEX_FILE', originalIndex);
});

describe('effective configuration Git provenance', () => {
  it('binds canonical Git, repository, HEAD, and index while comparing raw blobs', () => {
    const root = repository();
    const config = join(root, '.mcp.json');
    const snapshot = captureConfigurationGitSnapshot(root);

    expect(snapshot).toMatchObject({ repositoryRoot: root });
    expect(snapshot.headCommit).toMatch(/^[a-f0-9]{40}$/);
    expect(snapshot.indexTree).toMatch(/^[a-f0-9]{64}$/);
    expect(snapshot.gitExecutableDigest).toMatch(/^[a-f0-9]{64}$/);
    expect(snapshot.digest).toBe(createHash('sha256').update(JSON.stringify([
      root, snapshot.headCommit, snapshot.indexTree, snapshot.gitExecutableDigest,
    ])).digest('hex'));
    expect(configurationFileProvenance(root, '.mcp.json', readFileSync(config), snapshot))
      .toEqual({ provenance: 'tracked-clean', trustworthy: true });

    writeFileSync(config, '{"mcpServers":{"changed":{}}}\n');
    expect(configurationFileProvenance(root, '.mcp.json', readFileSync(config), snapshot))
      .toEqual({ provenance: 'tracked-dirty', trustworthy: true });

    writeFileSync(join(root, 'ignored.json'), '{}\n');
    writeFileSync(join(root, 'untracked.json'), '{}\n');
    expect(configurationFileProvenance(
      root, 'ignored.json', readFileSync(join(root, 'ignored.json')), snapshot,
    )).toEqual({ provenance: 'ignored', trustworthy: true });
    expect(configurationFileProvenance(
      root, 'untracked.json', readFileSync(join(root, 'untracked.json')), snapshot,
    )).toEqual({ provenance: 'untracked', trustworthy: true });
  });

  it('classifies staged content as dirty against HEAD', () => {
    const root = repository();
    const config = join(root, '.mcp.json');
    writeFileSync(config, '{"mcpServers":{"staged":{}}}\n');
    git(root, ['add', '--', '.mcp.json']);
    const snapshot = captureConfigurationGitSnapshot(root);

    expect(configurationFileProvenance(root, '.mcp.json', readFileSync(config), snapshot))
      .toEqual({ provenance: 'tracked-dirty', trustworthy: true });
  });

  it('ignores hostile PATH and GIT_INDEX_FILE values', () => {
    const root = repository();
    const hostile = join(root, 'hostile');
    const marker = join(root, 'hostile-git-ran');
    mkdirSync(hostile);
    const fakeGit = join(hostile, 'git');
    writeFileSync(fakeGit, `#!/bin/sh\ntouch ${JSON.stringify(marker)}\nexit 99\n`);
    chmodSync(fakeGit, 0o755);
    const fakeIndex = join(root, 'attacker.index');
    writeFileSync(fakeIndex, 'not a Git index');
    process.env.PATH = hostile;
    process.env.GIT_INDEX_FILE = fakeIndex;

    const snapshot = captureConfigurationGitSnapshot(root);
    expect(configurationFileProvenance(
      root, '.mcp.json', readFileSync(join(root, '.mcp.json')), snapshot,
    )).toEqual({ provenance: 'tracked-clean', trustworthy: true });
    expect(existsSync(marker)).toBe(false);
  });

  it.each([
    ['assume-unchanged', '--assume-unchanged'],
    ['skip-worktree', '--skip-worktree'],
  ])('rejects the %s index flag', (_label, flag) => {
    const root = repository();
    git(root, ['update-index', flag, '--', '.mcp.json']);
    const snapshot = captureConfigurationGitSnapshot(root);

    expect(() => configurationFileProvenance(
      root, '.mcp.json', readFileSync(join(root, '.mcp.json')), snapshot,
    )).toThrow('HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_FLAGS_UNSAFE');
  });

  it('rejects non-regular index entries', () => {
    const root = repository();
    rmSync(join(root, '.mcp.json'));
    symlinkSync('target.json', join(root, '.mcp.json'));
    git(root, ['add', '--', '.mcp.json']);
    const snapshot = captureConfigurationGitSnapshot(root);

    expect(() => configurationFileProvenance(
      root, '.mcp.json', Buffer.from('target.json'), snapshot,
    )).toThrow('HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_ENTRY_SPECIAL');
  });

  it('rejects unmerged indexes and detects a post-capture index change', () => {
    const root = repository();
    const snapshot = captureConfigurationGitSnapshot(root);
    git(root, ['update-index', '--assume-unchanged', '--', '.mcp.json']);
    expect(() => assertConfigurationGitSnapshot(root, snapshot))
      .toThrow('HARNESS_EFFECTIVE_CONFIG_GIT_SNAPSHOT_CHANGED');

    git(root, ['update-index', '--no-assume-unchanged', '--', '.mcp.json']);
    git(root, ['checkout', '-q', '-b', 'other']);
    writeFileSync(join(root, '.mcp.json'), '{"branch":"other"}\n');
    git(root, ['add', '--', '.mcp.json']);
    git(root, ['commit', '--quiet', '-m', 'other']);
    git(root, ['checkout', '-q', 'master']);
    writeFileSync(join(root, '.mcp.json'), '{"branch":"master"}\n');
    git(root, ['add', '--', '.mcp.json']);
    git(root, ['commit', '--quiet', '-m', 'master']);
    git(root, ['merge', 'other'], [1]);

    expect(() => captureConfigurationGitSnapshot(root))
      .toThrow('HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_UNMERGED');
  });
});

function repository(): string {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-config-git-'));
  roots.push(root);
  git(root, ['init', '--quiet', '--initial-branch=master']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  writeFileSync(join(root, '.gitignore'), 'ignored.json\n');
  writeFileSync(join(root, '.mcp.json'), '{"mcpServers":{}}\n');
  git(root, ['add', '--', '.gitignore', '.mcp.json']);
  git(root, ['commit', '--quiet', '-m', 'configuration']);
  return root;
}

function git(cwd: string, args: string[], acceptedStatuses: readonly number[] = [0]): void {
  const result = spawnSync('/usr/bin/git', args, {
    cwd,
    env: GIT_ENVIRONMENT,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  if (result.status === null || !acceptedStatuses.includes(result.status)) {
    throw new Error(result.stderr || `git exited ${String(result.status)}`);
  }
}

function restoreEnvironment(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}

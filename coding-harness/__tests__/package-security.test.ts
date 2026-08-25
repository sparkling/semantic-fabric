// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

type LockPackage = {
  integrity?: string;
  link?: boolean;
  resolved?: string;
  version?: string;
};

type Lockfile = {
  lockfileVersion: number;
  packages: Record<string, LockPackage>;
};

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const packageJson = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8')) as Record<string, unknown>;
const lockfile = JSON.parse(readFileSync(resolve(root, 'package-lock.json'), 'utf8')) as Lockfile;
const manifest = JSON.parse(readFileSync(resolve(root, '.harness/manifest.json'), 'utf8')) as Record<string, unknown>;

const expectedRuntime = {
  '@metaharness/harness': '0.2.0',
  '@metaharness/host-claude-code': '0.1.2',
  '@metaharness/host-codex': '0.1.2',
  '@metaharness/router': '0.4.0',
};

describe('private package boundary', () => {
  it('has no package publication or executable surface', () => {
    expect(packageJson.private).toBe(true);
    expect(packageJson).not.toHaveProperty('bin');
    expect(packageJson).not.toHaveProperty('files');
    expect(packageJson).not.toHaveProperty('exports');
    expect(packageJson).not.toHaveProperty('publishConfig');
    expect(packageJson.scripts).toMatchObject({ prepublishOnly: 'node scripts/deny-publish.mjs' });
  });

  it('pins the verified runtime packages exactly', () => {
    expect(packageJson.dependencies).toEqual(expectedRuntime);
    expect(packageJson.dependencies).not.toHaveProperty('@metaharness/kernel');
    expect(packageJson.devDependencies).not.toHaveProperty('@metaharness/darwin');
    for (const version of Object.values({
      ...(packageJson.dependencies as object),
      ...(packageJson.devDependencies as object),
    })) {
      expect(version).toMatch(/^\d+\.\d+\.\d+$/);
    }
  });

  it('has no executable evolution path before the evidence gate', () => {
    const scripts = packageJson.scripts as Record<string, string>;
    expect(Object.keys(scripts).some((name) => name.startsWith('evolve'))).toBe(false);
    expect(packageJson.devDependencies).not.toHaveProperty('@metaharness/darwin');
    expect(manifest.evolution).toEqual({
      eligible: false,
      minimumTrainingTasks: 5,
      minimumSealedHoldouts: 5,
      suiteFile: null,
    });
    const trackedSuite = spawnSync(
      'git',
      ['ls-files', '--', 'coding-harness/suite.json'],
      { cwd: resolve(root, '..'), encoding: 'utf8' },
    );
    expect(trackedSuite.status).toBe(0);
    expect(trackedSuite.stdout).toBe('');
    expect(existsSync(resolve(root, 'suite.json'))).toBe(false);
    expect(existsSync(resolve(root, '.metaharness'))).toBe(false);
  });
});

describe('lockfile supply chain', () => {
  it('uses only integrity-pinned public HTTPS registry artifacts', () => {
    expect(lockfile.lockfileVersion).toBe(3);
    const fetched = Object.entries(lockfile.packages).filter(([, entry]) => entry.resolved !== undefined);
    expect(fetched.length).toBeGreaterThan(0);

    for (const [path, entry] of fetched) {
      const url = new URL(entry.resolved as string);
      expect(url.protocol, path).toBe('https:');
      expect(url.hostname, path).toBe('registry.npmjs.org');
      expect(url.port, path).toBe('');
      expect(url.username, path).toBe('');
      expect(url.password, path).toBe('');
      expect(entry.integrity, path).toMatch(/^sha512-[A-Za-z0-9+/]+={0,2}$/);
    }
  });
});

describe('verified package exports', () => {
  it('loads the pinned harness, router, and both native host adapters', async () => {
    const [harness, router, claude, codex] = await Promise.all([
      import('@metaharness/harness'),
      import('@metaharness/router'),
      import('@metaharness/host-claude-code'),
      import('@metaharness/host-codex'),
    ]);
    expect(harness).toHaveProperty('PolicyGate');
    expect(harness).toHaveProperty('HarnessKernel');
    expect(router).toHaveProperty('Router');
    expect(claude.default.name).toBe('claude-code');
    expect(codex.default.name).toBe('codex');
  });
});

// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

type LockPackage = {
  dependencies?: Record<string, string>;
  dev?: boolean;
  devDependencies?: Record<string, string>;
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
  '@metaharness/harness': 'latest',
  '@metaharness/host-claude-code': 'latest',
  '@metaharness/host-codex': 'latest',
  '@metaharness/router': 'latest',
};

const expectedReadinessPackages = {
  'node_modules/metaharness': {
    version: '0.4.16',
    resolved: 'https://registry.npmjs.org/metaharness/-/metaharness-0.4.16.tgz',
    integrity: 'sha512-dfpuU2pqZow4mi7WYQLRs4z914v8fpA7lWh5iw2lNl6yinu+iKsFFbQheVKZULDAmvE8/LxiLg9Rx48iFsEeaQ==',
    dev: true,
  },
  'node_modules/@metaharness/darwin': {
    version: '0.10.2',
    resolved: 'https://registry.npmjs.org/@metaharness/darwin/-/darwin-0.10.2.tgz',
    integrity: 'sha512-Glczy9YJDLf5x4HlfVuQVbZuPuue45K+8ohfLixZPJ18oc0Q8RR+BcsopHUQsL4hBkvs3YPXUyZ16piDJQp4gw==',
    dev: true,
  },
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

  it('tracks the verified runtime packages through latest tags', () => {
    expect(packageJson.dependencies).toEqual(expectedRuntime);
    expect(packageJson.dependencies).not.toHaveProperty('@metaharness/kernel');
    expect(packageJson.devDependencies).not.toHaveProperty('@metaharness/darwin');
    expect(packageJson.devDependencies).toMatchObject({ vite: 'latest', vitest: 'latest' });
    for (const version of Object.values({
      ...(packageJson.dependencies as object),
      ...(packageJson.devDependencies as object),
    })) {
      expect(version).toBe('latest');
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
    const metaharnessRoot = resolve(root, '.metaharness');
    if (existsSync(metaharnessRoot)) {
      expect(readdirSync(metaharnessRoot).sort()).toEqual(['runs']);
    }
    for (const path of ['archive.json', 'lineage.json', 'variants', 'reports']) {
      expect(existsSync(resolve(metaharnessRoot, path))).toBe(false);
    }
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

  it('binds the readiness CLI and Darwin engine to reviewed registry artifacts', () => {
    expect(packageJson.devDependencies).toHaveProperty('metaharness', 'latest');
    expect(lockfile.packages['']?.devDependencies).toHaveProperty('metaharness', 'latest');
    for (const [path, expected] of Object.entries(expectedReadinessPackages)) {
      expect(lockfile.packages[path], path).toMatchObject(expected);
    }
    expect(lockfile.packages['node_modules/metaharness']?.dependencies)
      .toHaveProperty('@metaharness/darwin', '^0.10.0');
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

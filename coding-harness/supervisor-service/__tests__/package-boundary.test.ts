// SPDX-License-Identifier: MIT

import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

describe('private supervisor-service package boundary', () => {
  it('requires the Linux x64 GNU Rollup binary in the clean-install closure', () => {
    const packageJson = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8')) as {
      devDependencies?: Record<string, string>;
    };
    const lock = JSON.parse(readFileSync(resolve(root, 'package-lock.json'), 'utf8')) as {
      packages: Record<string, {
        cpu?: string[];
        dev?: boolean;
        devDependencies?: Record<string, string>;
        libc?: string[];
        optional?: boolean;
        os?: string[];
        version?: string;
      }>;
    };
    const expectedVersion = '4.63.1';
    const nativePackage = lock.packages['node_modules/@rollup/rollup-linux-x64-gnu'];

    expect(packageJson.devDependencies)
      .toHaveProperty('@rollup/rollup-linux-x64-gnu', expectedVersion);
    expect(lock.packages['']?.devDependencies)
      .toHaveProperty('@rollup/rollup-linux-x64-gnu', expectedVersion);
    expect(nativePackage).toMatchObject({
      version: expectedVersion,
      dev: true,
      cpu: ['x64'],
      os: ['linux'],
    });
    // npm 9.6.4 reads the wrong Node 20 report key and rejects a required glibc lock entry.
    expect(nativePackage).not.toHaveProperty('libc');
    expect(nativePackage?.optional).not.toBe(true);
  });

  it('owns a private package and nonoperational service manifest', () => {
    const packagePath = resolve(root, 'package.json');
    const manifestPath = resolve(root, '.service/manifest.json');

    expect(existsSync(packagePath)).toBe(true);
    expect(existsSync(manifestPath)).toBe(true);

    const packageJson = JSON.parse(readFileSync(packagePath, 'utf8')) as Record<string, unknown>;
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as Record<string, unknown>;

    expect(packageJson).toMatchObject({
      name: 'semantic-fabric-supervisor-service',
      version: '0.1.0',
      description: 'Private, non-deployable ADR-0042 development/evidence oracle',
      private: true,
      type: 'module',
      engines: { node: '>=20.0.0' },
    });
    expect(packageJson).not.toHaveProperty('bin');
    expect(packageJson).not.toHaveProperty('files');
    expect(packageJson).not.toHaveProperty('exports');
    expect(packageJson).not.toHaveProperty('publishConfig');
    expect(packageJson).not.toHaveProperty('workspaces');
    expect(packageJson.dependencies).toEqual({});
    expect(packageJson.devDependencies).toEqual({
      '@rollup/rollup-linux-x64-gnu': '4.63.1',
      '@types/node': '20.19.43',
      esbuild: '0.28.2',
      typescript: '5.9.3',
      vite: '6.4.3',
      vitest: '3.2.7',
    });
    expect(packageJson.scripts).toEqual({
      build: 'tsc --noEmit && node scripts/build.mjs',
      test: 'vitest run',
      verify: 'npm run build && npm test && node scripts/verify-artifact.mjs',
      prepublishOnly: 'node scripts/deny-publish.mjs',
    });

    expect(manifest).toEqual({
      schemaVersion: 1,
      serviceKind: 'programme-capture-supervisor-service-v1',
      authority: 'nonoperational-proposed-adr-0042',
      sourceEntrypoint: 'src/index.ts',
      bundlePath: 'dist/supervisor-service.mjs',
      operational: false,
      externallyReachable: false,
      registrationMutationsEnabled: false,
      databaseAccessEnabled: false,
      signerAccessEnabled: false,
      publicationAccessEnabled: false,
      parentRuntimeDependencyAllowed: false,
    });
  });

  it('owns an integrity-pinned lock without links or local dependency URLs', () => {
    const lock = JSON.parse(readFileSync(resolve(root, 'package-lock.json'), 'utf8')) as {
      lockfileVersion: number;
      packages: Record<string, {
        integrity?: string;
        link?: boolean;
        resolved?: string;
      }>;
    };

    expect(lock.lockfileVersion).toBe(3);
    for (const [path, entry] of Object.entries(lock.packages)) {
      expect(entry.link, path).not.toBe(true);
      if (entry.resolved !== undefined) {
        const resolved = new URL(entry.resolved);
        expect(resolved.protocol, path).toBe('https:');
        expect(resolved.hostname, path).toBe('registry.npmjs.org');
        expect(entry.integrity, path).toMatch(/^sha512-[A-Za-z0-9+/]+={0,2}$/);
      }
    }
  });
});

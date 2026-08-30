// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { parseControllerBuildManifest } from '../src/controller-build.js';

const buildPath = new URL('../.harness/controller-build.json', import.meta.url);
const launcherPath = new URL('../dist/native-proxy-launcher.cjs', import.meta.url);
const harnessManifestPath = new URL('../.harness/manifest.json', import.meta.url);
const lockfilePath = new URL('../package-lock.json', import.meta.url);

describe('sealed controller build manifest', () => {
  it('binds the runtime entry, exact outputs, and production dependency files', () => {
    const build = parseControllerBuildManifest(JSON.parse(readFileSync(buildPath, 'utf8')));
    expect(build.runtimeEntry).toBe('coding-harness/dist/issue-8-program.js');
    expect(build.outputs['coding-harness/dist/native-proxy-launcher.cjs'])
      .toMatch(/^[a-f0-9]{64}$/);
    expect(Object.keys(build.outputs).length).toBeGreaterThan(50);
    expect(Object.keys(build.productionFiles).length).toBeGreaterThan(50);
    expect(build.runtimeTreeDigest).toMatch(/^[a-f0-9]{64}$/);
    expect(build.harnessManifestDigest).toBe(sha256(readFileSync(harnessManifestPath)));
    expect(build.lockfileDigest).toBe(sha256(readFileSync(lockfilePath)));
  });

  it('rejects an output mutation or an ambient dependency path', () => {
    const original = JSON.parse(readFileSync(buildPath, 'utf8'));
    const output = Object.keys(original.outputs)[0];
    expect(() => parseControllerBuildManifest({
      ...original,
      outputs: { ...original.outputs, [output]: 'f'.repeat(64) },
    })).toThrow(/TREE_DIGEST_MISMATCH/);
    expect(() => parseControllerBuildManifest({
      ...original,
      productionFiles: { '../ambient.js': 'a'.repeat(64) },
    })).toThrow(/PRODUCTION_PATH_INVALID/);
  });

  it('executes the CommonJS launcher outside a package boundary', () => {
    const directory = mkdtempSync(join(tmpdir(), 'semantic-fabric-launcher-'));
    const isolatedLauncher = join(directory, 'native-proxy-launcher.cjs');
    try {
      copyFileSync(launcherPath, isolatedLauncher);
      const result = spawnSync(process.execPath, [isolatedLauncher], {
        cwd: directory,
        encoding: 'utf8',
      });
      expect(result.status).toBe(1);
      expect(result.signal).toBeNull();
      expect(result.stderr).toContain('HARNESS_NATIVE_PROXY_LAUNCH_ARGUMENT_INVALID');
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

function sha256(bytes: Buffer): string {
  return createHash('sha256').update(bytes).digest('hex');
}

// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parseControllerBuildManifest } from '../src/controller-build.js';

const buildPath = new URL('../.harness/controller-build.json', import.meta.url);

describe('sealed controller build manifest', () => {
  it('binds the runtime entry, exact outputs, and production dependency files', () => {
    const build = parseControllerBuildManifest(JSON.parse(readFileSync(buildPath, 'utf8')));
    expect(build.runtimeEntry).toBe('coding-harness/dist/issue-8-program.js');
    expect(Object.keys(build.outputs).length).toBeGreaterThan(50);
    expect(Object.keys(build.productionFiles).length).toBeGreaterThan(50);
    expect(build.runtimeTreeDigest).toMatch(/^[a-f0-9]{64}$/);
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
});

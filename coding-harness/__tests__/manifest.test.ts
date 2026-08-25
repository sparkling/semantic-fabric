// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { parseHarnessManifest } from '../src/manifest.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(readFileSync(resolve(root, '.harness/manifest.json'), 'utf8')) as unknown;

describe('canonical harness manifest', () => {
  it('matches the protected runtime config and exposes the actual coordination surface', () => {
    const parsed = parseHarnessManifest(manifest, SECURE_HARNESS_CONFIG);
    expect(parsed.coordinationSurface).toBe('.mcp.json');
    expect(parsed.diagnostics.minimumScore).toBe(98);
    expect(parsed.evolution).toMatchObject({ eligible: false, suiteFile: null });
  });

  it('rejects a reduced protected set or synthetic clean diagnostic', () => {
    expect(() => parseHarnessManifest({
      ...(manifest as object),
      protectedPaths: ['coding-harness/package.json'],
    }, SECURE_HARNESS_CONFIG)).toThrow('HARNESS_MANIFEST_PROTECTED_PATHS_MISMATCH');
    expect(() => parseHarnessManifest({
      ...(manifest as object),
      diagnostics: { minimumScore: 98, blindSurfaceOutcome: 'CLEAN' },
    }, SECURE_HARNESS_CONFIG)).toThrow(/diagnostic gates/);
  });
});

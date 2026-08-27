// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import {
  PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
  PROGRAMME_V5_CONTROLLER_REQUIRED_PATHS,
  controllerExecutionPaths,
} from '../src/controller-attestation.js';
import {
  normalizeAcceptanceTaskPath,
  parseHarnessManifest,
  selectAcceptanceTaskPath,
} from '../src/manifest.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(readFileSync(resolve(root, '.harness/manifest.json'), 'utf8')) as unknown;

describe('canonical harness manifest', () => {
  it('matches the protected runtime config and exposes the actual coordination surface', () => {
    const parsed = parseHarnessManifest(manifest, SECURE_HARNESS_CONFIG);
    expect(parsed.coordinationSurface).toBe('.mcp.json');
    expect(parsed.diagnostics.programmeAcceptanceThreshold).toBe(98);
    expect(parsed.evolution).toMatchObject({ eligible: false, suiteFile: null });
  });

  it('rejects a reduced protected set or synthetic clean diagnostic', () => {
    expect(() => parseHarnessManifest({
      ...(manifest as object),
      protectedPaths: ['coding-harness/package.json'],
    }, SECURE_HARNESS_CONFIG)).toThrow('HARNESS_MANIFEST_PROTECTED_PATHS_MISMATCH');
    expect(() => parseHarnessManifest({
      ...(manifest as object),
      diagnostics: {
        programmeAcceptanceThreshold: 98,
        upstreamScores: 'diagnostic-only',
        blindSurfaceOutcome: 'CLEAN',
      },
    }, SECURE_HARNESS_CONFIG)).toThrow(/diagnostic gates/);
  });

  it('selects one normalized manifest-bound acceptance task', () => {
    const parsed = parseHarnessManifest(manifest, SECURE_HARNESS_CONFIG);
    const taskPath = 'coding-harness/config/issue-8-acceptance.json';
    const programmeV5TaskPath = 'coding-harness/config/programme-v5-acceptance.json';

    expect(normalizeAcceptanceTaskPath(taskPath)).toBe(taskPath);
    expect(selectAcceptanceTaskPath(parsed, taskPath)).toBe(taskPath);
    expect(normalizeAcceptanceTaskPath(programmeV5TaskPath)).toBe(programmeV5TaskPath);
    expect(selectAcceptanceTaskPath(parsed, programmeV5TaskPath)).toBe(programmeV5TaskPath);
    for (const invalid of [
      '',
      '/coding-harness/config/issue-8-acceptance.json',
      './coding-harness/config/issue-8-acceptance.json',
      'coding-harness/config/../config/issue-8-acceptance.json',
      'coding-harness\\config\\issue-8-acceptance.json',
      'coding-harness/config/issue-8-acceptance.json\0',
      'coding-harness/config/Issue-8-acceptance.json',
      'coding-harness/config/issué-8-acceptance.json',
      'coding-harness/config/issue-8-acceptance.json/',
      'coding-harness/config/issue-8.json',
      'docs/issue-8-acceptance.json',
    ]) {
      expect(() => normalizeAcceptanceTaskPath(invalid)).toThrow();
    }
    expect(() => selectAcceptanceTaskPath(
      parsed,
      'coding-harness/config/m0-reproducibility-acceptance.json',
    )).toThrow('HARNESS_MANIFEST_TASK_NOT_LISTED');
  });

  it('requires every acceptance task to be a unique protected controller input', () => {
    const input = structuredClone(manifest as Record<string, unknown>) as Record<string, any>;
    input.acceptanceTasks = [
      ...input.acceptanceTasks,
      'coding-harness/config/m0-reproducibility-acceptance.json',
    ];
    expect(() => parseHarnessManifest(input, SECURE_HARNESS_CONFIG))
      .toThrow('HARNESS_MANIFEST_TASK_NOT_PROTECTED');

    input.acceptanceTasks = [input.acceptanceTasks[0], input.acceptanceTasks[0]];
    expect(() => parseHarnessManifest(input, SECURE_HARNESS_CONFIG)).toThrow(/duplicates/);
  });

  it('fails closed when any trusted programme-v5 execution source is omitted', () => {
    const paths = SECURE_HARNESS_CONFIG.requiredProtectedPaths;
    expect(controllerExecutionPaths(paths, PROGRAMME_V5_ACCEPTANCE_TASK_PATH)).toEqual(
      expect.arrayContaining([...PROGRAMME_V5_CONTROLLER_REQUIRED_PATHS]),
    );
    for (const required of PROGRAMME_V5_CONTROLLER_REQUIRED_PATHS) {
      expect(() => controllerExecutionPaths(
        paths.filter((path) => path !== required),
        PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
      )).toThrow('HARNESS_CONTROLLER_EXECUTION_MANIFEST_INCOMPLETE');
    }
  });

  it('protects every tracked ADR, Cargo manifest, CI workflow, and publication control', () => {
    const repository = resolve(root, '..');
    const listed = spawnSync('git', ['-C', repository, 'ls-files', '-z'], {
      encoding: 'utf8',
    });
    expect(listed.status, listed.stderr).toBe(0);
    const paths = listed.stdout.split('\0').filter(Boolean);
    const governed = paths.filter((path) =>
      path === 'Cargo.toml'
      || path.endsWith('/Cargo.toml')
      || (path.startsWith('docs/adr/') && path.endsWith('.md'))
      || path.startsWith('.github/workflows/'));
    governed.push(
      '.gitignore', 'AGENTS.md', 'CLAUDE.md', 'LICENSE-APACHE', 'LICENSE-MIT',
      'README.md', 'rust-toolchain.toml',
    );
    expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths).toEqual(
      expect.arrayContaining(governed),
    );
  });
});

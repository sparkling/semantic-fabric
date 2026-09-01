// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import {
  allGates,
  formatOutputs,
  parseDiffOutput,
  readChangedPaths,
  selectForPaths,
  selectFromGit,
} from '../scripts/select-ci-gates.mjs';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(harnessRoot, '..');
const manifestPath = resolve(harnessRoot, '.harness/manifest.json');
const temporaryDirectories: string[] = [];

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function gates(
  rust: boolean,
  coding_harness: boolean,
  supervisor = false,
  acl_replay = false,
) {
  return { rust, coding_harness, supervisor, acl_replay };
}

describe('fail-closed CI impact selector', () => {
  it.each([
    [['crates/sf-core/src/lib.rs'], gates(true, false)],
    [['docs/adr/ADR-0049-exact-recursive-property-path-fixed-points.md'], gates(false, true)],
    [['coding-harness/src/model-controller.ts'], gates(false, true)],
    [['coding-harness/supervisor-service/src/index.ts'], gates(false, true, true, true)],
    [['tests/capabilities/catalog-v1.json'], gates(true, true)],
    [['docs/capability-matrix.md'], gates(true, true)],
    [['Cargo.lock'], gates(true, true)],
    [['crates/new-provider/Cargo.toml'], gates(true, true)],
    [['README.md'], gates(true, true)],
    [['.github/workflows/ci.yml'], allGates()],
    [['coding-harness/scripts/select-ci-gates.mjs'], allGates()],
  ] as const)('classifies %j conservatively', (paths, expected) => {
    expect(selectForPaths([...paths])).toEqual(expected);
  });

  it('uses monotonic OR and preserves gate implications', () => {
    expect(selectForPaths([
      'crates/sf-core/src/lib.rs',
      'coding-harness/src/model-controller.ts',
    ])).toEqual(gates(true, true));
    expect(selectForPaths([
      'docs/adr/ADR-0048-rust-production-and-node-evidence-runtime-boundary.md',
      'coding-harness/supervisor-service/README.md',
    ])).toEqual(gates(false, true, true, true));
    expect(selectForPaths([
      'crates/sf-core/src/lib.rs', 'unclassified/new-surface.txt',
    ])).toEqual(allGates());
  });

  it.each([
    [],
    ['../Cargo.toml'],
    ['/Cargo.toml'],
    ['coding-harness-evil/src/index.ts'],
    ['crates//sf-core/lib.rs'],
    ['crates/./sf-core/lib.rs'],
    ['crates/sf-core/../sf-sql/lib.rs'],
    ['crates\\sf-core\\lib.rs'],
    ['docs/adr/bad\npath.md'],
    ['docs/cafe\u0301.md'],
    ['Cargo.toml', 'Cargo.toml'],
    ['x'.repeat(4_097)],
  ])('selects every gate for empty, malformed, or unknown paths: %j', (paths) => {
    expect(selectForPaths(paths)).toEqual(allGates());
  });

  it('strictly decodes bounded NUL-delimited Git output', () => {
    expect(parseDiffOutput(Buffer.from('old.rs\0new.rs\0'))).toEqual(['old.rs', 'new.rs']);
    for (const bytes of [
      Buffer.alloc(0),
      Buffer.from('unterminated'),
      Buffer.from([0xff, 0x00]),
      Buffer.alloc(1_048_577, 0x61),
    ]) {
      expect(() => parseDiffOutput(bytes)).toThrow(/CI_SELECTOR_DIFF_INVALID|encoded data/);
    }
  });

  it('emits only fixed boolean outputs', () => {
    expect(formatOutputs(gates(true, false, true, true))).toBe(
      'rust=true\ncoding_harness=false\nsupervisor=true\nacl_replay=true\n',
    );
    expect(() => formatOutputs({ rust: 'true' })).toThrow(/OUTPUT_INVALID/);
  });

  it('selects the harness for every protected manifest path', () => {
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as {
      protectedPaths: string[];
    };
    expect(manifest.protectedPaths.length).toBeGreaterThan(100);
    for (const path of manifest.protectedPaths) {
      expect(selectForPaths([path], manifest.protectedPaths).coding_harness, path).toBe(true);
    }
  });

  it('selects supervisor and ACL replay for every tracked supervisor path', () => {
    const tracked = git(repository, ['ls-files', 'coding-harness/supervisor-service/'])
      .stdout.trim().split('\n').filter(Boolean);
    expect(tracked.length).toBeGreaterThan(100);
    for (const path of tracked) {
      expect(selectForPaths([path])).toMatchObject({
        coding_harness: true, supervisor: true, acl_replay: true,
      });
    }
  });

  it('uses exact commits, exposes rename endpoints, and fails closed on Git errors', () => {
    const fixture = gitFixture();
    const base = commit(fixture, 'base');
    mkdirSync(join(fixture, 'crates/sf-core/src'), { recursive: true });
    writeFileSync(join(fixture, 'crates/sf-core/src/old.rs'), 'old\n');
    git(fixture, ['add', '.']);
    const withOld = commit(fixture, 'old path');
    git(fixture, ['mv', 'crates/sf-core/src/old.rs', 'crates/sf-core/src/new.rs']);
    const renamed = commit(fixture, 'rename');
    expect(readChangedPaths({ repository: fixture, baseSha: withOld, headSha: renamed }).sort())
      .toEqual(['crates/sf-core/src/new.rs', 'crates/sf-core/src/old.rs']);

    const manifest = join(fixture, 'manifest.json');
    writeFileSync(manifest, '{"protectedPaths":["Cargo.toml"]}\n');
    expect(selectFromGit({
      eventName: 'pull_request', repository: fixture,
      baseSha: withOld, headSha: renamed, manifestPath: manifest,
    })).toEqual(gates(true, false));
    expect(selectFromGit({
      eventName: 'push', repository: fixture,
      baseSha: withOld, headSha: renamed, manifestPath: manifest,
    })).toEqual(allGates());
    expect(selectFromGit({
      eventName: 'pull_request', repository: fixture,
      baseSha: 'f'.repeat(40), headSha: renamed, manifestPath: manifest,
    })).toEqual(allGates());
    expect(selectFromGit({
      eventName: 'pull_request', repository: fixture,
      baseSha: renamed, headSha: withOld, manifestPath: manifest,
    })).toEqual(allGates());
    expect(selectFromGit({
      eventName: 'pull_request', repository: fixture,
      baseSha: base, headSha: base, manifestPath: manifest,
    })).toEqual(allGates());
  });

  it('keeps the workflow base-controlled, fail-open-to-run, and stably aggregated', () => {
    const workflow = readFileSync(resolve(repository, '.github/workflows/ci.yml'), 'utf8');
    const changesJob = workflow.split('  changes:')[1]?.split('\n  coding-harness:')[0] ?? '';
    expect(workflow).toContain('merge_group:');
    expect(workflow).toContain('types: [checks_requested]');
    expect(workflow).toContain('permissions:\n  contents: read');
    expect(workflow).toContain('cancel-in-progress: ${{ github.event_name == \'pull_request\' }}');
    expect(workflow).not.toContain('pull_request_target:');
    expect(workflow).not.toMatch(/^\s+paths(?:-ignore)?:/m);
    expect(changesJob).toContain('fetch-depth: 0');
    expect(changesJob).toContain(
      'git show "${BASE_SHA}:coding-harness/scripts/select-ci-gates.mjs"',
    );
    expect(changesJob).toContain(
      'git show "${BASE_SHA}:coding-harness/.harness/manifest.json"',
    );
    expect(changesJob).toContain('HEAD_SHA: ${{ github.sha }}');
    for (const [job, output] of [
      ['coding-harness', 'coding_harness'],
      ['supervisor-service', 'supervisor'],
      ['postgresql-public-acl-replay', 'acl_replay'],
      ['build', 'rust'],
    ]) {
      expect(workflow.split(`  ${job}:`)).toHaveLength(2);
      expect(workflow).toContain(
        `needs.changes.result != 'success' || needs.changes.outputs.${output} != 'false'`,
      );
      const resultReference = job.includes('-')
        ? `needs['${job}'].result`
        : `needs.${job}.result`;
      expect(workflow.split(resultReference)).toHaveLength(2);
    }
    expect(workflow).toContain('  required:\n    name: required');
    expect(workflow).toContain('    if: ${{ always() }}');
    expect(workflow).toContain(
      'test "$SELECT_ACL_REPLAY" != true || test "$SELECT_SUPERVISOR" = true',
    );
    expect(workflow).toContain(
      'test "$SELECT_SUPERVISOR" != true || test "$SELECT_CODING_HARNESS" = true',
    );
    expect(workflow).not.toContain('needs.readiness.result');
  });
});

function gitFixture(): string {
  const directory = mkdtempSync(join(tmpdir(), 'semantic-fabric-ci-selector-'));
  temporaryDirectories.push(directory);
  git(directory, ['init', '--initial-branch=main']);
  git(directory, ['config', 'user.name', 'Selector Test']);
  git(directory, ['config', 'user.email', 'selector@example.invalid']);
  writeFileSync(join(directory, 'README.md'), 'fixture\n');
  git(directory, ['add', '.']);
  return directory;
}

function commit(directory: string, message: string): string {
  git(directory, ['commit', '--allow-empty', '-m', message]);
  return git(directory, ['rev-parse', 'HEAD']).stdout.trim();
}

function git(directory: string, args: string[]): { stdout: string } {
  const result = spawnSync('git', ['-C', directory, ...args], { encoding: 'utf8' });
  expect(result.status, result.stderr).toBe(0);
  return { stdout: result.stdout };
}

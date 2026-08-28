// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(harnessRoot, '..');
const manifest = JSON.parse(
  readFileSync(resolve(harnessRoot, '.harness/manifest.json'), 'utf8'),
) as { protectedPaths: string[] };

const M0_AUTHORITY_PATHS = [
  'crates/sf-conformance/src/bin/capability-matrix.rs',
  'crates/sf-conformance/src/bin/rdb2rdf-execution-receipt.rs',
  'crates/sf-conformance/src/bin/rdb2rdf-inventory.rs',
  'crates/sf-conformance/src/capability_catalog.rs',
  'crates/sf-conformance/src/capability_model.rs',
  'crates/sf-conformance/src/capability_render.rs',
  'crates/sf-conformance/src/execution_receipt.rs',
  'crates/sf-conformance/src/execution_receipt/format.rs',
  'crates/sf-conformance/src/execution_receipt/tests.rs',
  'crates/sf-conformance/src/graph.rs',
  'crates/sf-conformance/src/inventory.rs',
  'crates/sf-conformance/src/inventory/format.rs',
  'crates/sf-conformance/src/inventory/policy.rs',
  'crates/sf-conformance/src/lib.rs',
  'crates/sf-conformance/src/manifest.rs',
  'crates/sf-conformance/src/oracle.rs',
  'crates/sf-conformance/src/pg.rs',
  'crates/sf-conformance/src/runner.rs',
  'crates/sf-conformance/src/sealed_suite.rs',
  'crates/sf-conformance/src/sqlite.rs',
  'crates/sf-conformance/tests/capability_matrix.rs',
  'crates/sf-conformance/tests/rdb2rdf_execution_receipt.rs',
  'crates/sf-conformance/tests/rdb2rdf_inventory.rs',
  'crates/sf-conformance/tests/rdb2rdf_runner_seal.rs',
  'crates/sf-conformance/tests/w3c_pg_suite.rs',
  'docs/capability-matrix.json',
  'docs/capability-matrix.md',
  'tests/capabilities/catalog-v1.json',
  'tests/capabilities/schema-v1.json',
  'tests/w3c/rdb2rdf/inventory.tsv',
  'tests/w3c/rdb2rdf/sqlite-execution-receipt.tsv',
] as const;

const REQUIRED_CI_COMMANDS = [
  'cargo run --locked -p sf-conformance --bin rdb2rdf-inventory -- --check',
  'cargo run --locked -p sf-conformance --bin rdb2rdf-execution-receipt -- --check',
  'cargo run --locked -p sf-conformance --bin capability-matrix -- --check',
] as const;

describe('M0 protected authority and CI contract', () => {
  it('protects every tracked authority in both config and manifest', () => {
    expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths)
      .toEqual(expect.arrayContaining([...M0_AUTHORITY_PATHS]));
    expect(manifest.protectedPaths).toEqual(expect.arrayContaining([...M0_AUTHORITY_PATHS]));

    const tracked = spawnSync(
      'git', ['-C', repository, 'ls-files', '--error-unmatch', '--', ...M0_AUTHORITY_PATHS],
      { encoding: 'utf8' },
    );
    expect(tracked.status, tracked.stderr).toBe(0);
  });

  it('automatically protects every tracked capability and receipt authority', () => {
    for (const directory of [
      'crates/sf-conformance/src/execution_receipt/',
      'tests/capabilities/',
    ]) {
      const listed = spawnSync(
        'git', ['-C', repository, 'ls-files', directory], { encoding: 'utf8' },
      );
      expect(listed.status, listed.stderr).toBe(0);
      const paths = listed.stdout.split(/\r?\n/).filter(Boolean);
      expect(paths.length).toBeGreaterThan(0);
      expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths)
        .toEqual(expect.arrayContaining(paths));
      expect(manifest.protectedPaths).toEqual(expect.arrayContaining(paths));
    }
  });

  it('runs each read-only authority check exactly once and in dependency order', () => {
    const workflow = readFileSync(resolve(repository, '.github/workflows/ci.yml'), 'utf8');
    const positions = REQUIRED_CI_COMMANDS.map((command) => {
      expect(workflow.split(command)).toHaveLength(2);
      return workflow.indexOf(command);
    });
    expect(positions).toEqual([...positions].sort((left, right) => left - right));
    for (const binary of ['rdb2rdf-inventory', 'rdb2rdf-execution-receipt', 'capability-matrix']) {
      expect(workflow).not.toMatch(new RegExp(`--bin ${binary} -- --generate`));
    }
  });
});

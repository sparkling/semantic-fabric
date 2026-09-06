// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { parseHarnessConfig } from '../src/contracts.js';
import {
  POSTGRESQL_PRODUCT_SLICE_PROTECTED_PATHS_V1,
  POSTGRESQL_QUALIFICATION_EVIDENCE_PROTECTED_PATHS_V1,
  POSTGRESQL_QUALIFICATION_INPUT_PROTECTED_PATHS_V1,
  POSTGRESQL_QUALIFICATION_OUTPUT_PROTECTED_PATHS_V1,
}
  from '../src/programme-capture-protected-paths-v1.js';
import { parsePostgresQualificationInventory } from
  '../src/postgres-observation-qualification-provenance.js';
import { parseJsonWithoutDuplicateKeys } from '../src/strict-json.js';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(harnessRoot, '..');
const PRODUCT_SLICE_PATHSPECS = [
  'crates/sf-serve/src/activation.rs',
  'crates/sf-serve/src/activation',
  'crates/sf-serve/src/backend.rs',
  'crates/sf-serve/src/binding_identity.rs',
  'crates/sf-serve/src/budget.rs',
  'crates/sf-serve/src/config.rs',
  'crates/sf-serve/src/config',
  'crates/sf-serve/src/deadline.rs',
  'crates/sf-serve/src/deadline_tests.rs',
  'crates/sf-serve/src/observed_source.rs',
  'crates/sf-serve/src/pg_direct_lifecycle.rs',
  'crates/sf-serve/src/pg_direct_lifecycle',
  'crates/sf-serve/src/pg_generation.rs',
  'crates/sf-serve/src/pg_generation',
  'crates/sf-serve/src/pg_pool.rs',
  'crates/sf-serve/src/pg_response.rs',
  'crates/sf-serve/src/request_generation.rs',
  'crates/sf-serve/src/run.rs',
  'crates/sf-serve/src/runtime_snapshot_tests.rs',
  'crates/sf-serve/src/schema_observation.rs',
  'crates/sf-serve/src/semantic_admission.rs',
  'crates/sf-serve/src/semantic_admission_tests.rs',
  'crates/sf-serve/src/snapshot.rs',
  'crates/sf-serve/src/source.rs',
  'crates/sf-sparql/src/runtime_identity.rs',
  'crates/sf-sql/src/backend/pg.rs',
  'crates/sf-sql/src/backend/pg',
  'crates/sf-sql/src/introspect.rs',
  'crates/sf-sql/src/introspect/postgres.rs',
  'crates/sf-sql/src/introspect/postgres',
] as const;
const QUALIFICATION_EVIDENCE_PATHSPECS = [
  ':(glob)coding-harness/__tests__/postgres-observation-qualification*.test.ts',
  ':(glob)coding-harness/src/postgres-observation-qualification*.ts',
  'crates/sf-conformance/src/bin/postgres-observation-qualification.rs',
  'crates/sf-conformance/src/bin/postgres-observation-qualification',
  'tests/postgresql',
  ':(exclude)tests/postgresql/postgresql-16-observation-qualification-receipt-pair-v1.json',
] as const;

function discoverTrackedPaths(pathspecs: readonly string[]): string[] {
  const result = spawnSync(
    'git', [
      '-C', repository, 'ls-files', '-z', '--cached', '--others', '--exclude-standard',
      '--', ...pathspecs,
    ],
    { encoding: 'utf8' },
  );
  expect(result.status, result.stderr).toBe(0);
  return result.stdout.split('\0').filter(Boolean).sort((left, right) => left.localeCompare(right));
}

function discoverTrackedProductSlice(): string[] {
  return discoverTrackedPaths(PRODUCT_SLICE_PATHSPECS);
}

function discoverQualificationEvidence(): string[] {
  return discoverTrackedPaths(QUALIFICATION_EVIDENCE_PATHSPECS);
}

function qualificationInventoryPaths(): string[] {
  const source = readFileSync(resolve(
    repository, 'tests/postgresql/observation-qualification-inputs-v1.tsv',
  ));
  return parsePostgresQualificationInventory(source).map(({ path }) => path);
}

describe('PostgreSQL verified-generation product-slice closure', () => {
  it('derives every current tracked generation and introspection file independently', () => {
    const discovered = discoverTrackedProductSlice();
    expect([...POSTGRESQL_PRODUCT_SLICE_PROTECTED_PATHS_V1]).toEqual(discovered);
    expect(new Set(discovered).size).toBe(discovered.length);
  });

  it('derives the complete qualification evidence closure independently', () => {
    const discovered = discoverQualificationEvidence();
    expect([...POSTGRESQL_QUALIFICATION_EVIDENCE_PROTECTED_PATHS_V1]).toEqual(discovered);
    expect(discovered).not.toEqual(expect.arrayContaining(
      POSTGRESQL_QUALIFICATION_OUTPUT_PROTECTED_PATHS_V1,
    ));
    expect(new Set(discovered).size).toBe(discovered.length);
  });

  it('mirrors every strictly parsed qualification inventory path exactly', () => {
    const parsed = qualificationInventoryPaths();
    expect([...POSTGRESQL_QUALIFICATION_INPUT_PROTECTED_PATHS_V1]).toEqual(parsed);
    expect(parsed).toHaveLength(101);
    expect(new Set(parsed).size).toBe(parsed.length);
  });

  it('binds the discovered slice and every Cargo manifest into both authorities', () => {
    const manifest = parseJsonWithoutDuplicateKeys(readFileSync(
      resolve(harnessRoot, '.harness/manifest.json'), 'utf8',
    ), 'canonical harness manifest') as { protectedPaths: string[] };
    const cargoManifests = spawnSync(
      'git', ['-C', repository, 'ls-files', '-z', '--', 'Cargo.toml', ':(glob)**/Cargo.toml'],
      { encoding: 'utf8' },
    );
    expect(cargoManifests.status, cargoManifests.stderr).toBe(0);
    const required = [
      ...discoverTrackedProductSlice(),
      ...discoverQualificationEvidence(),
      ...qualificationInventoryPaths(),
      ...POSTGRESQL_QUALIFICATION_OUTPUT_PROTECTED_PATHS_V1,
      ...cargoManifests.stdout.split('\0').filter(Boolean),
    ];
    expect(required).toContain('crates/sf-validation/Cargo.toml');
    expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths).toEqual(expect.arrayContaining(required));
    expect(manifest.protectedPaths).toEqual(expect.arrayContaining(required));
  });

  it('rejects a duplicate introduced by overlapping protected-path registries', () => {
    expect(() => parseHarnessConfig({
      ...SECURE_HARNESS_CONFIG,
      requiredProtectedPaths: [
        ...SECURE_HARNESS_CONFIG.requiredProtectedPaths,
        ...POSTGRESQL_QUALIFICATION_INPUT_PROTECTED_PATHS_V1,
      ],
    })).toThrow('config.requiredProtectedPaths must not contain duplicates');
  });
});

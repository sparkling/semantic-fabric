// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
// @ts-expect-error The test exercises the private test-only JavaScript mutator directly.
import * as finalWhereMutations from '../scripts/postgresql-public-acl-projection-final-where-mutations-v1.mjs';
// @ts-expect-error The test exercises the private test-only oracle directly.
import * as finalWhereOracle from '../scripts/postgresql-public-acl-final-where-mutation-oracle-v1.mjs';

const {
  FINAL_WHERE_MUTATION_SOURCE_PIN_V1, FINAL_WHERE_MUTATION_SPECS_V1,
  buildPublicAclProjectionFinalWhereMutantsV1,
} = finalWhereMutations;
const {
  FINAL_WHERE_MUTATION_BATCHES_V1, classifyFinalWhereMutationV1,
  parseFinalWhereLooseRecordsV1, splitFinalWhereMutationTranscriptV1,
  validateFinalWhereMutationReceiptV1,
} = finalWhereOracle;

const ROOT = resolve(import.meta.dirname, '..');
const PROJECTION_PATH = resolve(import.meta.dirname,
  'fixtures/postgresql-16.15-public-acl-projection-v1.sql');
const MUTATOR_PATH = 'scripts/postgresql-public-acl-projection-final-where-mutations-v1.mjs';
const ORACLE_PATH = 'scripts/postgresql-public-acl-final-where-mutation-oracle-v1.mjs';
const RUNNER_PATH = 'scripts/verify-postgresql-public-acl-projection-final-where-mutations-v1.mjs';
const TEST_PATH = '__tests__/registration-postgresql-public-acl-projection-final-where-mutations-v1.test.ts';
const SOURCE_SHA = '0e3ad724f4ce85191564c245c51dd7665b6d9aa704c355067a0056cdbfe95232';

const EXPECTED = Object.freeze([
  ['delete-schema-where-public-grantee', 'kill', 829, 847,
    'edc3367c0e8a3ac92c2031b3a5745ab987441ccb1e8ed4aa60917da93b7b5509'],
  ['delete-schema-where-excluded-namespace', 'kill', 842, 894,
    'a1699a676732fbacd863c0f4c65e6326e06af4e0e616fb82a61a3ec39b417818'],
  ['delete-relation-where-supported-relkind', 'guard-equivalent', 1624, 1672,
    '867c92776cafd21ef213acfa80c4b994ce28a6b4dd822eca0d72ca0b55753f35'],
  ['delete-relation-where-public-grantee', 'kill', 1667, 1685,
    '1c13c5791460fdbf7c53c16887cc9cc2661a625e5f254d204d0f851ee5aa39e2'],
  ['delete-relation-where-excluded-namespace', 'kill', 1686, 1742,
    'e379e71814be89433b4594243f1dddb2fdc7267e4f76c51c2c9c970702cb7841'],
  ['delete-column-where-positive-attnum', 'kill', 2456, 2475,
    '3333b8672b3d9124aa6ca6558c5c1b89ab5b821332af5c6c8e2ba04d5298540c'],
  ['delete-column-where-not-dropped', 'kill', 2470, 2495,
    'a767b46920bdb68ef503f9b262700ad0ba63f805960a381ed8c6e35b1460d273'],
  ['delete-column-where-supported-parent-relkind', 'guard-equivalent', 2503, 2546,
    '94b0d8d2c91cc6a63dfc47a401e6a1c4e0a421024091ad906548a8b138580aab'],
  ['delete-column-where-public-grantee', 'kill', 2542, 2560,
    '479a0248e4428380d4d7a70d56f2b29c3e52150e526cc24bff4f128077f9cb48'],
  ['delete-column-where-excluded-namespace', 'kill', 2561, 2617,
    '8e6e6a25c5f4ad3511e3c8050367a659b53d226de765ec8ea9f4797281ad1f2d'],
  ['delete-routine-where-supported-prokind', 'guard-equivalent', 3270, 3308,
    '91291cdc20788acf6cc3ce68a79c33efb7bb8073421d4ccd3f5d4cce4aeced58'],
  ['delete-routine-where-public-grantee', 'kill', 3303, 3321,
    '3fe642de2c741ceb0fc388034fa4d7b1b6c91c0f2a1f3993aa3aa4ad2ab2ae52'],
  ['delete-routine-where-excluded-namespace', 'kill', 3322, 3378,
    '4c4e72fa3d00c0a6dc92cc0f8c8becb21985cd54490e58b3e1aecae26fef55b1'],
  ['delete-type-where-supported-typtype', 'guard-equivalent', 4739, 4792,
    'c01051e8b0624536acbd9a0eec675757497ed74a253561a298609c1a33420e3a'],
  ['delete-type-where-public-grantee', 'kill', 4787, 4805,
    '6d7fdde6c69b3651e38a54ddae9bbd2f6f63d1be0d310674e4d89ccbad89620f'],
  ['delete-type-where-excluded-namespace', 'kill', 4806, 4862,
    '1b53568593fac521266a748a0740fc7812ba16c73871bbd5661ae87fb3c01c5c'],
  ['delete-language-where-public-grantee', 'kill', 5259, 5279,
    '834039dcac2fb85b5027f34f324e4dcc8cc916fa17c1073b1b6a11bece20261b'],
  ['delete-foreign-data-wrapper-where-public-grantee', 'kill', 5713, 5733,
    'c0d1dcb6fc033d2663e0b39abf62f45d07dfe1c00985030d298f4e564960fdbc'],
  ['delete-foreign-server-where-public-grantee', 'kill', 6149, 6169,
    '77e8b71d2cab9beeeae464ac6e6df5379a16402d9b81887cde5caf44fb222e6d'],
] as const);

const RECORD = Object.freeze({
  objectClass: 'schema', schemaName: null, objectName: 'x', subobjectName: null,
  objectKind: 'schema', routineIdentityArguments: null, privilege: 'USAGE', grantable: false,
});

describe('PostgreSQL PUBLIC ACL final-WHERE mutation catalogue V1', () => {
  const source = readFileSync(PROJECTION_PATH, 'utf8');

  it('pins the exact source and fixed 19-member classification', () => {
    expect(FINAL_WHERE_MUTATION_SOURCE_PIN_V1).toEqual({ bytes: 6_859, sha256: SOURCE_SHA });
    expect(Object.isFrozen(FINAL_WHERE_MUTATION_SOURCE_PIN_V1)).toBe(true);
    expect(FINAL_WHERE_MUTATION_SPECS_V1.map((value: any) => [
      value.id, value.expectedOutcome, value.start, value.end, value.sourceSha256,
    ])).toEqual(EXPECTED);
    expect(FINAL_WHERE_MUTATION_SPECS_V1).toHaveLength(19);
    expect(FINAL_WHERE_MUTATION_SPECS_V1.filter(
      (value: any) => value.expectedOutcome === 'kill',
    )).toHaveLength(15);
    expect(FINAL_WHERE_MUTATION_SPECS_V1.filter(
      (value: any) => value.expectedOutcome === 'guard-equivalent',
    )).toHaveLength(4);
    expect(new Set(FINAL_WHERE_MUTATION_SPECS_V1.map((value: any) => value.id)).size).toBe(19);
  });

  it('makes exactly one contiguous source edit per mutant', () => {
    const before = `${source}`;
    const catalogue = buildPublicAclProjectionFinalWhereMutantsV1(source);
    expect(source).toBe(before);
    expect(Object.isFrozen(catalogue)).toBe(true);
    expect(Object.isFrozen(catalogue.mutants)).toBe(true);
    expect(catalogue.authority).toBe('test-only-non-runtime');
    expect(catalogue.mutants).toHaveLength(19);
    for (const [index, mutant] of catalogue.mutants.entries()) {
      const spec = FINAL_WHERE_MUTATION_SPECS_V1[index];
      expect(Object.isFrozen(spec)).toBe(true);
      expect(Object.isFrozen(mutant)).toBe(true);
      expect(mutant.sequence).toBe(index + 1);
      expect(source.slice(spec.start, spec.end)).toBe(spec.removed);
      expect(mutant.source).toBe(source.slice(0, spec.start) + source.slice(spec.end));
      expect(mutant.source).not.toMatch(/WHERE\s+(?:AND|OR)\b|\bAND\s*(?:\)|UNION ALL)/u);
      expect(mutant.source.endsWith(';\n')).toBe(true);
      expect(Buffer.byteLength(mutant.source)).toBe(6_859 - Buffer.byteLength(spec.removed));
      expect(createHash('sha256').update(mutant.source).digest('hex')).toBe(spec.sourceSha256);
    }
  });

  it('fails closed before anchor use when source bytes move, duplicate, disappear, or drift', () => {
    const candidates = [
      source.slice(1), `${source}\n`, source.replace('a.grantee = 0', 'a.grantee=0'),
      source.replace('a.grantee = 0', 'a.grantee = 0 AND a.grantee = 0'),
      source.replace("n.nspname NOT IN ('public', 'sf_supervisor_v1')", 'TRUE'),
    ];
    candidates.forEach((candidate) => expect(
      () => buildPublicAclProjectionFinalWhereMutantsV1(candidate),
    ).toThrow('ACL_FINAL_WHERE_SOURCE_PIN_INVALID'));
  });

  it('freezes two complete non-overlapping execution batches of ten and nine', () => {
    expect(FINAL_WHERE_MUTATION_BATCHES_V1.map((batch: any) => batch.mutantIds.length))
      .toEqual([10, 9]);
    expect(FINAL_WHERE_MUTATION_BATCHES_V1.flatMap((batch: any) => batch.mutantIds))
      .toEqual(EXPECTED.map(([id]) => id));
    expect(Object.isFrozen(FINAL_WHERE_MUTATION_BATCHES_V1)).toBe(true);
    FINAL_WHERE_MUTATION_BATCHES_V1.forEach((batch: any) => {
      expect(Object.isFrozen(batch)).toBe(true);
      expect(Object.isFrozen(batch.mutantIds)).toBe(true);
    });
  });

  it('uses a loose closed eight-field bag parser for executable mutants', () => {
    const hostile = { ...RECORD, grantable: true };
    expect(parseFinalWhereLooseRecordsV1(`${JSON.stringify(hostile)}\n`)).toEqual([hostile]);
    const invalid = [
      `${JSON.stringify({ ...RECORD, extra: true })}\n`, `${JSON.stringify(RECORD)}\r\n`,
      `${JSON.stringify(RECORD)}\ntrailing`, '{bad}\n', `${JSON.stringify([RECORD])}\n`,
    ];
    invalid.forEach((value) => expect(() => parseFinalWhereLooseRecordsV1(value))
      .toThrow(/ACL_FINAL_WHERE_/u));
  });

  it('kills only a changed non-equivalent bag and proves every equivalent guard', () => {
    const kill = FINAL_WHERE_MUTATION_SPECS_V1[0];
    const equivalent = FINAL_WHERE_MUTATION_SPECS_V1[2];
    const guards = Object.freeze({ relationUnsupported: 0, columnUnsupportedParent: 0,
      routineUnsupported: 0, typeUnsupported: 0 });
    expect(classifyFinalWhereMutationV1(kill, [RECORD], [RECORD, { ...RECORD, objectName: 'y' }],
      guards).outcome).toBe('killed');
    expect(classifyFinalWhereMutationV1(equivalent, [RECORD], [RECORD], guards)).toMatchObject({
      outcome: 'guard-equivalent', guardWitness: 'relationUnsupported', guardCount: 0,
    });
    expect(() => classifyFinalWhereMutationV1(kill, [RECORD], [RECORD], guards))
      .toThrow('ACL_FINAL_WHERE_KILLABLE_SURVIVED');
    expect(() => classifyFinalWhereMutationV1(equivalent, [RECORD], [], guards))
      .toThrow('ACL_FINAL_WHERE_EQUIVALENT_CHANGED');
    expect(() => classifyFinalWhereMutationV1(equivalent, [RECORD], [RECORD],
      { ...guards, relationUnsupported: 1 })).toThrow('ACL_FINAL_WHERE_GUARD_PROOF_INVALID');
  });

  it('rejects malformed, duplicated, oversized, and trailing transcript sections', () => {
    const ids = ['one', 'two'];
    const marker = (id: string, edge: string) => `@@ADR0047-FINAL-WHERE-V1/${id}/${edge}@@\n`;
    const valid = `raw\n@@ADR0047-RAW-V1/CONTROL/END@@\n${marker('seed-witness', 'BEGIN')}`
      + `{}\n${marker('seed-witness', 'END')}${marker('guard-witness', 'BEGIN')}{}\n`
      + `${marker('guard-witness', 'END')}${marker('original', 'BEGIN')}${JSON.stringify(RECORD)}\n`
      + `${marker('original', 'END')}${ids.map((id) => marker(id, 'BEGIN')
        + `${JSON.stringify(RECORD)}\n${marker(id, 'END')}`).join('')}`;
    expect(splitFinalWhereMutationTranscriptV1(Buffer.from(valid), ids).mutations)
      .toEqual(expect.objectContaining({ one: expect.any(String), two: expect.any(String) }));
    const invalid = [valid + 'x', valid.replace(marker('one', 'END'), ''),
      valid.replace(marker('two', 'BEGIN'), marker('one', 'BEGIN')),
      valid.replace(marker('seed-witness', 'BEGIN'), marker('seed-witness', 'BEGIN')
        + marker('seed-witness', 'BEGIN'))];
    invalid.forEach((value) => expect(
      () => splitFinalWhereMutationTranscriptV1(Buffer.from(value), ids),
    ).toThrow(/ACL_FINAL_WHERE_/u));
    expect(() => splitFinalWhereMutationTranscriptV1(Buffer.alloc(16 * 1024 * 1024 + 1), ids))
      .toThrow('ACL_FINAL_WHERE_TRANSCRIPT_FRAMING_INVALID');
  });

  it('accepts only the exact 19/15/4/0 replay receipt', () => {
    const value = receipt();
    expect(validateFinalWhereMutationReceiptV1(value)).toBe(value);
    const mutations = value.batches.flatMap((batch: any) => batch.mutations);
    for (const replacement of ['survived', 'changed-equivalent', 'unresolved']) {
      const changed = structuredClone(value); changed.batches[0].mutations[0].outcome = replacement;
      expect(() => validateFinalWhereMutationReceiptV1(changed))
        .toThrow('ACL_FINAL_WHERE_RECEIPT_MUTATION_INVALID');
    }
    const changedEquivalent = structuredClone(value);
    changedEquivalent.batches.flatMap((batch: any) => batch.mutations)
      .find((entry: any) => entry.expectedOutcome === 'guard-equivalent').guardCount = 1;
    expect(() => validateFinalWhereMutationReceiptV1(changedEquivalent))
      .toThrow('ACL_FINAL_WHERE_RECEIPT_MUTATION_INVALID');
    expect(mutations).toHaveLength(19);
  });

  it('keeps the quartet test-only, protected, source-pinned, and below 500 lines', () => {
    const paths = [MUTATOR_PATH, ORACLE_PATH, RUNNER_PATH, TEST_PATH];
    const runner = readFileSync(resolve(ROOT, RUNNER_PATH), 'utf8');
    expect(runner).toContain('runOwnedReplayPair');
    expect(runner).toContain('BEGIN ISOLATION LEVEL SERIALIZABLE;');
    expect(runner).toContain('ROLLBACK;');
    expect(runner).toContain('MAX_TRANSCRIPT_BYTES');
    expect(runner).not.toMatch(/a108e05f|4_059|860_988/iu);
    const artifact = JSON.parse(readFileSync(resolve(ROOT, '.service/artifact.json'), 'utf8')) as {
      buildInputs: Record<string, string>; sourceInputs: Record<string, string>;
    };
    const inputs = [...Object.keys(artifact.buildInputs), ...Object.keys(artifact.sourceInputs)];
    const manifest = JSON.parse(readFileSync(resolve(ROOT, '../.harness/manifest.json'), 'utf8')) as {
      protectedPaths: string[];
    };
    const registry = readFileSync(resolve(ROOT,
      '../src/programme-capture-protected-paths-v1.ts'), 'utf8');
    paths.forEach((path) => {
      expect(inputs).not.toContain(path);
      const repositoryPath = `coding-harness/supervisor-service/${path}`;
      expect(manifest.protectedPaths.filter((value) => value === repositoryPath)).toHaveLength(1);
      expect(registry.split(`'${repositoryPath}'`)).toHaveLength(2);
      const file = readFileSync(resolve(ROOT, path), 'utf8');
      expect(file.endsWith('\n')).toBe(true);
      expect(file.split('\n').length - 1, path).toBeLessThan(500);
    });
  });
});

function receipt(): any {
  const batches = FINAL_WHERE_MUTATION_BATCHES_V1.map((batch: any) => ({
    sequence: batch.sequence, transcriptBytes: 1, transcriptSha256: 'a'.repeat(64),
    originalEquivalent: true, seedWitnessComplete: true,
    mutations: batch.mutantIds.map((id: string) => {
      const spec = FINAL_WHERE_MUTATION_SPECS_V1.find((entry: any) => entry.id === id);
      const equivalent = spec.expectedOutcome === 'guard-equivalent';
      return { sequence: spec.sequence, id, expectedOutcome: spec.expectedOutcome,
        outcome: equivalent ? 'guard-equivalent' : 'killed', executed: true, parsed: true,
        guardWitness: spec.guardWitness, guardCount: equivalent ? 0 : null,
        recordCount: 1, recordsSha256: 'b'.repeat(64), sourceSha256: spec.sourceSha256 };
    }),
  }));
  return { schemaVersion: 1, authority: 'test-only-non-runtime', executed: 19,
    killed: 15, guardEquivalent: 4, unresolved: 0, batches };
}

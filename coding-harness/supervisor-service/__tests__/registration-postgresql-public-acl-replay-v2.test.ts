// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
// @ts-expect-error The test exercises the private JavaScript receipt validator directly.
import { validateReceipt as validateV2Receipt } from '../scripts/replay-postgresql-public-acl-baseline-v2.mjs';

const ROOT = resolve(import.meta.dirname, '..');
const V1_PATH = resolve(
  import.meta.dirname, 'fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json',
);
const V2_PATH = resolve(
  import.meta.dirname, 'fixtures/postgresql-16.15-public-acl-capture-receipt-v2.json',
);
const V1_PIN = Object.freeze({
  bytes: 4_835,
  sha256: '14fbd3ff2d2b50d3a8adbe0b51dc921eb926cd644a4a765183723518ec4fd08b',
});
const V2_PIN = Object.freeze({
  bytes: 8_816,
  sha256: '48d54b635ff6bafc6bdb4ffcb1bb9d74c8357e932e22f7b6453bb54cb0d698e8',
});
const SOURCE_KEYS = Object.freeze([
  'projection', 'rawOracle', 'captureRunner', 'oracleWire', 'oracleDeriver',
  'oracleRunner', 'v1ReplayRunner', 'witnessInventorySql', 'witnessSql',
  'witnessInventoryParser', 'witnessVerifier', 'witnessRunner', 'v2ReplaySupport',
  'v2ReplayRunner',
]);
const LEGACY_SOURCE_KEYS = Object.freeze({
  projection: 'projection',
  rawOracle: 'rawOracle',
  captureRunner: 'captureRunner',
  oracleWire: 'oracleWire',
  oracleDeriver: 'oracleDeriver',
  oracleRunner: 'oracleRunner',
  v1ReplayRunner: 'replayRunner',
});
const EVIDENCE_PATHS = Object.freeze([
  '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v2.json',
  '__tests__/fixtures/postgresql-16.15-public-acl-has-privilege-inventory-v1.sql',
  '__tests__/fixtures/postgresql-16.15-public-acl-has-privilege-witness-v1.sql',
  '__tests__/registration-postgresql-public-acl-has-privilege-v1.test.ts',
  '__tests__/registration-postgresql-public-acl-replay-v2.test.ts',
  'scripts/postgresql-public-acl-has-privilege-inventory-v1.mjs',
  'scripts/postgresql-public-acl-has-privilege-v1.mjs',
  'scripts/postgresql-public-acl-replay-support-v2.mjs',
  'scripts/replay-postgresql-public-acl-baseline-v2.mjs',
  'scripts/verify-postgresql-public-acl-has-privilege-v1.mjs',
]);
const EXPECTED_WITNESS = Object.freeze({
  name: 'postgresql-16.15-public-acl-has-privilege-witness-v1',
  roleName: 'sf_public_acl_no_membership_witness_v1',
  inventoryEntries: 13_603,
  checkCount: 13_603,
  plainTrueCount: 5_958,
  plainFalseCount: 7_645,
  grantOptionTrueCount: 0,
  corroboratedAtoms: 4_059,
  columnLocalAtoms: 16,
  trueArrayAtoms: 294,
  inventoryBytes: 4_362_966,
  inventorySha256: '3630c07af1bec0d18a177ddcb1139c2de3314da9651cfc328d750e2f1f085ac6',
  observationsBytes: 5_432_583,
  observationsSha256: '740f3b3d26d3aea8f90feb30d454eb7b9d4116959097e24a987a7b73219785e2',
  classCounts: {
    schema: { checks: 6, plainTrue: 2, plainFalse: 4 },
    relation: { checks: 1_463, plainTrue: 189, plainFalse: 1_274 },
    column: { checks: 8_220, plainTrue: 1_915, plainFalse: 6_305 },
    routine: { checks: 3_297, plainTrue: 3_235, plainFalse: 62 },
    type: { checks: 613, plainTrue: 613, plainFalse: 0 },
    language: { checks: 4, plainTrue: 4, plainFalse: 0 },
    'foreign-data-wrapper': { checks: 0, plainTrue: 0, plainFalse: 0 },
    'foreign-server': { checks: 0, plainTrue: 0, plainFalse: 0 },
  },
});

interface SourcePin {
  readonly path: string;
  readonly bytes: number;
  readonly sha256: string;
}

interface ReceiptRun {
  readonly sequence: number;
  readonly networkMode: string;
  readonly publishedPorts: boolean;
  readonly dataVolumeNameSha256: string;
  readonly [key: string]: string | number | boolean;
}

interface LegacyReceipt {
  readonly schemaVersion: string;
  readonly image: unknown;
  readonly database: unknown;
  readonly sources: Readonly<Record<string, SourcePin>>;
  readonly profile: unknown;
  readonly result: unknown;
  readonly runs: readonly ReceiptRun[];
}

interface V2Receipt extends LegacyReceipt {
  readonly authority: string;
  readonly captureDate: string;
  readonly predecessor: Readonly<Record<string, unknown>>;
  readonly witness: Readonly<Record<string, unknown>>;
  readonly replay: Readonly<Record<string, unknown>>;
}

describe('PostgreSQL 16.15 PUBLIC ACL additive V2 replay receipt', () => {
  const v1Source = readFileSync(V1_PATH);
  const v2Source = readFileSync(V2_PATH);
  const v1 = parseCanonical(v1Source) as unknown as LegacyReceipt;
  const v2 = parseCanonical(v2Source) as unknown as V2Receipt;

  it('pins canonical V1 and V2 receipt bytes without reinterpreting V1', () => {
    expect({ bytes: v1Source.byteLength, sha256: sha256(v1Source) }).toEqual(V1_PIN);
    expect({ bytes: v2Source.byteLength, sha256: sha256(v2Source) }).toEqual(V2_PIN);
    expect(v1.schemaVersion).toBe('semantic-fabric.postgresql-public-acl-capture-receipt/v1');
    expect(v2.schemaVersion).toBe('semantic-fabric.postgresql-public-acl-capture-receipt/v2');
    expect(Object.keys(v2)).toEqual([
      'schemaVersion', 'authority', 'captureDate', 'predecessor', 'image', 'database',
      'sources', 'profile', 'result', 'witness', 'runs', 'replay',
    ]);
    expect(Object.keys(v1)).not.toEqual(Object.keys(v2));
    expect(v2.authority).toBe('test-only-non-runtime');
    expect(v2.captureDate).toBe('2026-08-31');
  });

  it('hard-binds the frozen V1 predecessor and every inherited field', () => {
    expect(v2.predecessor).toEqual({
      path: '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json',
      schemaVersion: v1.schemaVersion,
      ...V1_PIN,
    });
    expect(v2.image).toEqual(v1.image);
    expect(v2.database).toEqual(v1.database);
    expect(v2.profile).toEqual(v1.profile);
    expect(v2.result).toEqual(v1.result);
    for (const [v2Key, v1Key] of Object.entries(LEGACY_SOURCE_KEYS)) {
      expect(v2.sources[v2Key]).toEqual(v1.sources[v1Key]);
    }
    const runner = readFileSync(
      resolve(ROOT, 'scripts/replay-postgresql-public-acl-baseline-v2.mjs'), 'utf8',
    );
    expect(runner).toContain('const V1_RECEIPT_BYTES = 4_835;');
    expect(runner).toContain(V1_PIN.sha256);
    expect(runner).not.toMatch(/\bwriteFile(?:Sync)?\b/u);
  });

  it('pins every closed source entry against its regular committed byte carrier', () => {
    expect(Object.keys(v2.sources)).toEqual(SOURCE_KEYS);
    for (const pin of Object.values(v2.sources)) {
      expect(Object.keys(pin)).toEqual(['path', 'bytes', 'sha256']);
      const source = readFileSync(resolve(ROOT, pin.path));
      expect(source.byteLength).toBe(pin.bytes);
      expect(sha256(source)).toBe(pin.sha256);
    }
  });

  it('seals the independent witness authority and both deterministic captures', () => {
    expect(v2.witness).toEqual(EXPECTED_WITNESS);
    expect(v2.runs).toHaveLength(2);
    expect(v2.runs.map((run) => run.sequence)).toEqual([1, 2]);
    expect(v2.runs.every((run) => run.networkMode === 'none'
      && run.publishedPorts === false)).toBe(true);
    expect(v2.runs[0]?.dataVolumeNameSha256).not.toBe(v2.runs[1]?.dataVolumeNameSha256);
    expect(withoutRunIdentity(v2.runs[0]!)).toEqual(withoutRunIdentity(v2.runs[1]!));
    expect(withoutRunIdentity(v2.runs[0]!)).toEqual({
      profileSha256: '15d6ff996e0cf5cec2fd269898c6ec470f35d2b8e25da6f2535daa95324f92c7',
      rawTranscriptBytes: 1_124_407,
      rawTranscriptSha256: 'e1f9f698c9778f3e80eec44346e5f76305831783c8f28cd7d465cb5a5065b463',
      sessionTranscriptBytes: 2_078_806,
      sessionTranscriptSha256: '2ef4061ee541352db821b16540220a9c356bd117b8921df866d1d783387b95ad',
      recordCount: 4_059,
      recordsBytes: 860_988,
      recordsSha256: 'a108e05f9cfd6d6485a86fe198a87b3800e21986b5c62e6251519de6577d05be',
      inventoryTranscriptBytes: 1_560_477,
      inventoryTranscriptSha256: '1eadd95675449acf353e504463e99a9d032322080abcfbc6387878db81940690',
      witnessTranscriptBytes: 1_648_647,
      witnessTranscriptSha256: '6bfe02214cfef676bed55ca20edf69517a627bc6bcfba3a99c9fe78e18c107bb',
    });
  });

  it('rejects fractional, negative, oversized, and class-inconsistent authority counts', () => {
    expect(() => validateV2Receipt(structuredClone(v2))).not.toThrow();
    const mutate = (change: (value: any) => void): void => {
      const value = structuredClone(v2) as any;
      change(value);
      expect(() => validateV2Receipt(value)).toThrow();
    };
    mutate((value) => { value.witness.plainFalseCount = -1; });
    mutate((value) => { value.witness.columnLocalAtoms = 0.5; });
    mutate((value) => { value.witness.trueArrayAtoms = value.result.records + 1; });
    mutate((value) => {
      value.witness.classCounts.schema.plainTrue += 1;
      value.witness.classCounts.schema.plainFalse -= 1;
    });
  });

  it('records frozen replay commands and gates their additive V3 profiles in CI', () => {
    expect(v2.replay).toEqual({
      minimumRuns: 2,
      requiresDistinctAnonymousDataVolumes: true,
      requiresNoPublishedPorts: true,
      runnerArgv: ['node', 'scripts/replay-postgresql-public-acl-baseline-v2.mjs'],
      captureArgv: ['node', 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
        'CONTAINER_NAME'],
      oracleArgv: ['node', 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
        'CONTAINER_NAME'],
      witnessArgv: ['node', 'scripts/verify-postgresql-public-acl-has-privilege-v1.mjs',
        'CONTAINER_NAME'],
    });
    const workflow = readFileSync(resolve(ROOT, '../../.github/workflows/ci.yml'), 'utf8');
    const start = workflow.indexOf('  postgresql-public-acl-replay:');
    const end = workflow.indexOf('\n  build:', start);
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    const job = workflow.slice(start, end);
    expect(job.match(/- node: '[^']+'/gu)).toEqual(["- node: '20.0.0'", "- node: '24.14.1'"]);
    expect(job.match(/docker pull postgres@sha256:/gu)).toHaveLength(1);
    const v1Command = 'node coding-harness/supervisor-service/scripts/replay-postgresql-public-acl-baseline-v1.mjs';
    const v2Command = 'node coding-harness/supervisor-service/scripts/replay-postgresql-public-acl-baseline-v2.mjs';
    expect(job).not.toContain(v1Command);
    expect(job).not.toContain(v2Command);
    const v3Command = 'node coding-harness/supervisor-service/scripts/'
      + 'replay-postgresql-public-acl-suite-v3.mjs';
    expect(job.split(`${v3Command} baseline-v1`)).toHaveLength(2);
    expect(job.split(`${v3Command} baseline-v2`)).toHaveLength(2);
    expect(job.indexOf(`${v3Command} baseline-v1`))
      .toBeLessThan(job.indexOf(`${v3Command} baseline-v2`));
  });

  it('keeps every V2 evidence file outside the non-deployable oracle artifact', () => {
    const artifact = JSON.parse(
      readFileSync(resolve(ROOT, '.service/artifact.json'), 'utf8'),
    ) as { buildInputs: Record<string, string>; sourceInputs: Record<string, string> };
    const inputs = [...Object.keys(artifact.buildInputs), ...Object.keys(artifact.sourceInputs)];
    EVIDENCE_PATHS.forEach((path) => expect(inputs).not.toContain(path));
  });

  it('keeps every new code, SQL, and test source within the 500-line limit', () => {
    EVIDENCE_PATHS.filter((path) => !path.endsWith('.json')).forEach((path) => {
      const source = readFileSync(resolve(ROOT, path), 'utf8');
      expect(source.endsWith('\n')).toBe(true);
      expect(source.split('\n').length - 1, path).toBeLessThanOrEqual(500);
    });
  });
});

function parseCanonical(source: Buffer): unknown {
  const text = new TextDecoder('utf-8', { fatal: true }).decode(source);
  expect(text.endsWith('\n')).toBe(true);
  expect(text.includes('\r') || text.includes('\0') || text.startsWith('\uFEFF')).toBe(false);
  const value: unknown = JSON.parse(text);
  expect(`${JSON.stringify(value, null, 2)}\n`).toBe(text);
  return value;
}

function withoutRunIdentity(run: ReceiptRun): Record<string, string | number | boolean> {
  const { sequence: _sequence, networkMode: _network, publishedPorts: _ports,
    dataVolumeNameSha256: _volume, ...deterministic } = run;
  return deterministic;
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

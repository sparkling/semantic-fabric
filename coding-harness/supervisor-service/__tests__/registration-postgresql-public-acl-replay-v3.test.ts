// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
// @ts-expect-error The test exercises the private JavaScript lifecycle gate directly.
import { finalPostgresServerReady } from '../scripts/postgresql-public-acl-replay-support-v3.mjs';
import {
  buildSummary, validateReplayContract, validateRun, validateWitness,
  verifyPredecessorClosure,
// @ts-expect-error The test exercises the private JavaScript contract validator directly.
} from '../scripts/replay-postgresql-public-acl-suite-v3.mjs';

const ROOT = resolve(import.meta.dirname, '..');
const V1_PATH = resolve(
  import.meta.dirname, 'fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json',
);
const V2_PATH = resolve(
  import.meta.dirname, 'fixtures/postgresql-16.15-public-acl-capture-receipt-v2.json',
);
const V3_PATH = resolve(
  import.meta.dirname, 'fixtures/postgresql-16.15-public-acl-replay-contract-v3.json',
);
const V1_PIN = Object.freeze({
  bytes: 4_835,
  sha256: '14fbd3ff2d2b50d3a8adbe0b51dc921eb926cd644a4a765183723518ec4fd08b',
});
const V2_PIN = Object.freeze({
  bytes: 8_816,
  sha256: '48d54b635ff6bafc6bdb4ffcb1bb9d74c8357e932e22f7b6453bb54cb0d698e8',
});
const V3_PIN = Object.freeze({
  bytes: 7_026,
  sha256: 'eb4346bf5d463dc0e0433c0a32e3c16670778a0a98538f5e2a66162cc36bf957',
});
const LEGACY_IMPLEMENTATION_PINS = Object.freeze({
  'scripts/replay-postgresql-public-acl-baseline-v1.mjs': Object.freeze({
    bytes: 26_662,
    sha256: '8c624596bd228d62e9ec01c2d700996371ec0cce25a78badcaa92d826e4fddbc',
  }),
  'scripts/postgresql-public-acl-replay-support-v2.mjs': Object.freeze({
    bytes: 12_519,
    sha256: 'f70f4462fae6b050eaff2a1ef6226813f2819c263a68349ac709146a6ec33398',
  }),
  'scripts/replay-postgresql-public-acl-baseline-v2.mjs': Object.freeze({
    bytes: 24_214,
    sha256: '52c04914c751b01dff72d8d511b966cf1effbcc92c631221bb8bd888b6f93db9',
  }),
});
const IMAGE =
  'postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8';
const PROFILES = Object.freeze(['baseline-v1', 'baseline-v2', 'branch', 'final-where']);

interface ReceiptSource {
  readonly path: string;
  readonly bytes: number;
  readonly sha256: string;
}

interface V3ReplayContract {
  readonly schemaVersion: string;
  readonly authority: string;
  readonly contractDate: string;
  readonly predecessors: readonly Readonly<Record<string, unknown>>[];
  readonly historicalFailure: Readonly<Record<string, unknown>>;
  readonly image: Readonly<Record<string, unknown>>;
  readonly sources: Readonly<Record<string, ReceiptSource>>;
  readonly lifecycle: Readonly<Record<string, unknown>>;
  readonly replay: {
    readonly profiles: readonly Readonly<Record<string, unknown>>[];
    readonly [key: string]: unknown;
  };
}

interface BaselineReceipt {
  readonly profile: { readonly sha256: string };
  readonly result: { readonly records: number; readonly bytes: number; readonly sha256: string };
  readonly sources: {
    readonly [key: string]: { readonly path?: string; readonly bytes: number;
      readonly sha256: string };
    readonly projection: { readonly bytes: number; readonly sha256: string };
    readonly rawOracle: { readonly bytes: number; readonly sha256: string };
    readonly witnessInventorySql: { readonly bytes: number; readonly sha256: string };
    readonly witnessSql: { readonly bytes: number; readonly sha256: string };
  };
  readonly runs: readonly [{
    readonly rawTranscriptBytes: number; readonly rawTranscriptSha256: string;
    readonly sessionTranscriptBytes: number; readonly sessionTranscriptSha256: string;
    readonly inventoryTranscriptBytes: number; readonly inventoryTranscriptSha256: string;
    readonly witnessTranscriptBytes: number; readonly witnessTranscriptSha256: string;
  }];
  readonly witness: Readonly<Record<string, unknown>> & {
    readonly name: string; readonly roleName: string;
    readonly classCounts: Readonly<Record<string, unknown>>;
  };
}

describe('PostgreSQL 16.15 PUBLIC ACL additive V3 replay lifecycle', () => {
  it('preserves every historical V1/V2 receipt and replay implementation byte exactly', () => {
    const v1 = readFileSync(V1_PATH);
    const v2 = readFileSync(V2_PATH);
    expect({ bytes: v1.byteLength, sha256: sha256(v1) }).toEqual(V1_PIN);
    expect({ bytes: v2.byteLength, sha256: sha256(v2) }).toEqual(V2_PIN);
    Object.entries(LEGACY_IMPLEMENTATION_PINS).forEach(([path, pin]) => {
      const source = readFileSync(resolve(ROOT, path));
      expect({ bytes: source.byteLength, sha256: sha256(source) }, path).toEqual(pin);
      expect(source.toString('utf8'), path).not.toContain('/proc/1/comm');
    });
  });

  it('accepts readiness only after the temporary init server becomes PID 1 postgres', () => {
    const result = (status: number, stdout = '') => ({
      error: undefined, signal: null, status,
      stdout: Buffer.from(stdout), stderr: Buffer.alloc(0),
    });
    expect(finalPostgresServerReady(result(0, 'docker-entrypoi\n'), result(0))).toBe(false);
    expect(finalPostgresServerReady(result(0, 'postgres\n'), result(1))).toBe(false);
    expect(finalPostgresServerReady(result(0, 'postgres\n'), result(0))).toBe(true);
  });

  it('pins an additive V3 replay contract over both predecessors and the hosted failure', () => {
    const source = readFileSync(V3_PATH);
    expect({ bytes: source.byteLength, sha256: sha256(source) }).toEqual(V3_PIN);
    const receipt = parseCanonical(source) as V3ReplayContract;
    expect(() => validateReplayContract(structuredClone(receipt))).not.toThrow();
    expect(receipt.schemaVersion)
      .toBe('semantic-fabric.postgresql-public-acl-replay-contract/v3');
    expect(receipt.authority).toBe('test-only-non-runtime');
    expect(receipt.predecessors).toEqual([
      {
        path: '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json',
        schemaVersion: 'semantic-fabric.postgresql-public-acl-capture-receipt/v1',
        ...V1_PIN,
        disposition: 'preserved-historical-no-rerun',
      },
      {
        path: '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v2.json',
        schemaVersion: 'semantic-fabric.postgresql-public-acl-capture-receipt/v2',
        ...V2_PIN,
        disposition: 'preserved-historical-no-rerun',
      },
    ]);
    expect(receipt.historicalFailure).toEqual({
      runId: 33_612_211_004,
      headSha: 'a2a20e33239bf5d8210b75b51189d96f5cb1ecfe',
      diagnosis: 'temporary-init-server-readiness-race',
      jobs: [
        { node: '24.14.1', jobId: 100_189_827_960, error: 'REPLAY_PSQL_FAILED' },
        { node: '20.0.0', jobId: 100_189_828_074, error: 'REPLAY_DATABASE_CREATE_FAILED' },
      ],
      replayPolicy: 'historical-runners-not-invoked',
    });
  });

  it('pins every V3 source and keeps the exact image and eight-run isolation contract', () => {
    const receipt = parseCanonical(readFileSync(V3_PATH)) as V3ReplayContract;
    Object.values(receipt.sources).forEach((pin) => {
      const source = readFileSync(resolve(ROOT, pin.path));
      expect({ bytes: source.byteLength, sha256: sha256(source) }, pin.path).toEqual({
        bytes: pin.bytes, sha256: pin.sha256,
      });
    });
    expect(receipt.image.reference).toBe(IMAGE);
    expect(receipt.lifecycle).toEqual({
      pid1CommPath: '/proc/1/comm', expectedPid1Comm: 'postgres\n',
      readinessArgv: ['pg_isready', '-q', '-U', 'postgres', '-d', 'postgres'],
      acceptance: 'pid1-and-readiness', timeoutMilliseconds: 60_000,
    });
    expect(receipt.replay).toMatchObject({
      minimumRunsPerProfile: 2, totalMinimumRuns: 8,
      requiresDistinctAnonymousDataVolumes: true, requiresNoPublishedPorts: true,
      requiresNetworkMode: 'none',
    });
    expect(receipt.replay.profiles.map((profile) => profile.name)).toEqual(PROFILES);
    expect(JSON.stringify(receipt)).not.toContain('"volumeName"');
  });

  it('emits only hashed anonymous volume identities in the V3 replay summary', () => {
    const rawVolumes = ['a'.repeat(64), 'b'.repeat(64)];
    const runs = rawVolumes.map((volumeName, index) => ({
      sequence: index + 1,
      volumeName,
      capture: { dataVolumeNameSha256: index === 0 ? 'c'.repeat(64) : 'd'.repeat(64) },
      oracle: { dataVolumeNameSha256: index === 0 ? 'c'.repeat(64) : 'd'.repeat(64) },
    }));
    const summary = buildSummary(
      'baseline-v1', runs, { predecessors: [] }, Buffer.from('{}\n', 'utf8'),
    ) as Readonly<Record<string, unknown>>;
    const output = JSON.stringify(summary);
    expect(output).not.toContain('"volumeName"');
    rawVolumes.forEach((name) => expect(output).not.toContain(name));
    expect(output).toContain('"dataVolumeNameSha256"');
  });

  it('routes a branch run through V2 lineage and exact seeded-control validation', () => {
    const receipt = parseCanonical(readFileSync(V3_PATH)) as V3ReplayContract;
    const v1 = parseCanonical(readFileSync(V1_PATH)) as BaselineReceipt;
    const v2 = parseCanonical(readFileSync(V2_PATH)) as BaselineReceipt;
    const volumeHash = 'f'.repeat(64);
    const captureCounts = { column: 16, 'foreign-data-wrapper': 0, 'foreign-server': 0,
      language: 4, relation: 189, routine: 3_235, schema: 2, type: 613 };
    const oracleCounts = { column: 16, language: 4, relation: 189, routine: 3_235,
      schema: 2, type: 613 };
    const capture = {
      schemaVersion: 1, profile: 'postgresql-16.15-clean-template0-public-object-acl-v1',
      image: IMAGE,
      imageConfiguration: 'sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2',
      platform: 'linux/amd64', dataVolumeNameSha256: volumeHash,
      profileSha256: v2.profile.sha256, projectionBytes: v2.sources.projection.bytes,
      projectionSha256: v2.sources.projection.sha256, recordCount: v2.result.records,
      recordsBytes: v2.result.bytes, recordsSha256: v2.result.sha256,
      classCounts: captureCounts,
    };
    const oracle = {
      schemaVersion: 1, oracle: 'postgresql-16.15-public-acl-completeness-oracle-v1',
      image: IMAGE, imageConfiguration: capture.imageConfiguration,
      platform: 'linux/amd64', dataVolumeNameSha256: volumeHash,
      oracleSourceBytes: v2.sources.rawOracle.bytes,
      oracleSourceSha256: v2.sources.rawOracle.sha256,
      projectionSourceBytes: v2.sources.projection.bytes,
      projectionSourceSha256: v2.sources.projection.sha256,
      rawTranscriptBytes: v2.runs[0].rawTranscriptBytes,
      rawTranscriptSha256: v2.runs[0].rawTranscriptSha256,
      sessionTranscriptBytes: v2.runs[0].sessionTranscriptBytes,
      sessionTranscriptSha256: v2.runs[0].sessionTranscriptSha256,
      recordCount: v2.result.records, recordsBytes: v2.result.bytes,
      recordsSha256: v2.result.sha256, classCounts: oracleCounts,
    };
    const pin = (key: string) => {
      const source = receipt.sources[key];
      expect(source).toBeDefined();
      return { bytes: source!.bytes, sha256: source!.sha256 };
    };
    const witness = {
      schemaVersion: 1, verifier: 'postgresql-16.15-public-acl-projection-mutations-v1',
      authority: 'test-only-non-runtime', image: IMAGE,
      imageConfiguration: capture.imageConfiguration, platform: 'linux/amd64',
      dataVolumeNameSha256: volumeHash,
      sources: { projection: pin('projection'), rawOracle: pin('rawOracle'),
        mutator: pin('branchMutator'), replaySupport: pin('mutationReplaySupport'),
        verifier: pin('branchVerifier') },
      seed: { foreignDataWrapperName: 'sf_public_acl_mutation_fdw_v1',
        foreignServerName: 'sf_public_acl_mutation_server_v1', publicAclAtoms: 2 },
      rawTranscriptBytes: 1_124_809,
      rawTranscriptSha256: '004d74db066cd41b78a739cf8030211ebe450ddcd5fee810e6b744e66dbf2c7f',
      sessionTranscriptBytes: 12_584_275,
      sessionTranscriptSha256: 'b2db44b9feed4ea7c6dd4b294abfa8a3334fb8139d9817ada9d30851cd88da22',
      control: { recordCount: 4_061, recordsBytes: 861_437,
        recordsSha256: '2ded27910e4aae2d44242ca7ea5c4ebd9c30ce10ad541a1c6f916bdbddf34935',
        normalizedEquivalent: true, classCounts: { schema: 2, relation: 189, column: 16,
          routine: 3_235, type: 613, language: 4, 'foreign-data-wrapper': 1,
          'foreign-server': 1 } },
      mutations: branchMutations(),
      cleanup: { transactionRolledBack: true, foreignDataWrapperAbsent: true,
        foreignServerAbsent: true },
    };
    expect(() => validateRun('branch', {
      sequence: 1, volumeName: 'e'.repeat(64), capture, oracle, witness,
    }, 1, receipt, v1, v2)).not.toThrow();
    const fabricated = structuredClone(witness);
    fabricated.mutations[0]!.id = 'fabricated-branch';
    expect(() => validateRun('branch', {
      sequence: 1, volumeName: 'e'.repeat(64), capture, oracle, witness: fabricated,
    }, 1, receipt, v1, v2)).toThrow('REPLAY_V3_BRANCH_INVALID');
  });

  it('retains V2 witness source and transcript authority in the V3 gate', () => {
    const v2 = parseCanonical(readFileSync(V2_PATH)) as BaselineReceipt;
    const witness = v2Witness(v2, 'f'.repeat(64));
    expect(() => validateWitness(witness, v2)).not.toThrow();
    const drifted = structuredClone(witness);
    drifted.inventorySourceSha256 = '0'.repeat(64);
    expect(() => validateWitness(drifted, v2))
      .toThrow('REPLAY_V3_WITNESS_SOURCE_OR_TRANSCRIPT_INVALID');
  });

  it('verifies every transitive predecessor source and the result fixture before replay', () => {
    const v2 = parseCanonical(readFileSync(V2_PATH)) as BaselineReceipt;
    expect(() => verifyPredecessorClosure(v2)).not.toThrow();
    const sourceDrift = structuredClone(v2);
    Object.assign(sourceDrift.sources.oracleWire!, { sha256: '0'.repeat(64) });
    expect(() => verifyPredecessorClosure(sourceDrift))
      .toThrow('REPLAY_V3_PREDECESSOR_SOURCE_PIN_MISMATCH');
    const fixtureDrift = structuredClone(v2);
    Object.assign(fixtureDrift.result, { sha256: '0'.repeat(64) });
    expect(() => verifyPredecessorClosure(fixtureDrift))
      .toThrow('REPLAY_V3_PREDECESSOR_FIXTURE_PIN_MISMATCH');
  });

  it('configures hosted CI to run only the four additive V3 profiles', () => {
    const workflow = readFileSync(resolve(ROOT, '../../.github/workflows/ci.yml'), 'utf8');
    const start = workflow.indexOf('  postgresql-public-acl-replay:');
    const end = workflow.indexOf('\n  build:', start);
    const job = workflow.slice(start, end);
    const runner = 'node coding-harness/supervisor-service/scripts/'
      + 'replay-postgresql-public-acl-suite-v3.mjs';
    expect(PROFILES.map((profile) => `${runner} ${profile}`)
      .every((command) => job.includes(command))).toBe(true);
    expect((job.match(/replay-postgresql-public-acl-suite-v3\.mjs/gu) ?? [])).toHaveLength(4);
    expect(job).not.toContain('replay-postgresql-public-acl-baseline-v1.mjs');
    expect(job).not.toContain('replay-postgresql-public-acl-baseline-v2.mjs');
    expect(job).not.toContain('verify-postgresql-public-acl-projection-');
  });
});

function branchMutations(): Array<Record<string, unknown>> {
  const classes = [
    ['schema', 2], ['relation', 189], ['column', 16], ['routine', 3_235], ['type', 613],
    ['language', 4], ['foreign-data-wrapper', 1], ['foreign-server', 1],
  ] as const;
  const specs: Array<readonly [string, string, string | null, string, number]> = [
    ...classes.map(([objectClass, count]) => [`delete-${objectClass}-branch`,
      'branch-deletion', objectClass, 'ORACLE_RECORD_BAG_KEYS_MISMATCH', -count] as const),
    ['return-zero', 'record-set-sensitivity', null, 'ORACLE_RECORD_BAG_KEYS_MISMATCH', -4_061],
    ['omit-first-atom', 'record-set-sensitivity', null,
      'ORACLE_RECORD_BAG_KEYS_MISMATCH', -1],
    ['add-sentinel-atom', 'record-set-sensitivity', null,
      'ORACLE_RECORD_BAG_KEYS_MISMATCH', 1],
    ['substitute-first-atom', 'record-set-sensitivity', null,
      'ORACLE_RECORD_BAG_MULTIPLICITY_MISMATCH', 0],
  ];
  return specs.map(([id, kind, objectClass, oracleRejection, recordDelta], index) => ({
    sequence: index + 1, id, kind, objectClass, sourceBytes: 1,
    sourceSha256: '1'.repeat(64), executed: true, parsed: true, oracleRejection, recordDelta,
    recordCount: 4_061 + recordDelta, recordsBytes: 1, recordsSha256: '2'.repeat(64),
  }));
}

function v2Witness(receipt: BaselineReceipt, volumeHash: string) {
  const authority = receipt.witness;
  const expected = receipt.runs[0];
  return {
    schemaVersion: 1, witness: authority.name, authority: 'test-only-non-runtime',
    roleName: authority.roleName, image: IMAGE,
    imageConfiguration: 'sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2',
    platform: 'linux/amd64', dataVolumeNameSha256: volumeHash,
    inventorySourceBytes: receipt.sources.witnessInventorySql.bytes,
    inventorySourceSha256: receipt.sources.witnessInventorySql.sha256,
    witnessSourceBytes: receipt.sources.witnessSql.bytes,
    witnessSourceSha256: receipt.sources.witnessSql.sha256,
    fixtureBytes: receipt.result.bytes, fixtureSha256: receipt.result.sha256,
    inventoryTranscriptBytes: expected.inventoryTranscriptBytes,
    inventoryTranscriptSha256: expected.inventoryTranscriptSha256,
    witnessTranscriptBytes: expected.witnessTranscriptBytes,
    witnessTranscriptSha256: expected.witnessTranscriptSha256,
    inventoryEntries: authority.inventoryEntries, checkCount: authority.checkCount,
    plainTrueCount: authority.plainTrueCount, plainFalseCount: authority.plainFalseCount,
    grantOptionTrueCount: authority.grantOptionTrueCount,
    corroboratedAtoms: authority.corroboratedAtoms, columnLocalAtoms: authority.columnLocalAtoms,
    trueArrayAtoms: authority.trueArrayAtoms, inventoryBytes: authority.inventoryBytes,
    inventorySha256: authority.inventorySha256, observationsBytes: authority.observationsBytes,
    observationsSha256: authority.observationsSha256,
    classCounts: structuredClone(authority.classCounts),
    cleanup: { preflightAbsent: true, created: true, dropped: true, postDropAbsent: true },
  };
}

function parseCanonical(source: Buffer): unknown {
  const text = new TextDecoder('utf-8', { fatal: true }).decode(source);
  expect(text.endsWith('\n') && !text.includes('\r') && !text.includes('\0')).toBe(true);
  const value: unknown = JSON.parse(text);
  expect(`${JSON.stringify(value, null, 2)}\n`).toBe(text);
  return value;
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

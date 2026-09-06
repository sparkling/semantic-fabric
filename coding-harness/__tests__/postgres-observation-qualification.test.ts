import { canonical } from '@metaharness/harness';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  createPostgresObservationQualificationReceipt,
  parsePostgresObservationQualificationCandidate,
  parsePostgresObservationQualificationProbeOutput,
  parsePostgresObservationQualificationReceiptPairJson,
  parsePostgresObservationQualificationReceiptJson,
  verifyPostgresObservationQualificationReceipt,
  verifyPostgresObservationQualificationReceiptPair,
} from '../src/postgres-observation-qualification.js';
import {
  POSTGRES_OBSERVATION_COMPLETED_PHASES,
  POSTGRES_OBSERVATION_STREAM_IDS,
} from '../src/postgres-observation-qualification-runner.js';
import {
  POSTGRES_QUALIFICATION_BUILDER_EXPECTATION,
  POSTGRES_QUALIFICATION_IMAGE_EXPECTATIONS,
  expectedQualificationBuilderImageIdentity,
  expectedQualificationImageIdentity,
} from '../src/postgres-observation-qualification-protocol.js';
import {
  POSTGRES_QUALIFICATION_RECEIPT_PAIR_PATH,
  readPostgresQualificationReceiptPair,
  writePostgresQualificationReceiptPair,
} from '../src/postgres-observation-qualification-io.js';

const hex = (character: string): string => character.repeat(64);
const sha = (character: string): string => `sha256:${hex(character)}`;
const EMPTY_SHA256 = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';

function stream(cap: number, decoded = 1) {
  return {
    cap, polled: decoded, decoded, retainedPeak: decoded, overflow: false,
    terminal: 'complete' as const,
  };
}

const richStreaming = {
  relations: stream(4_096),
  attributes: stream(65_536),
  catalogConstraints: stream(65_535),
};

function queryAccounting() {
  const evidence = POSTGRES_OBSERVATION_STREAM_IDS.map((id) => {
    if (id === 'legacy-tables' || id === 'legacy-earlier-collisions') {
      return { id, evidence: stream(4_096) };
    }
    if (id === 'rich-relations') return { id, evidence: structuredClone(richStreaming.relations) };
    if (id === 'rich-attributes') return { id, evidence: structuredClone(richStreaming.attributes) };
    if (id === 'rich-catalog-constraints') {
      return { id, evidence: structuredClone(richStreaming.catalogConstraints) };
    }
    return { id, evidence: stream(65_536) };
  });
  return {
    prequalificationGuard: 1 as const,
    richCapture: 4 as const,
    guardQueriesObserved: 2 as const,
    streamQueriesObserved: 10 as const,
    totalQueriesObserved: 12 as const,
    streams: evidence,
  };
}

function candidateFor(
  patch: '16.9' | '16.15' = '16.15',
  outputDigest = patch === '16.9' ? '6' : '1',
) {
  const serverVersionNum = patch === '16.9' ? 160009 as const : 160015 as const;
  const imageExpectation = POSTGRES_QUALIFICATION_IMAGE_EXPECTATIONS.find(
    (image) => image.patch === patch,
  )!;
  return {
    provenance: {
      source: {
        commit: 'a'.repeat(40),
        tree: 'b'.repeat(40),
        cargoLockSha256: hex('c'),
        probeArtifactSha256: hex('d'),
        probeStdoutSha256: hex(outputDigest),
      },
      inputs: {
        adrSha256: hex('e'),
        profileSha256: hex('f'),
        queriesSha256: hex('1'),
        testsSha256: hex('2'),
        fixtureSha256: hex('3'),
        runnerSha256: hex('4'),
        protocolSha256: hex('5'),
      },
      toolchain: {
        rustcVvSha256: hex('6'),
        cargoVersionSha256: hex('7'),
        targetTriple: 'x86_64-unknown-linux-gnu' as const,
        builderImage: expectedQualificationBuilderImageIdentity(
          POSTGRES_QUALIFICATION_BUILDER_EXPECTATION,
        ),
      },
      image: expectedQualificationImageIdentity(imageExpectation),
    },
    preflight: {
      networkMode: 'none' as const,
      fixedDatabase: true as const,
      unixSocket: true as const,
      comparisonRole: {
        superuser: false as const,
        databaseOwner: false as const,
        inherit: false as const,
        bypassRls: false as const,
        canSetRole: false as const,
        canDdl: false as const,
        privilegesExact: true as const,
      },
      ownerIdentityEqual: true as const,
      legacyConstraintVisibilityDiffers: true as const,
    },
    observation: {
      serverVersion: `PostgreSQL ${patch} (Debian)`,
      serverVersionNum,
      guard: 'pass' as const,
      rowCounts: {
        relations: 1,
        attributes: 1,
        notNullConstraints: 1,
        catalogConstraints: 1,
        combinedConstraints: 2,
      },
      streaming: structuredClone(richStreaming),
      identity: { structural: hex('b'), types: hex('c'), constraints: hex('d') },
      legacyCoordinateComparison: 'equal' as const,
      errorCode: null,
      failurePhase: null,
    },
    queryAccounting: queryAccounting(),
    lifecycle: {
      savepoint: 'released' as const,
      recovery: 'not-needed' as const,
      commit: 'complete' as const,
      phasesCompleted: [...POSTGRES_OBSERVATION_COMPLETED_PHASES],
    },
  };
}

function execution(
  candidate: ReturnType<typeof candidateFor>,
  slot: 1 | 2,
  resource: string,
) {
  return {
    slot,
    containerIdentitySha256: hex(resource),
    volumeIdentitySha256: hex(resource === 'a' ? 'b' : resource === 'c' ? 'd' : resource),
    imageConfigDigestBefore: candidate.provenance.image.configDigest,
    imageConfigDigestAfter: candidate.provenance.image.configDigest,
    stdout: {
      bytes: 512, sha256: candidate.provenance.source.probeStdoutSha256, truncated: false as const,
    },
    stderr: { bytes: 0, sha256: EMPTY_SHA256, truncated: false as const },
    exitCode: 0 as const,
    timedOut: false as const,
    outputLimitExceeded: false as const,
    cleanup: {
      containerRemoved: true as const,
      volumeRemoved: true as const,
      labelledContainersRemaining: 0 as const,
      labelledVolumesRemaining: 0 as const,
      verificationSha256: hex(slot === 1 ? '3' : '4'),
    },
  };
}

function receiptFromCandidate(
  candidate: ReturnType<typeof candidateFor>,
  resources: [string, string] = ['a', 'c'],
) {
  return createPostgresObservationQualificationReceipt({
    candidates: [candidate, structuredClone(candidate)],
    executions: [execution(candidate, 1, resources[0]), execution(candidate, 2, resources[1])],
  });
}

function receiptFor(patch: '16.9' | '16.15', resources: [string, string] = ['a', 'c']) {
  return receiptFromCandidate(candidateFor(patch), resources);
}

describe('PostgreSQL observation qualification receipt', () => {
  it('binds the exact successful observation without promoting runtime authority', () => {
    const receipt = receiptFor('16.15');
    expect(receipt).toMatchObject({
      authority: 'observation-profile-only',
      observationProfileQualification: 'pass',
      runtimeAdmissionStatus: 'withheld-independent-gates',
      productionAdmission: false,
      verifiedLease: false,
      reload: false,
      directMapping: false,
      replayStatus: 'pass',
    });
    expect(receipt.replay.executions.map((value) => value.stdout.sha256))
      .toEqual([hex('1'), hex('1')]);
    expect(receipt.replay.executions[0].candidateSha256)
      .toBe(receipt.replay.executions[1].candidateSha256);
    expect(() => verifyPostgresObservationQualificationReceipt(receipt)).not.toThrow();
  });

  it('requires all five candidate sections and the exact accounting/lifecycle inventories', () => {
    const candidate = candidateFor();
    const missingAccounting = structuredClone(candidate) as Record<string, unknown>;
    delete missingAccounting.queryAccounting;
    expect(() => parsePostgresObservationQualificationCandidate(missingAccounting))
      .toThrow(/queryAccounting/);

    const reordered = structuredClone(candidate);
    [reordered.queryAccounting.streams[0], reordered.queryAccounting.streams[1]] =
      [reordered.queryAccounting.streams[1], reordered.queryAccounting.streams[0]];
    expect(() => parsePostgresObservationQualificationCandidate(reordered)).toThrow(/stream.*order/i);

    const wrongLegacyCap = structuredClone(candidate);
    wrongLegacyCap.queryAccounting.streams[0].evidence.cap = 65_536;
    expect(() => parsePostgresObservationQualificationCandidate(wrongLegacyCap))
      .toThrow(/cap is inconsistent/i);

    const richDrift = structuredClone(candidate);
    richDrift.queryAccounting.streams[7].evidence.decoded = 0;
    richDrift.queryAccounting.streams[7].evidence.polled = 0;
    richDrift.queryAccounting.streams[7].evidence.retainedPeak = 0;
    expect(() => parsePostgresObservationQualificationCandidate(richDrift))
      .toThrow(/rich stream evidence/);

    const wrongPhaseOrder = structuredClone(candidate);
    [wrongPhaseOrder.lifecycle.phasesCompleted[8], wrongPhaseOrder.lifecycle.phasesCompleted[9]] =
      [wrongPhaseOrder.lifecycle.phasesCompleted[9], wrongPhaseOrder.lifecycle.phasesCompleted[8]];
    expect(() => parsePostgresObservationQualificationCandidate(wrongPhaseOrder))
      .toThrow(/phase.*order/i);
    expect(POSTGRES_OBSERVATION_COMPLETED_PHASES.slice(-2))
      .toEqual(['legacy-comparison', 'identity-build']);

    const wrongBuilder = structuredClone(candidate);
    wrongBuilder.provenance.toolchain.builderImage.configDigest = sha('8');
    expect(() => parsePostgresObservationQualificationCandidate(wrongBuilder))
      .toThrow(/builder image identity/);

    const failed = JSON.parse(JSON.stringify(candidate));
    failed.observation.guard = 'fail';
    failed.observation.identity = null;
    failed.observation.legacyCoordinateComparison = 'unavailable';
    failed.observation.errorCode = 'CatalogQuery';
    failed.observation.failurePhase = 'guard';
    expect(() => parsePostgresObservationQualificationCandidate(failed))
      .toThrow(/guard result/);
  });

  it('strictly parses duplicate-free closed probe and receipt JSON', () => {
    const candidate = candidateFor();
    const { provenance: _provenance, ...probe } = candidate;
    expect(parsePostgresObservationQualificationProbeOutput(canonical(probe)))
      .toEqual(probe);
    expect(() => parsePostgresObservationQualificationProbeOutput(
      canonical(probe).replace('"savepoint":"released"',
        '"savepoint":"released","savepoint":"released"'),
    )).toThrow(/duplicate JSON key/);

    const receipt = receiptFor('16.15');
    const serialized = canonical(receipt);
    expect(parsePostgresObservationQualificationReceiptJson(serialized)).toEqual(receipt);
    expect(() => parsePostgresObservationQualificationReceiptJson(`${serialized}\n`))
      .toThrow(/exact canonical JSON bytes/);
    expect(() => parsePostgresObservationQualificationReceiptJson(JSON.stringify(receipt, null, 2)))
      .toThrow(/exact canonical JSON bytes/);
    expect(() => parsePostgresObservationQualificationReceiptJson(
      serialized.replace('"authority":"observation-profile-only"',
        '"authority":"observation-profile-only","authority":"observation-profile-only"'),
    )).toThrow(/duplicate JSON key/);
    expect(() => parsePostgresObservationQualificationReceiptJson(
      canonical({ ...receipt, surprise: true }),
    )).toThrow(/invalid keys/);

    const pair = [receiptFor('16.9', ['5', '6']), receipt] as const;
    expect(parsePostgresObservationQualificationReceiptPairJson(canonical(pair))).toEqual(pair);
    expect(() => parsePostgresObservationQualificationReceiptPairJson(
      canonical([pair[1], pair[0]]),
    )).toThrow(/order must be 16\.9 then 16\.15/);
    expect(() => parsePostgresObservationQualificationReceiptPairJson(
      `${canonical(pair)}\n`,
    )).toThrow(/exact canonical JSON bytes/);
  });

  it('publishes the two receipts through one atomically replaceable pair artifact', () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'semantic-fabric-pgq-io-')));
    try {
      mkdirSync(resolve(root, 'tests/postgresql'), { recursive: true });
      const first = [
        receiptFor('16.9', ['5', '6']), receiptFor('16.15', ['a', 'c']),
      ] as const;
      writePostgresQualificationReceiptPair(root, [first[1], first[0]]);
      expect(readPostgresQualificationReceiptPair(root)).toEqual(first);
      expect(readFileSync(resolve(root, POSTGRES_QUALIFICATION_RECEIPT_PAIR_PATH), 'utf8'))
        .toBe(`${canonical(first)}\n`);

      const replacement = [
        receiptFor('16.9', ['7', '8']), receiptFor('16.15', ['9', 'e']),
      ] as const;
      writePostgresQualificationReceiptPair(root, replacement);
      expect(readPostgresQualificationReceiptPair(root)).toEqual(replacement);
      expect(readdirSync(resolve(root, 'tests/postgresql')))
        .toEqual(['postgresql-16-observation-qualification-receipt-pair-v1.json']);
      expect(existsSync(resolve(
        root, 'tests/postgresql/postgresql-16.9-observation-qualification-receipt-v1.json',
      ))).toBe(false);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it('rejects unbound output, nonempty stderr, reused resources, drift, and incomplete cleanup', () => {
    const candidate = candidateFor();
    const base = [execution(candidate, 1, 'a'), execution(candidate, 2, 'c')] as const;
    const create = (executions: unknown[]) => createPostgresObservationQualificationReceipt({
      candidates: [candidate, structuredClone(candidate)], executions,
    });
    const differentCandidate = structuredClone(candidate);
    differentCandidate.observation.identity!.constraints = hex('a');
    expect(() => createPostgresObservationQualificationReceipt({
      candidates: [candidate, differentCandidate], executions: base,
    })).toThrow(/not byte equal/);
    expect(() => create([base[0], {
      ...base[1], stdout: { ...base[1].stdout, sha256: hex('f') },
    }])).toThrow(/stdout.*provenance/i);
    expect(() => create([base[0], {
      ...base[1], stderr: { ...base[1].stderr, sha256: hex('f') },
    }])).toThrow(/empty SHA-256/i);
    expect(() => create([base[0], {
      ...base[1], stderr: { ...base[1].stderr, bytes: 1 },
    }])).toThrow(/empty SHA-256/i);
    expect(() => create([base[0], { ...base[1], containerIdentitySha256: hex('a') }]))
      .toThrow(/containers.*distinct/);
    expect(() => create([base[0], {
      ...base[1], imageConfigDigestAfter: sha('7'),
    }])).toThrow(/image configuration/);
    expect(() => create([base[0], {
      ...base[1], cleanup: { ...base[1].cleanup, volumeRemoved: false },
    }])).toThrow(/cleanup/);
  });

  it('requires a coherent 16.9 plus 16.15 pair with identical critical bindings', () => {
    const pg169 = receiptFor('16.9', ['5', '6']);
    const pg1615 = receiptFor('16.15', ['a', 'c']);
    expect(() => verifyPostgresObservationQualificationReceiptPair([pg1615, pg169]))
      .not.toThrow();
    expect(() => verifyPostgresObservationQualificationReceiptPair([pg1615, pg1615]))
      .toThrow(/16\.9.*16\.15/);
    expect(() => verifyPostgresObservationQualificationReceiptPair([
      pg1615, receiptFor('16.9', ['a', 'c']),
    ])).toThrow(/cross-patch containers.*distinct/);

    const criticalCandidate = candidateFor('16.9');
    criticalCandidate.provenance.inputs.protocolSha256 = hex('a');
    const criticalDrift = receiptFromCandidate(criticalCandidate, ['5', '6']);
    expect(() => verifyPostgresObservationQualificationReceiptPair([pg1615, criticalDrift]))
      .toThrow(/critical bindings/);

    const observationCandidate = candidateFor('16.9');
    observationCandidate.observation.identity!.types = hex('a');
    const observationDrift = receiptFromCandidate(observationCandidate, ['5', '6']);
    expect(() => verifyPostgresObservationQualificationReceiptPair([pg1615, observationDrift]))
      .toThrow(/cross-patch observation/);

    const imageCandidate = JSON.parse(JSON.stringify(candidateFor('16.9')));
    imageCandidate.provenance.image.configDigest = sha('7');
    imageCandidate.provenance.image.inspectSha256 = hex('7');
    const imageDrift = receiptFromCandidate(imageCandidate, ['5', '6']);
    expect(() => verifyPostgresObservationQualificationReceiptPair([pg1615, imageDrift]))
      .toThrow(/image identity is not pinned/);

    expect(() => verifyPostgresObservationQualificationReceipt({
      ...pg1615, productionAdmission: true,
    })).toThrow(/production admission/);
    expect(() => verifyPostgresObservationQualificationReceipt({
      ...pg1615, runtimeAdmissionStatus: 'admitted',
    })).toThrow(/runtime admission/);
    expect(() => verifyPostgresObservationQualificationReceipt({
      ...pg1615, observationProfileQualification: 'fail', replayStatus: 'fail',
    })).toThrow(/observation profile qualification/);
  });
});

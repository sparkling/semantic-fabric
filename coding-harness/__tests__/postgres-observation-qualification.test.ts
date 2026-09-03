import { describe, expect, it } from 'vitest';
import {
  createPostgresObservationQualificationReceipt,
  parsePostgresObservationQualificationCandidate,
  verifyPostgresObservationQualificationReceipt,
} from '../src/postgres-observation-qualification.js';
import {
  POSTGRES_OBSERVATION_QUALIFICATION_RUNNER_STATUS,
  runPostgresObservationQualificationProbe,
} from '../src/postgres-observation-qualification-runner.js';

const hex = (character: string): string => character.repeat(64);
const sha = (character: string): string => `sha256:${hex(character)}`;

const candidate = {
  provenance: {
    source: {
      commit: 'a'.repeat(40),
      tree: 'b'.repeat(40),
      cargoLockSha256: hex('c'),
      probeArtifactSha256: hex('d'),
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
    },
    image: {
      repository: 'postgres' as const,
      manifestDigest: sha('8'),
      configDigest: sha('9'),
      platform: 'linux/amd64' as const,
      inspectSha256: hex('a'),
    },
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
    serverVersion: 'PostgreSQL 16.15 (Debian 16.15-1.pgdg13+2)',
    serverVersionNum: 160015 as const,
    guard: 'pass' as const,
    rowCounts: {
      relations: 1,
      attributes: 1,
      notNullConstraints: 1,
      catalogConstraints: 1,
      combinedConstraints: 2,
    },
    streaming: {
      relations: {
        cap: 4_096, polled: 1, decoded: 1, retainedPeak: 1, overflow: false,
        terminal: 'complete' as const,
      },
      attributes: {
        cap: 65_536, polled: 1, decoded: 1, retainedPeak: 1, overflow: false,
        terminal: 'complete' as const,
      },
      catalogConstraints: {
        cap: 65_535, polled: 1, decoded: 1, retainedPeak: 1, overflow: false,
        terminal: 'complete' as const,
      },
    },
    identity: { structural: hex('b'), types: hex('c'), constraints: hex('d') },
    legacyComparison: 'equal' as const,
    errorCode: null,
    failurePhase: null,
  },
};

function execution(slot: 1 | 2, identity: string) {
  return {
    slot,
    containerIdentitySha256: hex(identity),
    volumeIdentitySha256: hex(slot === 1 ? 'e' : 'f'),
    imageConfigDigestBefore: candidate.provenance.image.configDigest,
    imageConfigDigestAfter: candidate.provenance.image.configDigest,
    stdout: { bytes: 512, sha256: hex('1'), truncated: false as const },
    stderr: { bytes: 0, sha256: hex('2'), truncated: false as const },
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

function receiptInput(overrides: Record<string, unknown> = {}) {
  return {
    candidates: [candidate, structuredClone(candidate)],
    executions: [execution(1, '1'), execution(2, '2')],
    ...overrides,
  };
}

describe('PostgreSQL observation qualification receipt', () => {
  it('binds two distinct exact replays while withholding runtime qualification', () => {
    const receipt = createPostgresObservationQualificationReceipt(receiptInput());
    expect(receipt.replayStatus).toBe('pass');
    expect(receipt.evidenceClass).toBe('test-only-non-runtime');
    expect(receipt.authority).toBe('development-only-no-promotion');
    expect(receipt.qualificationStatus).toBe('withheld-runtime-gaps');
    expect(receipt.productionAdmission).toBe(false);
    expect(receipt.verifiedLease).toBe(false);
    expect(receipt.reload).toBe(false);
    expect(receipt.directMapping).toBe(false);
    expect(receipt.replay.executions[0].candidateSha256)
      .toBe(receipt.replay.executions[1].candidateSha256);
    expect(() => verifyPostgresObservationQualificationReceipt(receipt)).not.toThrow();
  });

  it('rejects unknown fields, tampering, same resources, drift, or unverified cleanup', () => {
    expect(() => createPostgresObservationQualificationReceipt({
      ...receiptInput(), surprise: true,
    })).toThrow(/invalid keys/);
    const receipt = createPostgresObservationQualificationReceipt(receiptInput());
    expect(() => verifyPostgresObservationQualificationReceipt({
      ...receipt, productionAdmission: true,
    })).toThrow();
    expect(() => createPostgresObservationQualificationReceipt(receiptInput({
      executions: [execution(1, '1'), execution(2, '1')],
    }))).toThrow(/distinct containers/);
    expect(() => createPostgresObservationQualificationReceipt(receiptInput({
      executions: [execution(1, '1'), {
        ...execution(2, '2'), imageConfigDigestAfter: sha('7'),
      }],
    }))).toThrow(/image configuration/);
    expect(() => createPostgresObservationQualificationReceipt(receiptInput({
      executions: [execution(1, '1'), {
        ...execution(2, '2'), cleanup: {
          ...execution(2, '2').cleanup, volumeRemoved: false,
        },
      }],
    }))).toThrow(/cleanup/);
    expect(() => createPostgresObservationQualificationReceipt(receiptInput({
      candidates: [candidate, {
        ...candidate,
        observation: {
          ...candidate.observation,
          identity: { ...candidate.observation.identity!, constraints: hex('e') },
        },
      }],
    }))).toThrow(/not byte equal/);
    expect(() => createPostgresObservationQualificationReceipt(receiptInput({
      executions: [execution(1, '1'), {
        ...execution(2, '2'), stdout: {
          ...execution(2, '2').stdout, bytes: 262_145,
        },
      }],
    }))).toThrow(/byte limit/);
  });

  it('closes version, error, combined-cap, and sentinel evidence', () => {
    expect(() => parsePostgresObservationQualificationCandidate({
      ...candidate,
      observation: { ...candidate.observation, serverVersion: 'PostgreSQL 16.150' },
    })).toThrow(/version/);
    expect(() => parsePostgresObservationQualificationCandidate({
      ...candidate,
      observation: { ...candidate.observation, serverVersion: 'PostgreSQL 16.15\u0000bad' },
    })).toThrow(/version/);
    expect(() => parsePostgresObservationQualificationCandidate({
      ...candidate,
      observation: {
        ...candidate.observation, identity: null, legacyComparison: 'unavailable',
        errorCode: 'Invented:Reason',
      },
    })).toThrow(/error code/);
    expect(() => parsePostgresObservationQualificationCandidate({
      ...candidate,
      observation: {
        ...candidate.observation,
        identity: null,
        legacyComparison: 'unavailable',
        errorCode: 'UnqualifiedEnginePatch',
      },
    })).toThrow(/profile-selection/);

    const overflowCandidate = {
      ...candidate,
      observation: {
        ...candidate.observation,
        rowCounts: {
          ...candidate.observation.rowCounts,
          catalogConstraints: 65_535,
          combinedConstraints: 65_536,
        },
        streaming: {
          ...candidate.observation.streaming,
          catalogConstraints: {
            cap: 65_535,
            polled: 65_536,
            decoded: 65_535,
            retainedPeak: 65_535,
            overflow: true,
            terminal: 'overflow' as const,
          },
        },
        identity: null,
        legacyComparison: 'unavailable' as const,
        errorCode: 'LimitExceeded:RawConstraints' as const,
        failurePhase: 'catalog-constraints-stream' as const,
      },
    };
    const receipt = createPostgresObservationQualificationReceipt({
      candidates: [overflowCandidate, structuredClone(overflowCandidate)],
      executions: [execution(1, '1'), execution(2, '2')],
    });
    expect(receipt.replayStatus).toBe('fail');
    expect(() => verifyPostgresObservationQualificationReceipt(receipt)).not.toThrow();
    expect(() => parsePostgresObservationQualificationCandidate({
      ...overflowCandidate,
      observation: {
        ...overflowCandidate.observation,
        streaming: {
          ...overflowCandidate.observation.streaming,
          catalogConstraints: {
            ...overflowCandidate.observation.streaming.catalogConstraints,
            decoded: 65_536,
            retainedPeak: 65_536,
          },
        },
      },
    })).toThrow(/overflow sentinel/);
    expect(() => parsePostgresObservationQualificationCandidate({
      ...overflowCandidate,
      observation: {
        ...overflowCandidate.observation,
        errorCode: 'LimitExceeded:PhysicalAttributes',
      },
    })).toThrow(/exact error/);
    const relationNormalizationFailure = {
      ...candidate,
      observation: {
        ...candidate.observation,
        identity: null,
        legacyComparison: 'unavailable',
        errorCode: 'LimitExceeded:PhysicalAttributes',
        failurePhase: 'relation-normalization',
        rowCounts: {
          ...candidate.observation.rowCounts,
          notNullConstraints: 0,
          catalogConstraints: 0,
          combinedConstraints: 0,
        },
        streaming: {
          ...candidate.observation.streaming,
          catalogConstraints: {
            cap: 65_536, polled: 0, decoded: 0, retainedPeak: 0, overflow: false,
            terminal: 'not-started',
          },
        },
      },
    };
    expect(() => parsePostgresObservationQualificationCandidate(
      relationNormalizationFailure,
    )).not.toThrow();
    expect(() => parsePostgresObservationQualificationCandidate({
      ...relationNormalizationFailure,
      observation: {
        ...relationNormalizationFailure.observation,
        rowCounts: {
          ...relationNormalizationFailure.observation.rowCounts,
          notNullConstraints: 1,
          combinedConstraints: 1,
        },
        streaming: {
          ...relationNormalizationFailure.observation.streaming,
          catalogConstraints: {
            ...relationNormalizationFailure.observation.streaming.catalogConstraints,
            cap: 65_535,
          },
        },
      },
    })).toThrow(/claims derived constraints/);

    const decodeFailure = {
      ...candidate,
      observation: {
        ...candidate.observation,
        rowCounts: {
          relations: 0, attributes: 0, notNullConstraints: 0,
          catalogConstraints: 0, combinedConstraints: 0,
        },
        streaming: {
          relations: {
            cap: 4_096, polled: 1, decoded: 0, retainedPeak: 0, overflow: false,
            terminal: 'row-failure' as const,
          },
          attributes: {
            cap: 65_536, polled: 0, decoded: 0, retainedPeak: 0, overflow: false,
            terminal: 'not-started' as const,
          },
          catalogConstraints: {
            cap: 65_536, polled: 0, decoded: 0, retainedPeak: 0, overflow: false,
            terminal: 'not-started' as const,
          },
        },
        identity: null,
        legacyComparison: 'unavailable' as const,
        errorCode: 'CatalogDecode' as const,
        failurePhase: 'relations-stream' as const,
      },
    };
    expect(() => parsePostgresObservationQualificationCandidate(decodeFailure)).not.toThrow();

    const semanticRowFailure = {
      ...candidate,
      observation: {
        ...candidate.observation,
        rowCounts: {
          relations: 1, attributes: 0, notNullConstraints: 0,
          catalogConstraints: 0, combinedConstraints: 0,
        },
        streaming: {
          relations: candidate.observation.streaming.relations,
          attributes: {
            cap: 65_536, polled: 1, decoded: 0, retainedPeak: 0, overflow: false,
            terminal: 'row-failure' as const,
          },
          catalogConstraints: {
            cap: 65_536, polled: 0, decoded: 0, retainedPeak: 0, overflow: false,
            terminal: 'not-started' as const,
          },
        },
        identity: null,
        legacyComparison: 'unavailable' as const,
        errorCode: 'UnsupportedType' as const,
        failurePhase: 'attributes-stream' as const,
      },
    };
    expect(() => parsePostgresObservationQualificationCandidate(semanticRowFailure)).not.toThrow();
    expect(() => parsePostgresObservationQualificationCandidate({
      ...semanticRowFailure,
      observation: {
        ...semanticRowFailure.observation,
        rowCounts: {
          ...semanticRowFailure.observation.rowCounts,
          notNullConstraints: 1,
          combinedConstraints: 1,
        },
        streaming: {
          ...semanticRowFailure.observation.streaming,
          catalogConstraints: {
            ...semanticRowFailure.observation.streaming.catalogConstraints,
            cap: 65_535,
          },
        },
      },
    })).toThrow(/claims derived constraints/);
    expect(() => parsePostgresObservationQualificationCandidate({
      ...semanticRowFailure,
      observation: {
        ...semanticRowFailure.observation,
        failurePhase: 'relation-normalization',
      },
    })).toThrow(/failure phase/);
    expect(() => parsePostgresObservationQualificationCandidate({
      ...semanticRowFailure,
      observation: {
        ...semanticRowFailure.observation,
        errorCode: 'UnsupportedCollation',
      },
    })).toThrow(/exact error/);
    expect(() => parsePostgresObservationQualificationCandidate({
      ...overflowCandidate,
      observation: {
        ...overflowCandidate.observation,
        streaming: {
          ...overflowCandidate.observation.streaming,
          catalogConstraints: {
            ...overflowCandidate.observation.streaming.catalogConstraints,
            overflow: false,
            terminal: 'row-failure',
          },
        },
        errorCode: 'UnsupportedConstraint',
      },
    })).toThrow(/overflow evidence/);
  });

  it('keeps the executable qualification path fail closed until Rust evidence is complete', async () => {
    expect(POSTGRES_OBSERVATION_QUALIFICATION_RUNNER_STATUS).toEqual({
      available: false,
      reason: 'rust-evidence-contract-incomplete',
    });
    await expect(runPostgresObservationQualificationProbe()).rejects
      .toThrow('POSTGRES_OBSERVATION_QUALIFICATION_WITHHELD');
  });
});

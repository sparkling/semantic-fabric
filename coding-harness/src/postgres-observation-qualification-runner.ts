export const POSTGRES_OBSERVATION_MAX_OUTPUT_BYTES = 262_144;

export const POSTGRES_OBSERVATION_STREAM_IDS = Object.freeze([
  'legacy-tables',
  'legacy-earlier-collisions',
  'legacy-columns',
  'legacy-keys',
  'legacy-foreign-keys',
  'legacy-relation-statistics',
  'legacy-column-statistics',
  'rich-relations',
  'rich-attributes',
  'rich-catalog-constraints',
] as const);

export type PostgresObservationStreamId =
  (typeof POSTGRES_OBSERVATION_STREAM_IDS)[number];

export const POSTGRES_OBSERVATION_COMPLETED_PHASES = Object.freeze([
  'guard',
  'relations-stream',
  'attributes-stream',
  'relation-normalization',
  'not-null-derivation',
  'constraint-budget',
  'catalog-constraints-stream',
  'constraint-normalization',
  'legacy-comparison',
  'identity-build',
] as const);

export type PostgresObservationCompletedPhase =
  (typeof POSTGRES_OBSERVATION_COMPLETED_PHASES)[number];

export interface PostgresObservationStreamEvidence {
  cap: number;
  polled: number;
  decoded: number;
  retainedPeak: number;
  overflow: false;
  terminal: 'complete';
}

export interface PostgresObservationQualificationCandidate {
  provenance: {
    source: {
      commit: string;
      tree: string;
      cargoLockSha256: string;
      probeArtifactSha256: string;
      probeStdoutSha256: string;
    };
    inputs: {
      adrSha256: string;
      profileSha256: string;
      queriesSha256: string;
      testsSha256: string;
      fixtureSha256: string;
      runnerSha256: string;
      protocolSha256: string;
    };
    toolchain: {
      rustcVvSha256: string;
      cargoVersionSha256: string;
      targetTriple: 'x86_64-unknown-linux-gnu';
      builderImage: {
        repository: 'rust';
        manifestDigest: string;
        configDigest: string;
        platform: 'linux/amd64';
        inspectSha256: string;
      };
    };
    image: {
      repository: 'postgres';
      manifestDigest: string;
      configDigest: string;
      platform: 'linux/amd64';
      inspectSha256: string;
    };
  };
  preflight: {
    networkMode: 'none';
    fixedDatabase: true;
    unixSocket: true;
    comparisonRole: {
      superuser: false;
      databaseOwner: false;
      inherit: false;
      bypassRls: false;
      canSetRole: false;
      canDdl: false;
      privilegesExact: true;
    };
    ownerIdentityEqual: true;
    legacyConstraintVisibilityDiffers: true;
  };
  observation: {
    serverVersion: string;
    serverVersionNum: 160009 | 160015;
    guard: 'pass';
    rowCounts: {
      relations: number;
      attributes: number;
      notNullConstraints: number;
      catalogConstraints: number;
      combinedConstraints: number;
    };
    streaming: {
      relations: PostgresObservationStreamEvidence;
      attributes: PostgresObservationStreamEvidence;
      catalogConstraints: PostgresObservationStreamEvidence;
    };
    identity: { structural: string; types: string; constraints: string };
    legacyCoordinateComparison: 'equal';
    errorCode: null;
    failurePhase: null;
  };
  queryAccounting: {
    prequalificationGuard: 1;
    richCapture: 4;
    guardQueriesObserved: 2;
    streamQueriesObserved: 10;
    totalQueriesObserved: 12;
    streams: Array<{
      id: PostgresObservationStreamId;
      evidence: PostgresObservationStreamEvidence;
    }>;
  };
  lifecycle: {
    savepoint: 'released';
    recovery: 'not-needed';
    commit: 'complete';
    phasesCompleted: PostgresObservationCompletedPhase[];
  };
}

export type PostgresObservationQualificationProbeOutput = Pick<
  PostgresObservationQualificationCandidate,
  'preflight' | 'observation' | 'queryAccounting' | 'lifecycle'
>;

export interface PostgresObservationQualificationExecutionInput {
  slot: 1 | 2;
  containerIdentitySha256: string;
  volumeIdentitySha256: string;
  imageConfigDigestBefore: string;
  imageConfigDigestAfter: string;
  stdout: { bytes: number; sha256: string; truncated: false };
  stderr: { bytes: number; sha256: string; truncated: false };
  exitCode: 0;
  timedOut: false;
  outputLimitExceeded: false;
  cleanup: {
    containerRemoved: true;
    volumeRemoved: true;
    labelledContainersRemaining: 0;
    labelledVolumesRemaining: 0;
    verificationSha256: string;
  };
}

export interface PostgresObservationQualificationExecution
  extends PostgresObservationQualificationExecutionInput {
  candidateSha256: string;
}

export interface PostgresObservationQualificationReceipt {
  receiptKind: 'postgresql-public-observation-qualification-v1';
  evidenceClass: 'test-only-non-runtime';
  authority: 'observation-profile-only';
  observationProfileQualification: 'pass';
  runtimeAdmissionStatus: 'withheld-independent-gates';
  productionAdmission: false;
  verifiedLease: false;
  reload: false;
  directMapping: false;
  candidates: [
    PostgresObservationQualificationCandidate,
    PostgresObservationQualificationCandidate,
  ];
  replay: {
    requiredRuns: 2;
    canonicalCandidatesByteEqual: true;
    freshContainers: true;
    freshVolumes: true;
    cleanupVerified: true;
    executions: [
      PostgresObservationQualificationExecution,
      PostgresObservationQualificationExecution,
    ];
  };
  replayStatus: 'pass';
  receiptSha256: string;
}

export const POSTGRES_OBSERVATION_QUALIFICATION_RUNNER_STATUS = Object.freeze({
  available: true as const,
  authority: 'observation-profile-only' as const,
  runtimeAdmissionStatus: 'withheld-independent-gates' as const,
});

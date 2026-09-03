export const POSTGRES_OBSERVATION_MAX_OUTPUT_BYTES = 262_144;

export const POSTGRES_OBSERVATION_ERROR_CODES = Object.freeze([
  'ProfileNotImplemented',
  'UnqualifiedEnginePatch',
  'GuardUnsupported:ServerEncoding',
  'GuardUnsupported:IdentifierLength',
  'GuardUnsupported:IndexKeyLimit',
  'GuardUnsupported:IntegerDatetimes',
  'GuardUnsupported:ReplicationRole',
  'GuardUnsupported:SearchPath',
  'GuardUnsupported:PublicNamespace',
  'GuardUnsupported:CurrentDatabase',
  'LegacyCoordinateMismatch',
  'CatalogQuery',
  'CatalogDecode',
  'LimitExceeded:RichRelations',
  'LimitExceeded:PhysicalAttributes',
  'LimitExceeded:LiveColumns',
  'LimitExceeded:RawConstraints',
  'LimitExceeded:KeyMembers',
  'LimitExceeded:Facets',
  'LimitExceeded:TextBytes',
  'LimitExceeded:CanonicalBody',
  'UnsupportedRelation',
  'UnsupportedType',
  'UnsupportedCollation',
  'UnsupportedConstraint',
  'IdentityRejected',
] as const);

export type PostgresObservationErrorCode =
  (typeof POSTGRES_OBSERVATION_ERROR_CODES)[number];

export const POSTGRES_OBSERVATION_FAILURE_PHASES = Object.freeze([
  'guard',
  'relations-stream',
  'attributes-stream',
  'relation-normalization',
  'not-null-derivation',
  'constraint-budget',
  'catalog-constraints-stream',
  'constraint-normalization',
  'identity-build',
  'legacy-comparison',
] as const);

export type PostgresObservationFailurePhase =
  (typeof POSTGRES_OBSERVATION_FAILURE_PHASES)[number];

export interface PostgresObservationStreamEvidence {
  cap: number;
  polled: number;
  decoded: number;
  retainedPeak: number;
  overflow: boolean;
  terminal: 'complete' | 'overflow' | 'row-failure' | 'query-failure' | 'not-started';
}

export interface PostgresObservationQualificationCandidate {
  provenance: {
    source: {
      commit: string;
      tree: string;
      cargoLockSha256: string;
      probeArtifactSha256: string;
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
    guard: 'pass' | 'fail';
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
    identity: { structural: string; types: string; constraints: string } | null;
    legacyComparison: 'equal' | 'unavailable';
    errorCode: PostgresObservationErrorCode | null;
    failurePhase: PostgresObservationFailurePhase | null;
  };
}

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
  authority: 'development-only-no-promotion';
  productionAdmission: false;
  verifiedLease: false;
  reload: false;
  directMapping: false;
  runtimeCoverage: {
    savepointRecovery: 'not-implemented';
    committedUnavailable: 'not-implemented';
    transactionCommitFaultMatrix: 'not-exercised';
    runtimeCarrier: 'not-integrated';
  };
  qualificationStatus: 'withheld-runtime-gaps';
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
  replayStatus: 'pass' | 'fail';
  receiptSha256: string;
}

type ObservationState = Pick<PostgresObservationQualificationCandidate['observation'],
  | 'guard' | 'rowCounts' | 'streaming' | 'identity' | 'legacyComparison' | 'errorCode'
  | 'failurePhase'>;

const STREAM_PHASES = [
  'relations-stream', 'attributes-stream', 'catalog-constraints-stream',
] as const;
const STREAM_OVERFLOW_CODES = [
  'LimitExceeded:RichRelations',
  'LimitExceeded:PhysicalAttributes',
  'LimitExceeded:RawConstraints',
] as const;
const STREAM_ROW_FAILURE_CODES: readonly (readonly PostgresObservationErrorCode[])[] = [
  ['CatalogDecode', 'LimitExceeded:TextBytes', 'IdentityRejected'],
  [
    'CatalogDecode', 'LimitExceeded:PhysicalAttributes', 'LimitExceeded:TextBytes',
    'UnsupportedType', 'IdentityRejected',
  ],
  ['CatalogDecode', 'LimitExceeded:KeyMembers', 'UnsupportedConstraint'],
];
const NON_STREAM_FAILURE_CODES: Readonly<Partial<Record<
  PostgresObservationFailurePhase, readonly PostgresObservationErrorCode[]
>>> = Object.freeze({
  'relation-normalization': [
    'LimitExceeded:RichRelations', 'LimitExceeded:PhysicalAttributes',
    'LimitExceeded:LiveColumns', 'LimitExceeded:TextBytes', 'LimitExceeded:CanonicalBody',
    'UnsupportedRelation', 'UnsupportedType', 'UnsupportedCollation', 'IdentityRejected',
  ],
  'not-null-derivation': ['LimitExceeded:RawConstraints'],
  'constraint-budget': ['LimitExceeded:RawConstraints'],
  'constraint-normalization': ['LimitExceeded:RawConstraints', 'UnsupportedConstraint'],
  'identity-build': [
    'LimitExceeded:RichRelations', 'LimitExceeded:LiveColumns',
    'LimitExceeded:RawConstraints', 'LimitExceeded:KeyMembers', 'LimitExceeded:Facets',
    'LimitExceeded:TextBytes', 'LimitExceeded:CanonicalBody', 'IdentityRejected',
  ],
  'legacy-comparison': ['LegacyCoordinateMismatch'],
});

export function validatePostgresObservationState(value: ObservationState): void {
  const streams = [
    value.streaming.relations,
    value.streaming.attributes,
    value.streaming.catalogConstraints,
  ];
  const complete = streams.every((stream) => stream.terminal === 'complete');
  const normalizationCompleted = value.guard === 'pass'
    && streams[0].terminal === 'complete' && streams[1].terminal === 'complete'
    && value.failurePhase !== 'relation-normalization';
  if (normalizationCompleted
    && (value.rowCounts.notNullConstraints > value.rowCounts.attributes
      || value.rowCounts.relations > value.rowCounts.attributes)) {
    throw new TypeError('post-normalization row counts are inconsistent');
  }
  const success = value.guard === 'pass' && complete
    && value.rowCounts.combinedConstraints <= 65_536 && value.identity !== null
    && value.legacyComparison === 'equal' && value.errorCode === null
    && value.failurePhase === null;
  if (success) return;
  if (value.errorCode === 'ProfileNotImplemented'
    || value.errorCode === 'UnqualifiedEnginePatch') {
    throw new TypeError('exact engine receipt cannot carry a profile-selection error');
  }
  if (value.identity !== null || value.legacyComparison !== 'unavailable'
    || value.errorCode === null || value.failurePhase === null) {
    throw new TypeError('failed qualification evidence has inconsistent identity state');
  }
  if (value.guard === 'fail') {
    const guardError = value.errorCode.startsWith('GuardUnsupported:')
      || ['CatalogQuery', 'CatalogDecode', 'LimitExceeded:TextBytes', 'IdentityRejected']
        .includes(value.errorCode);
    if (value.failurePhase !== 'guard' || !guardError
      || streams.some((stream) => stream.terminal !== 'not-started')
      || Object.values(value.rowCounts).some((count) => count !== 0)) {
      throw new TypeError('guard failure evidence is inconsistent');
    }
    return;
  }
  if (value.errorCode.startsWith('GuardUnsupported:')) {
    throw new TypeError('guard error code does not match a passing guard');
  }
  let stopped = false;
  let failure: { index: number; terminal: PostgresObservationStreamEvidence['terminal'] } | null
    = null;
  for (const [index, stream] of streams.entries()) {
    if (stopped && stream.terminal !== 'not-started') {
      throw new TypeError('stream terminal sequence is inconsistent');
    }
    if (stream.terminal === 'not-started') {
      stopped = true;
    } else if (stream.terminal !== 'complete') {
      if (failure !== null) throw new TypeError('multiple stream failures are invalid');
      failure = { index, terminal: stream.terminal };
      stopped = true;
    }
  }
  if (failure === null) {
    validateNonStreamFailure(value, complete);
    return;
  }
  if (value.failurePhase !== STREAM_PHASES[failure.index]) {
    throw new TypeError('stream failure phase does not match its execution position');
  }
  if (failure.index < 2 && !derivedConstraintCountsAreZero(value)) {
    throw new TypeError('pre-normalization stream failure claims derived constraints');
  }
  const matchesError = failure.terminal === 'query-failure'
    ? value.errorCode === 'CatalogQuery'
    : failure.terminal === 'overflow'
      ? value.errorCode === STREAM_OVERFLOW_CODES[failure.index]
      : failure.terminal === 'row-failure'
        && STREAM_ROW_FAILURE_CODES[failure.index].includes(value.errorCode);
  if (!matchesError) {
    throw new TypeError('stream failure does not match its exact error code');
  }
}

function validateNonStreamFailure(value: ObservationState, complete: boolean): void {
  const beforeConstraintStream = value.streaming.relations.terminal === 'complete'
    && value.streaming.attributes.terminal === 'complete'
    && value.streaming.catalogConstraints.terminal === 'not-started';
  const phasePositionMatches = beforeConstraintStream
    ? ['relation-normalization', 'not-null-derivation', 'constraint-budget']
      .includes(value.failurePhase as string)
    : complete && ['constraint-normalization', 'identity-build', 'legacy-comparison']
      .includes(value.failurePhase as string);
  const allowed = NON_STREAM_FAILURE_CODES[value.failurePhase!];
  if (!phasePositionMatches || allowed === undefined || !allowed.includes(value.errorCode!)) {
    throw new TypeError('non-stream failure phase does not match execution state or error code');
  }
  if (value.failurePhase === 'relation-normalization'
    && !derivedConstraintCountsAreZero(value)) {
    throw new TypeError('relation-normalization failure claims derived constraints');
  }
}

function derivedConstraintCountsAreZero(value: ObservationState): boolean {
  return value.rowCounts.notNullConstraints === 0
    && value.rowCounts.catalogConstraints === 0
    && value.rowCounts.combinedConstraints === 0;
}

/**
 * The Rust probe does not yet emit the closed streaming, role-preflight, and
 * committed-unavailability evidence required by ADR-0051. Keep execution
 * disabled instead of manufacturing a qualification receipt from partial data.
 */
export const POSTGRES_OBSERVATION_QUALIFICATION_RUNNER_STATUS = Object.freeze({
  available: false as const,
  reason: 'rust-evidence-contract-incomplete' as const,
});

export async function runPostgresObservationQualificationProbe(): Promise<never> {
  throw new Error('POSTGRES_OBSERVATION_QUALIFICATION_WITHHELD');
}

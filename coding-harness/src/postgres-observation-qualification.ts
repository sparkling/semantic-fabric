import { canonical, hash } from '@metaharness/harness';
import {
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  asInteger,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import type {
  PostgresObservationErrorCode,
  PostgresObservationFailurePhase,
  PostgresObservationQualificationCandidate,
  PostgresObservationQualificationExecution,
  PostgresObservationQualificationReceipt,
  PostgresObservationStreamEvidence,
} from './postgres-observation-qualification-runner.js';
import {
  POSTGRES_OBSERVATION_ERROR_CODES,
  POSTGRES_OBSERVATION_FAILURE_PHASES,
  POSTGRES_OBSERVATION_MAX_OUTPUT_BYTES,
  validatePostgresObservationState,
} from './postgres-observation-qualification-runner.js';
export type {
  PostgresObservationErrorCode,
  PostgresObservationFailurePhase,
  PostgresObservationQualificationCandidate,
  PostgresObservationQualificationExecution,
  PostgresObservationQualificationReceipt,
  PostgresObservationStreamEvidence,
} from './postgres-observation-qualification-runner.js';
const MAX_RELATIONS = 4_096;
const MAX_ATTRIBUTES = 65_536;
const MAX_CONSTRAINTS = 65_536;
const GIT_OBJECT_PATTERN = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/;
const ERROR_CODES = new Set<string>(POSTGRES_OBSERVATION_ERROR_CODES);

const RECEIPT_KEYS = [
  'receiptKind', 'evidenceClass', 'authority', 'productionAdmission', 'verifiedLease',
  'reload', 'directMapping', 'runtimeCoverage', 'qualificationStatus', 'candidates',
  'replay', 'replayStatus', 'receiptSha256',
] as const;

export function parsePostgresObservationQualificationCandidate(
  value: unknown,
): PostgresObservationQualificationCandidate {
  const input = closed(value, ['provenance', 'preflight', 'observation'], 'candidate');
  const candidate = {
    provenance: parseProvenance(input.provenance),
    preflight: parsePreflight(input.preflight),
    observation: parseObservation(input.observation),
  };
  return deepFreeze(candidate);
}
export function createPostgresObservationQualificationReceipt(
  value: unknown,
): PostgresObservationQualificationReceipt {
  const input = closed(value, ['candidates', 'executions'], 'qualification receipt input');
  const candidates = parseCandidatePair(input.candidates);
  const executions = parseExecutionPair(input.executions, candidates, false);
  const replayStatus = candidates[0].observation.identity === null ? 'fail' : 'pass';
  const body = receiptBody(candidates, executions, replayStatus);
  return deepFreeze({ ...body, receiptSha256: hash(canonical(body)) });
}
export function verifyPostgresObservationQualificationReceipt(value: unknown): void {
  const receipt = closed(value, RECEIPT_KEYS, 'qualification receipt');
  exact(receipt.receiptKind, ['postgresql-public-observation-qualification-v1'] as const, 'kind');
  exact(receipt.evidenceClass, ['test-only-non-runtime'] as const, 'evidence class');
  exact(receipt.authority, ['development-only-no-promotion'] as const, 'authority');
  exact(receipt.productionAdmission, [false] as const, 'production admission');
  exact(receipt.verifiedLease, [false] as const, 'verified lease');
  exact(receipt.reload, [false] as const, 'reload');
  exact(receipt.directMapping, [false] as const, 'Direct Mapping');
  parseRuntimeCoverage(receipt.runtimeCoverage);
  exact(receipt.qualificationStatus, ['withheld-runtime-gaps'] as const, 'qualification status');
  const candidates = parseCandidatePair(receipt.candidates);
  const replay = closed(receipt.replay, [
    'requiredRuns', 'canonicalCandidatesByteEqual', 'freshContainers', 'freshVolumes',
    'cleanupVerified', 'executions',
  ], 'replay');
  exactReplayFlags(replay);
  const executions = parseExecutionPair(replay.executions, candidates, true);
  const replayStatus = exact(receipt.replayStatus, ['pass', 'fail'] as const, 'replay status');
  if (replayStatus !== (candidates[0].observation.identity === null ? 'fail' : 'pass')) {
    throw new TypeError('qualification replay status is inconsistent');
  }
  const body = receiptBody(candidates, executions, replayStatus);
  if (digest(receipt.receiptSha256, 'receipt digest') !== hash(canonical(body))) {
    throw new Error('qualification receipt digest mismatch');
  }
}
function receiptBody(
  candidates: [PostgresObservationQualificationCandidate, PostgresObservationQualificationCandidate],
  executions: [PostgresObservationQualificationExecution, PostgresObservationQualificationExecution],
  replayStatus: 'pass' | 'fail',
) {
  return {
    receiptKind: 'postgresql-public-observation-qualification-v1' as const,
    evidenceClass: 'test-only-non-runtime' as const,
    authority: 'development-only-no-promotion' as const,
    productionAdmission: false as const,
    verifiedLease: false as const,
    reload: false as const,
    directMapping: false as const,
    runtimeCoverage: {
      savepointRecovery: 'not-implemented' as const,
      committedUnavailable: 'not-implemented' as const,
      transactionCommitFaultMatrix: 'not-exercised' as const,
      runtimeCarrier: 'not-integrated' as const,
    },
    qualificationStatus: 'withheld-runtime-gaps' as const,
    candidates,
    replay: {
      requiredRuns: 2 as const,
      canonicalCandidatesByteEqual: true as const,
      freshContainers: true as const,
      freshVolumes: true as const,
      cleanupVerified: true as const,
      executions,
    },
    replayStatus,
  };
}
function parseProvenance(value: unknown): PostgresObservationQualificationCandidate['provenance'] {
  const input = closed(value, ['source', 'inputs', 'toolchain', 'image'], 'provenance');
  const sourceInput = closed(input.source, [
    'commit', 'tree', 'cargoLockSha256', 'probeArtifactSha256',
  ], 'source provenance');
  const commit = gitObject(sourceInput.commit, 'source commit');
  const tree = gitObject(sourceInput.tree, 'source tree');
  if (commit.length !== tree.length) throw new TypeError('source Git object formats differ');
  const inputs = closed(input.inputs, [
    'adrSha256', 'profileSha256', 'queriesSha256', 'testsSha256', 'fixtureSha256',
    'runnerSha256', 'protocolSha256',
  ], 'input provenance');
  const toolchain = closed(input.toolchain, [
    'rustcVvSha256', 'cargoVersionSha256', 'targetTriple',
  ], 'toolchain provenance');
  const image = closed(input.image, [
    'repository', 'manifestDigest', 'configDigest', 'platform', 'inspectSha256',
  ], 'image provenance');
  return {
    source: {
      commit,
      tree,
      cargoLockSha256: digest(sourceInput.cargoLockSha256, 'Cargo lock digest'),
      probeArtifactSha256: digest(sourceInput.probeArtifactSha256, 'probe artifact digest'),
    },
    inputs: Object.fromEntries(Object.entries(inputs).map(([key, raw]) => [
      key, digest(raw, `${key} digest`),
    ])) as PostgresObservationQualificationCandidate['provenance']['inputs'],
    toolchain: {
      rustcVvSha256: digest(toolchain.rustcVvSha256, 'rustc digest'),
      cargoVersionSha256: digest(toolchain.cargoVersionSha256, 'Cargo version digest'),
      targetTriple: exact(
        toolchain.targetTriple, ['x86_64-unknown-linux-gnu'] as const, 'target triple',
      ),
    },
    image: {
      repository: exact(image.repository, ['postgres'] as const, 'image repository'),
      manifestDigest: prefixedDigest(image.manifestDigest, 'image manifest digest'),
      configDigest: prefixedDigest(image.configDigest, 'image config digest'),
      platform: exact(image.platform, ['linux/amd64'] as const, 'image platform'),
      inspectSha256: digest(image.inspectSha256, 'image inspection digest'),
    },
  };
}
function parsePreflight(value: unknown): PostgresObservationQualificationCandidate['preflight'] {
  const input = closed(value, [
    'networkMode', 'fixedDatabase', 'unixSocket', 'comparisonRole',
    'ownerIdentityEqual', 'legacyConstraintVisibilityDiffers',
  ], 'preflight');
  const role = closed(input.comparisonRole, [
    'superuser', 'databaseOwner', 'inherit', 'bypassRls', 'canSetRole', 'canDdl',
    'privilegesExact',
  ], 'comparison role');
  return {
    networkMode: exact(input.networkMode, ['none'] as const, 'network mode'),
    fixedDatabase: exact(input.fixedDatabase, [true] as const, 'fixed database'),
    unixSocket: exact(input.unixSocket, [true] as const, 'Unix socket'),
    comparisonRole: {
      superuser: exact(role.superuser, [false] as const, 'superuser'),
      databaseOwner: exact(role.databaseOwner, [false] as const, 'database owner'),
      inherit: exact(role.inherit, [false] as const, 'role inheritance'),
      bypassRls: exact(role.bypassRls, [false] as const, 'BYPASSRLS'),
      canSetRole: exact(role.canSetRole, [false] as const, 'SET ROLE'),
      canDdl: exact(role.canDdl, [false] as const, 'DDL'),
      privilegesExact: exact(role.privilegesExact, [true] as const, 'privileges'),
    },
    ownerIdentityEqual: exact(input.ownerIdentityEqual, [true] as const, 'owner identity'),
    legacyConstraintVisibilityDiffers: exact(
      input.legacyConstraintVisibilityDiffers, [true] as const, 'legacy visibility control',
    ),
  };
}
function parseObservation(value: unknown): PostgresObservationQualificationCandidate['observation'] {
  const input = closed(value, [
    'serverVersion', 'serverVersionNum', 'guard', 'rowCounts', 'streaming', 'identity',
    'legacyComparison', 'errorCode', 'failurePhase',
  ], 'observation');
  const serverVersionNum = exact(
    input.serverVersionNum, [160009, 160015] as const, 'server version number',
  );
  const patch = serverVersionNum === 160009 ? '16.9' : '16.15';
  if (!validServerVersion(input.serverVersion, patch)) {
    throw new TypeError('server version does not match the exact qualified patch');
  }
  const counts = closed(input.rowCounts, [
    'relations', 'attributes', 'notNullConstraints', 'catalogConstraints',
    'combinedConstraints',
  ], 'row counts');
  const rowCounts = {
    relations: asInteger(counts.relations, 'relation row count'),
    attributes: asInteger(counts.attributes, 'attribute row count'),
    notNullConstraints: asInteger(counts.notNullConstraints, 'NOT NULL count'),
    catalogConstraints: asInteger(counts.catalogConstraints, 'catalogue constraint count'),
    combinedConstraints: asInteger(counts.combinedConstraints, 'combined constraint count'),
  };
  if (rowCounts.notNullConstraints > MAX_CONSTRAINTS
    || rowCounts.notNullConstraints + rowCounts.catalogConstraints
      !== rowCounts.combinedConstraints) {
    throw new TypeError('combined constraint count is inconsistent');
  }
  const streams = closed(
    input.streaming, ['relations', 'attributes', 'catalogConstraints'], 'streaming evidence',
  );
  const streaming = {
    relations: streamEvidence(streams.relations, MAX_RELATIONS, 'relations'),
    attributes: streamEvidence(streams.attributes, MAX_ATTRIBUTES, 'attributes'),
    catalogConstraints: streamEvidence(
      streams.catalogConstraints, MAX_CONSTRAINTS - rowCounts.notNullConstraints,
      'catalog constraints',
    ),
  };
  if (rowCounts.relations !== streaming.relations.decoded
    || rowCounts.attributes !== streaming.attributes.decoded
    || rowCounts.catalogConstraints !== streaming.catalogConstraints.decoded) {
    throw new TypeError('row counts do not match streaming decode evidence');
  }
  const guard = exact(input.guard, ['pass', 'fail'] as const, 'guard result');
  const identity = input.identity === null ? null : parseIdentity(input.identity);
  const legacyComparison = exact(
    input.legacyComparison, ['equal', 'unavailable'] as const, 'legacy comparison',
  );
  const errorCode = parseErrorCode(input.errorCode);
  const failurePhase = input.failurePhase === null ? null : exact(
    input.failurePhase, POSTGRES_OBSERVATION_FAILURE_PHASES, 'failure phase',
  );
  validatePostgresObservationState({
    guard, rowCounts, streaming, identity, legacyComparison, errorCode, failurePhase,
  });
  return {
    serverVersion: input.serverVersion as string,
    serverVersionNum,
    guard,
    rowCounts,
    streaming,
    identity,
    legacyComparison,
    errorCode,
    failurePhase,
  };
}
function parseExecutionPair(
  value: unknown,
  candidates: [PostgresObservationQualificationCandidate, PostgresObservationQualificationCandidate],
  includesCandidateDigest: boolean,
): [PostgresObservationQualificationExecution, PostgresObservationQualificationExecution] {
  const values = asDenseArray(value, 'qualification executions');
  if (values.length !== 2) throw new TypeError('qualification requires exactly two executions');
  const parsed = values.map((entry, index) => parseExecution(
    entry, (index + 1) as 1 | 2, hash(canonical(candidates[index])),
    candidates[index].provenance.image.configDigest,
    includesCandidateDigest,
  )) as [PostgresObservationQualificationExecution, PostgresObservationQualificationExecution];
  if (parsed[0].containerIdentitySha256 === parsed[1].containerIdentitySha256) {
    throw new TypeError('qualification requires distinct containers');
  }
  if (parsed[0].volumeIdentitySha256 === parsed[1].volumeIdentitySha256) {
    throw new TypeError('qualification requires distinct volumes');
  }
  return parsed;
}
function parseCandidatePair(value: unknown): [
  PostgresObservationQualificationCandidate,
  PostgresObservationQualificationCandidate,
] {
  const values = asDenseArray(value, 'qualification candidates');
  if (values.length !== 2) throw new TypeError('qualification requires exactly two candidates');
  const candidates = values.map(parsePostgresObservationQualificationCandidate) as [
    PostgresObservationQualificationCandidate, PostgresObservationQualificationCandidate,
  ];
  if (canonical(candidates[0]) !== canonical(candidates[1])) {
    throw new TypeError('qualification replay candidates are not byte equal');
  }
  return candidates;
}
function parseExecution(
  value: unknown,
  expectedSlot: 1 | 2,
  candidateSha256: string,
  imageConfigDigest: string,
  includesCandidateDigest: boolean,
): PostgresObservationQualificationExecution {
  const keys = [
    'slot', ...(includesCandidateDigest ? ['candidateSha256'] : []),
    'containerIdentitySha256', 'volumeIdentitySha256', 'imageConfigDigestBefore',
    'imageConfigDigestAfter', 'stdout', 'stderr', 'exitCode', 'timedOut',
    'outputLimitExceeded', 'cleanup',
  ];
  const input = closed(value, keys, `execution ${expectedSlot}`);
  exact(input.slot, [expectedSlot] as const, `execution ${expectedSlot} slot`);
  if (includesCandidateDigest
    && digest(input.candidateSha256, 'candidate digest') !== candidateSha256) {
    throw new TypeError('execution candidate digest is inconsistent');
  }
  const before = prefixedDigest(input.imageConfigDigestBefore, 'image config before');
  const after = prefixedDigest(input.imageConfigDigestAfter, 'image config after');
  if (before !== imageConfigDigest || after !== imageConfigDigest) {
    throw new TypeError('execution image configuration drifted');
  }
  return {
    slot: expectedSlot,
    candidateSha256,
    containerIdentitySha256: digest(input.containerIdentitySha256, 'container identity'),
    volumeIdentitySha256: digest(input.volumeIdentitySha256, 'volume identity'),
    imageConfigDigestBefore: before,
    imageConfigDigestAfter: after,
    stdout: parseOutput(input.stdout, 'stdout'),
    stderr: parseOutput(input.stderr, 'stderr'),
    exitCode: exact(input.exitCode, [0] as const, 'exit code'),
    timedOut: exact(input.timedOut, [false] as const, 'timeout state'),
    outputLimitExceeded: exact(input.outputLimitExceeded, [false] as const, 'output limit'),
    cleanup: parseCleanup(input.cleanup),
  };
}
function streamEvidence(
  value: unknown,
  expectedCap: number,
  label: string,
): PostgresObservationStreamEvidence {
  const input = closed(value, [
    'cap', 'polled', 'decoded', 'retainedPeak', 'overflow', 'terminal',
  ], `${label} streaming evidence`);
  const cap = asInteger(input.cap, `${label} cap`);
  const polled = asInteger(input.polled, `${label} polled rows`);
  const decoded = asInteger(input.decoded, `${label} decoded rows`);
  const retainedPeak = asInteger(input.retainedPeak, `${label} retained peak`);
  const terminal = exact(input.terminal, [
    'complete', 'overflow', 'row-failure', 'query-failure', 'not-started',
  ] as const, `${label} terminal state`);
  if (cap !== expectedCap) throw new TypeError(`${label} cap is inconsistent`);
  if (typeof input.overflow !== 'boolean') throw new TypeError(`${label} overflow must be Boolean`);
  if (polled > cap + 1 || decoded > cap || retainedPeak !== decoded
    || input.overflow !== (terminal === 'overflow')) {
    throw new TypeError(`${label} decoded its overflow sentinel or has inconsistent retention`);
  }
  const validTerminal = terminal === 'complete' ? polled === decoded
    : terminal === 'overflow' ? polled === cap + 1 && decoded === cap
      : terminal === 'row-failure' ? polled === decoded + 1 && decoded < cap
        : terminal === 'query-failure' ? polled === decoded
          : polled === 0 && decoded === 0;
  if (!validTerminal) {
    throw new TypeError(`${label} overflow evidence is inconsistent`);
  }
  return { cap, polled, decoded, retainedPeak, overflow: input.overflow, terminal };
}
function parseOutput(value: unknown, label: string) {
  const input = closed(value, ['bytes', 'sha256', 'truncated'], `${label} evidence`);
  const bytes = asInteger(input.bytes, `${label} byte count`);
  if (bytes > POSTGRES_OBSERVATION_MAX_OUTPUT_BYTES) {
    throw new TypeError(`${label} evidence exceeds the protocol byte limit`);
  }
  return {
    bytes,
    sha256: digest(input.sha256, `${label} digest`),
    truncated: exact(input.truncated, [false] as const, `${label} truncation`),
  };
}
function parseCleanup(value: unknown) {
  const input = closed(value, [
    'containerRemoved', 'volumeRemoved', 'labelledContainersRemaining',
    'labelledVolumesRemaining', 'verificationSha256',
  ], 'cleanup evidence');
  try {
    return {
      containerRemoved: exact(input.containerRemoved, [true] as const, 'container cleanup'),
      volumeRemoved: exact(input.volumeRemoved, [true] as const, 'volume cleanup'),
      labelledContainersRemaining: exact(
        input.labelledContainersRemaining, [0] as const, 'remaining containers',
      ),
      labelledVolumesRemaining: exact(
        input.labelledVolumesRemaining, [0] as const, 'remaining volumes',
      ),
      verificationSha256: digest(input.verificationSha256, 'cleanup verification digest'),
    };
  } catch {
    throw new TypeError('qualification cleanup is incomplete');
  }
}
function parseRuntimeCoverage(value: unknown): void {
  const input = closed(value, [
    'savepointRecovery', 'committedUnavailable', 'transactionCommitFaultMatrix',
    'runtimeCarrier',
  ], 'runtime coverage');
  exact(input.savepointRecovery, ['not-implemented'] as const, 'savepoint recovery');
  exact(input.committedUnavailable, ['not-implemented'] as const, 'committed unavailability');
  exact(input.transactionCommitFaultMatrix, ['not-exercised'] as const, 'commit fault matrix');
  exact(input.runtimeCarrier, ['not-integrated'] as const, 'runtime carrier');
}
function exactReplayFlags(value: Record<string, unknown>): void {
  exact(value.requiredRuns, [2] as const, 'required replay count');
  exact(value.canonicalCandidatesByteEqual, [true] as const, 'candidate replay equality');
  exact(value.freshContainers, [true] as const, 'fresh containers');
  exact(value.freshVolumes, [true] as const, 'fresh volumes');
  exact(value.cleanupVerified, [true] as const, 'verified cleanup');
}
function parseIdentity(value: unknown) {
  const input = closed(value, ['structural', 'types', 'constraints'], 'identity');
  return {
    structural: digest(input.structural, 'structural identity'),
    types: digest(input.types, 'type identity'),
    constraints: digest(input.constraints, 'constraint identity'),
  };
}
function validServerVersion(value: unknown, patch: '16.9' | '16.15'): value is string {
  if (typeof value !== 'string' || Buffer.byteLength(value, 'utf8') > 256) return false;
  if (value !== `PostgreSQL ${patch}` && !value.startsWith(`PostgreSQL ${patch} `)) return false;
  return [...value].every((character) => {
    const code = character.charCodeAt(0);
    return code >= 0x20 && code <= 0x7e;
  });
}
function parseErrorCode(value: unknown): PostgresObservationErrorCode | null {
  if (value === null) return null;
  if (typeof value !== 'string' || !ERROR_CODES.has(value as never)) {
    throw new TypeError('qualification error code is invalid');
  }
  return value as PostgresObservationErrorCode;
}

function closed(value: unknown, keys: readonly string[], label: string): Record<string, unknown> {
  const input = asClosedRecord(value, label);
  assertExactKeys(input, keys, label);
  return input;
}

function exact<const T extends readonly (string | number | boolean)[]>(
  value: unknown,
  allowed: T,
  label: string,
): T[number] {
  if (!allowed.includes(value as never)) throw new TypeError(`${label} is invalid`);
  return value as T[number];
}

function digest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

function prefixedDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !value.startsWith('sha256:')) {
    throw new TypeError(`${label} is invalid`);
  }
  digest(value.slice(7), label);
  return value;
}

function gitObject(value: unknown, label: string): string {
  if (typeof value !== 'string' || !GIT_OBJECT_PATTERN.test(value)) {
    throw new TypeError(`${label} must be a full lowercase Git object ID`);
  }
  return value;
}

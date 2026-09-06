import { canonical, hash } from '@metaharness/harness';
import {
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  asInteger,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  parsePostgresObservationQualificationCandidate,
} from './postgres-observation-qualification-candidate.js';
import type {
  PostgresObservationQualificationCandidate,
  PostgresObservationQualificationExecution,
  PostgresObservationQualificationReceipt,
} from './postgres-observation-qualification-runner.js';
import {
  POSTGRES_OBSERVATION_MAX_OUTPUT_BYTES,
} from './postgres-observation-qualification-runner.js';
import {
  POSTGRES_QUALIFICATION_IMAGE_EXPECTATIONS,
  expectedQualificationImageIdentity,
} from './postgres-observation-qualification-protocol.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export {
  parsePostgresObservationQualificationCandidate,
  parsePostgresObservationQualificationCandidateJson,
  parsePostgresObservationQualificationProbeOutput,
} from './postgres-observation-qualification-candidate.js';
export type {
  PostgresObservationCompletedPhase,
  PostgresObservationQualificationCandidate,
  PostgresObservationQualificationExecution,
  PostgresObservationQualificationProbeOutput,
  PostgresObservationQualificationReceipt,
  PostgresObservationStreamEvidence,
  PostgresObservationStreamId,
} from './postgres-observation-qualification-runner.js';

const EMPTY_SHA256 = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';
const MAX_RECEIPT_BYTES = 1_048_576;
const MAX_RECEIPT_PAIR_BYTES = (MAX_RECEIPT_BYTES * 2) + 3;
const RECEIPT_KEYS = [
  'receiptKind', 'evidenceClass', 'authority', 'observationProfileQualification',
  'runtimeAdmissionStatus', 'productionAdmission', 'verifiedLease', 'reload',
  'directMapping', 'candidates', 'replay', 'replayStatus', 'receiptSha256',
] as const;

type CandidatePair = [
  PostgresObservationQualificationCandidate,
  PostgresObservationQualificationCandidate,
];
type ExecutionPair = [
  PostgresObservationQualificationExecution,
  PostgresObservationQualificationExecution,
];

export function createPostgresObservationQualificationReceipt(
  value: unknown,
): PostgresObservationQualificationReceipt {
  const input = closed(value, ['candidates', 'executions'], 'qualification receipt input');
  const candidates = parseCandidatePair(input.candidates);
  const executions = parseExecutionPair(input.executions, candidates, false);
  const body = receiptBody(candidates, executions);
  return deepFreeze({ ...body, receiptSha256: hash(canonical(body)) });
}

export function parsePostgresObservationQualificationReceiptJson(
  serialized: string,
): PostgresObservationQualificationReceipt {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8') > MAX_RECEIPT_BYTES) {
    throw new TypeError('qualification receipt must be bounded JSON');
  }
  const parsed = parseJsonWithoutDuplicateKeys(serialized, 'qualification receipt');
  if (canonical(parsed) !== serialized) {
    throw new TypeError('qualification receipt must use exact canonical JSON bytes');
  }
  return parseReceipt(parsed);
}

export function parsePostgresObservationQualificationReceiptPairJson(
  serialized: string,
): readonly [PostgresObservationQualificationReceipt, PostgresObservationQualificationReceipt] {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8') > MAX_RECEIPT_PAIR_BYTES) {
    throw new TypeError('qualification receipt pair must be bounded JSON');
  }
  const parsed = parseJsonWithoutDuplicateKeys(serialized, 'qualification receipt pair');
  if (canonical(parsed) !== serialized) {
    throw new TypeError('qualification receipt pair must use exact canonical JSON bytes');
  }
  const values = asDenseArray(parsed, 'qualification receipt pair');
  verifyPostgresObservationQualificationReceiptPair(values);
  const receipts = values.map(parseReceipt) as [
    PostgresObservationQualificationReceipt,
    PostgresObservationQualificationReceipt,
  ];
  if (receipts[0].candidates[0].observation.serverVersionNum !== 160009
    || receipts[1].candidates[0].observation.serverVersionNum !== 160015) {
    throw new TypeError('qualification receipt pair order must be 16.9 then 16.15');
  }
  return deepFreeze(receipts);
}

export function verifyPostgresObservationQualificationReceipt(value: unknown): void {
  parseReceipt(value);
}

export function verifyPostgresObservationQualificationReceiptPair(value: unknown): void {
  const values = asDenseArray(value, 'qualification receipt pair');
  if (values.length !== 2) {
    throw new TypeError('qualification requires exactly one 16.9 and one 16.15 receipt');
  }
  const receipts = values.map(parseReceipt) as [
    PostgresObservationQualificationReceipt,
    PostgresObservationQualificationReceipt,
  ];
  if (receipts.some((receipt) => receipt.observationProfileQualification !== 'pass'
    || receipt.replayStatus !== 'pass')) {
    throw new TypeError('cross-patch qualification requires two passing observation receipts');
  }
  const byPatch = new Map(
    receipts.map((receipt) => [receipt.candidates[0].observation.serverVersionNum, receipt]),
  );
  if (byPatch.size !== 2 || !byPatch.has(160009) || !byPatch.has(160015)) {
    throw new TypeError('qualification requires exactly one 16.9 and one 16.15 receipt');
  }
  const pg169 = byPatch.get(160009)!;
  const pg1615 = byPatch.get(160015)!;
  for (const receipt of receipts) {
    const version = receipt.candidates[0].observation.serverVersionNum;
    const expectation = POSTGRES_QUALIFICATION_IMAGE_EXPECTATIONS.find(
      (image) => image.serverVersionNum === version,
    );
    if (expectation === undefined || receipt.candidates.some((candidate) =>
      canonical(candidate.provenance.image)
        !== canonical(expectedQualificationImageIdentity(expectation)))) {
      throw new TypeError('qualification receipt image identity is not pinned');
    }
  }
  const candidate169 = pg169.candidates[0];
  const candidate1615 = pg1615.candidates[0];
  if (canonical(criticalBindings(candidate169)) !== canonical(criticalBindings(candidate1615))) {
    throw new TypeError('cross-patch qualification-critical bindings differ');
  }
  if (canonical(normalizedObservation(candidate169)) !== canonical(
    normalizedObservation(candidate1615),
  )) {
    throw new TypeError('cross-patch observation differs outside exact server version');
  }
  for (const [field, label] of [
    ['preflight', 'preflight'],
    ['queryAccounting', 'query accounting'],
    ['lifecycle', 'lifecycle'],
  ] as const) {
    if (canonical(candidate169[field]) !== canonical(candidate1615[field])) {
      throw new TypeError(`cross-patch ${label} differs`);
    }
  }
  const executions = receipts.flatMap((receipt) => receipt.replay.executions);
  requireAllDistinct(
    executions.map((execution) => execution.containerIdentitySha256),
    'cross-patch containers',
  );
  requireAllDistinct(
    executions.map((execution) => execution.volumeIdentitySha256),
    'cross-patch volumes',
  );
}

function parseReceipt(value: unknown): PostgresObservationQualificationReceipt {
  const receipt = closed(value, RECEIPT_KEYS, 'qualification receipt');
  exact(receipt.receiptKind, ['postgresql-public-observation-qualification-v1'] as const, 'kind');
  exact(receipt.evidenceClass, ['test-only-non-runtime'] as const, 'evidence class');
  exact(receipt.authority, ['observation-profile-only'] as const, 'authority');
  exact(
    receipt.observationProfileQualification, ['pass'] as const,
    'observation profile qualification',
  );
  exact(
    receipt.runtimeAdmissionStatus, ['withheld-independent-gates'] as const,
    'runtime admission status',
  );
  exact(receipt.productionAdmission, [false] as const, 'production admission');
  exact(receipt.verifiedLease, [false] as const, 'verified lease');
  exact(receipt.reload, [false] as const, 'reload');
  exact(receipt.directMapping, [false] as const, 'Direct Mapping');
  const candidates = parseCandidatePair(receipt.candidates);
  const replay = closed(receipt.replay, [
    'requiredRuns', 'canonicalCandidatesByteEqual', 'freshContainers', 'freshVolumes',
    'cleanupVerified', 'executions',
  ], 'replay');
  exactReplayFlags(replay);
  const executions = parseExecutionPair(replay.executions, candidates, true);
  exact(receipt.replayStatus, ['pass'] as const, 'replay status');
  const body = receiptBody(candidates, executions);
  if (digest(receipt.receiptSha256, 'receipt digest') !== hash(canonical(body))) {
    throw new Error('qualification receipt digest mismatch');
  }
  return deepFreeze({ ...body, receiptSha256: receipt.receiptSha256 as string });
}

function receiptBody(
  candidates: CandidatePair,
  executions: ExecutionPair,
) {
  return {
    receiptKind: 'postgresql-public-observation-qualification-v1' as const,
    evidenceClass: 'test-only-non-runtime' as const,
    authority: 'observation-profile-only' as const,
    observationProfileQualification: 'pass' as const,
    runtimeAdmissionStatus: 'withheld-independent-gates' as const,
    productionAdmission: false as const,
    verifiedLease: false as const,
    reload: false as const,
    directMapping: false as const,
    candidates,
    replay: {
      requiredRuns: 2 as const,
      canonicalCandidatesByteEqual: true as const,
      freshContainers: true as const,
      freshVolumes: true as const,
      cleanupVerified: true as const,
      executions,
    },
    replayStatus: 'pass' as const,
  };
}

function parseCandidatePair(value: unknown): CandidatePair {
  const values = asDenseArray(value, 'qualification candidates');
  if (values.length !== 2) throw new TypeError('qualification requires exactly two candidates');
  const candidates = values.map(parsePostgresObservationQualificationCandidate) as CandidatePair;
  if (canonical(candidates[0]) !== canonical(candidates[1])) {
    throw new TypeError('qualification replay candidates are not byte equal');
  }
  return candidates;
}

function parseExecutionPair(
  value: unknown,
  candidates: CandidatePair,
  includesCandidateDigest: boolean,
): ExecutionPair {
  const values = asDenseArray(value, 'qualification executions');
  if (values.length !== 2) throw new TypeError('qualification requires exactly two executions');
  const parsed = values.map((entry, index) => parseExecution(
    entry,
    (index + 1) as 1 | 2,
    candidates[index],
    includesCandidateDigest,
  )) as ExecutionPair;
  requireAllDistinct(
    parsed.map((execution) => execution.containerIdentitySha256),
    'qualification containers',
  );
  requireAllDistinct(
    parsed.map((execution) => execution.volumeIdentitySha256),
    'qualification volumes',
  );
  return parsed;
}

function parseExecution(
  value: unknown,
  expectedSlot: 1 | 2,
  candidate: PostgresObservationQualificationCandidate,
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
  const candidateSha256 = hash(canonical(candidate));
  if (includesCandidateDigest
    && digest(input.candidateSha256, 'candidate digest') !== candidateSha256) {
    throw new TypeError('execution candidate digest is inconsistent');
  }
  const before = prefixedDigest(input.imageConfigDigestBefore, 'image config before');
  const after = prefixedDigest(input.imageConfigDigestAfter, 'image config after');
  if (before !== candidate.provenance.image.configDigest
    || after !== candidate.provenance.image.configDigest) {
    throw new TypeError('execution image configuration drifted');
  }
  const stdout = parseOutput(input.stdout, 'stdout', 1);
  if (stdout.sha256 !== candidate.provenance.source.probeStdoutSha256) {
    throw new TypeError('execution stdout does not match candidate provenance');
  }
  const stderr = parseOutput(input.stderr, 'stderr');
  if (stderr.bytes !== 0 || stderr.sha256 !== EMPTY_SHA256) {
    throw new TypeError('execution stderr must bind the exact empty SHA-256');
  }
  return {
    slot: expectedSlot,
    candidateSha256,
    containerIdentitySha256: digest(input.containerIdentitySha256, 'container identity'),
    volumeIdentitySha256: digest(input.volumeIdentitySha256, 'volume identity'),
    imageConfigDigestBefore: before,
    imageConfigDigestAfter: after,
    stdout,
    stderr,
    exitCode: exact(input.exitCode, [0] as const, 'exit code'),
    timedOut: exact(input.timedOut, [false] as const, 'timeout state'),
    outputLimitExceeded: exact(input.outputLimitExceeded, [false] as const, 'output limit'),
    cleanup: parseCleanup(input.cleanup),
  };
}

function parseOutput(value: unknown, label: string, minimumBytes = 0) {
  const input = closed(value, ['bytes', 'sha256', 'truncated'], `${label} evidence`);
  const bytes = asInteger(input.bytes, `${label} byte count`, minimumBytes);
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

function criticalBindings(candidate: PostgresObservationQualificationCandidate) {
  const { probeStdoutSha256: _probeStdoutSha256, ...source } = candidate.provenance.source;
  return {
    source,
    inputs: candidate.provenance.inputs,
    toolchain: candidate.provenance.toolchain,
  };
}

function normalizedObservation(candidate: PostgresObservationQualificationCandidate) {
  const {
    serverVersion: _serverVersion,
    serverVersionNum: _serverVersionNum,
    ...observation
  } = candidate.observation;
  return observation;
}

function requireAllDistinct(values: readonly string[], label: string): void {
  if (new Set(values).size !== values.length) {
    throw new TypeError(`${label} must be distinct`);
  }
}

function exactReplayFlags(value: Record<string, unknown>): void {
  exact(value.requiredRuns, [2] as const, 'required replay count');
  exact(value.canonicalCandidatesByteEqual, [true] as const, 'candidate replay equality');
  exact(value.freshContainers, [true] as const, 'fresh containers');
  exact(value.freshVolumes, [true] as const, 'fresh volumes');
  exact(value.cleanupVerified, [true] as const, 'verified cleanup');
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

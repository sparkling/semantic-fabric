import { canonical } from '@metaharness/harness';
import {
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  asInteger,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  POSTGRES_OBSERVATION_COMPLETED_PHASES,
  POSTGRES_OBSERVATION_MAX_OUTPUT_BYTES,
  POSTGRES_OBSERVATION_STREAM_IDS,
} from './postgres-observation-qualification-runner.js';
import type {
  PostgresObservationQualificationCandidate,
  PostgresObservationQualificationProbeOutput,
  PostgresObservationStreamEvidence,
} from './postgres-observation-qualification-runner.js';
import {
  POSTGRES_QUALIFICATION_BUILDER_EXPECTATION,
  expectedQualificationBuilderImageIdentity,
} from './postgres-observation-qualification-protocol.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

const MAX_RELATIONS = 4_096;
const MAX_ATTRIBUTES = 65_536;
const MAX_CONSTRAINTS = 65_536;
const GIT_OBJECT_PATTERN = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/;

export function parsePostgresObservationQualificationCandidate(
  value: unknown,
): PostgresObservationQualificationCandidate {
  const input = closed(
    value,
    ['provenance', 'preflight', 'observation', 'queryAccounting', 'lifecycle'],
    'candidate',
  );
  return deepFreeze({
    provenance: parseProvenance(input.provenance),
    ...parseProbeSections(input),
  });
}

export function parsePostgresObservationQualificationCandidateJson(
  serialized: string,
): PostgresObservationQualificationCandidate {
  return parsePostgresObservationQualificationCandidate(parseStrictJson(serialized, 'candidate'));
}

export function parsePostgresObservationQualificationProbeOutput(
  serialized: string,
): PostgresObservationQualificationProbeOutput {
  const value = closed(
    parseStrictJson(serialized, 'qualification probe output'),
    ['preflight', 'observation', 'queryAccounting', 'lifecycle'],
    'qualification probe output',
  );
  return deepFreeze(parseProbeSections(value));
}

function parseStrictJson(serialized: string, label: string): unknown {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8') > POSTGRES_OBSERVATION_MAX_OUTPUT_BYTES) {
    throw new TypeError(`${label} must be bounded JSON`);
  }
  return parseJsonWithoutDuplicateKeys(serialized, label);
}

function parseProbeSections(
  input: Record<string, unknown>,
): PostgresObservationQualificationProbeOutput {
  const preflight = parsePreflight(input.preflight);
  const observation = parseObservation(input.observation);
  const queryAccounting = parseQueryAccounting(input.queryAccounting, observation.streaming);
  const lifecycle = parseLifecycle(input.lifecycle);
  return { preflight, observation, queryAccounting, lifecycle };
}

function parseProvenance(value: unknown): PostgresObservationQualificationCandidate['provenance'] {
  const input = closed(value, ['source', 'inputs', 'toolchain', 'image'], 'provenance');
  const sourceInput = closed(input.source, [
    'commit', 'tree', 'cargoLockSha256', 'probeArtifactSha256', 'probeStdoutSha256',
  ], 'source provenance');
  const commit = gitObject(sourceInput.commit, 'source commit');
  const tree = gitObject(sourceInput.tree, 'source tree');
  if (commit.length !== tree.length) throw new TypeError('source Git object formats differ');
  const inputs = closed(input.inputs, [
    'adrSha256', 'profileSha256', 'queriesSha256', 'testsSha256', 'fixtureSha256',
    'runnerSha256', 'protocolSha256',
  ], 'input provenance');
  const toolchain = closed(input.toolchain, [
    'rustcVvSha256', 'cargoVersionSha256', 'targetTriple', 'builderImage',
  ], 'toolchain provenance');
  const builderImage = closed(toolchain.builderImage, [
    'repository', 'manifestDigest', 'configDigest', 'platform', 'inspectSha256',
  ], 'builder image provenance');
  const image = closed(input.image, [
    'repository', 'manifestDigest', 'configDigest', 'platform', 'inspectSha256',
  ], 'image provenance');
  return {
    source: {
      commit,
      tree,
      cargoLockSha256: digest(sourceInput.cargoLockSha256, 'Cargo lock digest'),
      probeArtifactSha256: digest(sourceInput.probeArtifactSha256, 'probe artifact digest'),
      probeStdoutSha256: digest(sourceInput.probeStdoutSha256, 'probe stdout digest'),
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
      builderImage: parseBuilderImage(builderImage),
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

function parseBuilderImage(value: Record<string, unknown>) {
  const actual = {
    repository: exact(value.repository, ['rust'] as const, 'builder image repository'),
    manifestDigest: prefixedDigest(value.manifestDigest, 'builder image manifest digest'),
    configDigest: prefixedDigest(value.configDigest, 'builder image config digest'),
    platform: exact(value.platform, ['linux/amd64'] as const, 'builder image platform'),
    inspectSha256: digest(value.inspectSha256, 'builder image inspection digest'),
  };
  const expected = expectedQualificationBuilderImageIdentity(
    POSTGRES_QUALIFICATION_BUILDER_EXPECTATION,
  );
  if (canonical(actual) !== canonical(expected)) {
    throw new TypeError('builder image identity does not match the qualification protocol');
  }
  return actual;
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

function parseObservation(
  value: unknown,
): PostgresObservationQualificationCandidate['observation'] {
  const input = closed(value, [
    'serverVersion', 'serverVersionNum', 'guard', 'rowCounts', 'streaming', 'identity',
    'legacyCoordinateComparison', 'errorCode', 'failurePhase',
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
    relations: parseStreamEvidence(streams.relations, 'relations', MAX_RELATIONS),
    attributes: parseStreamEvidence(streams.attributes, 'attributes', MAX_ATTRIBUTES),
    catalogConstraints: parseStreamEvidence(
      streams.catalogConstraints,
      'catalog constraints',
      MAX_CONSTRAINTS - rowCounts.notNullConstraints,
    ),
  };
  if (rowCounts.relations !== streaming.relations.decoded
    || rowCounts.attributes !== streaming.attributes.decoded
    || rowCounts.catalogConstraints !== streaming.catalogConstraints.decoded) {
    throw new TypeError('row counts do not match streaming decode evidence');
  }
  const guard = exact(input.guard, ['pass'] as const, 'guard result');
  const identity = parseIdentity(input.identity);
  const legacyCoordinateComparison = exact(
    input.legacyCoordinateComparison, ['equal'] as const, 'legacy coordinate comparison',
  );
  const errorCode = exact(input.errorCode, [null] as const, 'error code');
  const failurePhase = exact(input.failurePhase, [null] as const, 'failure phase');
  if (rowCounts.relations > rowCounts.attributes
    || rowCounts.notNullConstraints > rowCounts.attributes
    || rowCounts.combinedConstraints > MAX_CONSTRAINTS) {
    throw new TypeError('successful observation row counts are inconsistent');
  }
  return {
    serverVersion: input.serverVersion as string,
    serverVersionNum,
    guard,
    rowCounts,
    streaming,
    identity,
    legacyCoordinateComparison,
    errorCode,
    failurePhase,
  };
}

function parseQueryAccounting(
  value: unknown,
  rich: PostgresObservationQualificationCandidate['observation']['streaming'],
): PostgresObservationQualificationCandidate['queryAccounting'] {
  const input = closed(value, [
    'prequalificationGuard', 'richCapture', 'guardQueriesObserved',
    'streamQueriesObserved', 'totalQueriesObserved', 'streams',
  ], 'query accounting');
  const values = asDenseArray(input.streams, 'accounted streams');
  if (values.length !== POSTGRES_OBSERVATION_STREAM_IDS.length) {
    throw new TypeError('accounted stream inventory must contain exactly 10 streams');
  }
  const streams = values.map((entry, index) => {
    const item = closed(entry, ['id', 'evidence'], `accounted stream ${index}`);
    const id = exact(
      item.id, [POSTGRES_OBSERVATION_STREAM_IDS[index]] as const, 'accounted stream order',
    );
    return {
      id,
      evidence: parseStreamEvidence(
        item.evidence, `accounted stream ${id}`, expectedStreamCap(id, rich),
      ),
    };
  }) as PostgresObservationQualificationCandidate['queryAccounting']['streams'];
  for (const [index, field] of [
    [7, 'relations'], [8, 'attributes'], [9, 'catalogConstraints'],
  ] as const) {
    if (canonical(streams[index].evidence) !== canonical(rich[field])) {
      throw new TypeError(`${streams[index].id} rich stream evidence does not match observation`);
    }
  }
  return {
    prequalificationGuard: exact(
      input.prequalificationGuard, [1] as const, 'prequalification guard query count',
    ),
    richCapture: exact(input.richCapture, [4] as const, 'rich capture query count'),
    guardQueriesObserved: exact(
      input.guardQueriesObserved, [2] as const, 'observed guard query count',
    ),
    streamQueriesObserved: exact(
      input.streamQueriesObserved, [10] as const, 'observed stream query count',
    ),
    totalQueriesObserved: exact(
      input.totalQueriesObserved, [12] as const, 'observed total query count',
    ),
    streams,
  };
}

function expectedStreamCap(
  id: (typeof POSTGRES_OBSERVATION_STREAM_IDS)[number],
  rich: PostgresObservationQualificationCandidate['observation']['streaming'],
): number {
  if (id === 'legacy-tables' || id === 'legacy-earlier-collisions'
    || id === 'rich-relations') return 4_096;
  if (id === 'rich-catalog-constraints') return rich.catalogConstraints.cap;
  return 65_536;
}

function parseLifecycle(value: unknown): PostgresObservationQualificationCandidate['lifecycle'] {
  const input = closed(
    value, ['savepoint', 'recovery', 'commit', 'phasesCompleted'], 'lifecycle',
  );
  const values = asDenseArray(input.phasesCompleted, 'completed phases');
  if (values.length !== POSTGRES_OBSERVATION_COMPLETED_PHASES.length) {
    throw new TypeError('completed phase inventory must contain exactly 10 phases');
  }
  const phasesCompleted = values.map((entry, index) => exact(
    entry, [POSTGRES_OBSERVATION_COMPLETED_PHASES[index]] as const, 'completed phase order',
  )) as PostgresObservationQualificationCandidate['lifecycle']['phasesCompleted'];
  return {
    savepoint: exact(input.savepoint, ['released'] as const, 'savepoint lifecycle'),
    recovery: exact(input.recovery, ['not-needed'] as const, 'recovery lifecycle'),
    commit: exact(input.commit, ['complete'] as const, 'commit lifecycle'),
    phasesCompleted,
  };
}

function parseStreamEvidence(
  value: unknown,
  label: string,
  expectedCap?: number,
): PostgresObservationStreamEvidence {
  const input = closed(value, [
    'cap', 'polled', 'decoded', 'retainedPeak', 'overflow', 'terminal',
  ], `${label} streaming evidence`);
  const cap = asInteger(input.cap, `${label} cap`);
  const polled = asInteger(input.polled, `${label} polled rows`);
  const decoded = asInteger(input.decoded, `${label} decoded rows`);
  const retainedPeak = asInteger(input.retainedPeak, `${label} retained peak`);
  const terminal = exact(input.terminal, ['complete'] as const, `${label} terminal state`);
  if (expectedCap !== undefined && cap !== expectedCap) {
    throw new TypeError(`${label} cap is inconsistent`);
  }
  const overflow = exact(input.overflow, [false] as const, `${label} overflow state`);
  if (cap > MAX_CONSTRAINTS) {
    throw new TypeError(`${label} streaming bound is invalid`);
  }
  if (polled !== decoded || decoded > cap || retainedPeak !== decoded) {
    throw new TypeError(`${label} decoded its overflow sentinel or has inconsistent retention`);
  }
  return { cap, polled, decoded, retainedPeak, overflow, terminal };
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

function closed(value: unknown, keys: readonly string[], label: string): Record<string, unknown> {
  const input = asClosedRecord(value, label);
  assertExactKeys(input, keys, label);
  return input;
}

function exact<const T extends readonly (string | number | boolean | null)[]>(
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

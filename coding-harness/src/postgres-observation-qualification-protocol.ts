// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  asClosedRecord,
  asDenseArray,
  asInteger,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const POSTGRES_QUALIFICATION_PROTOCOL_PATH =
  'tests/postgresql/observation-qualification-protocol-v1.json' as const;
export const POSTGRES_QUALIFICATION_FIXTURE_PATH =
  'tests/postgresql/observation-qualification-fixture.sql' as const;

export const POSTGRES_QUALIFICATION_BUILDER_EXPECTATION = Object.freeze({
  reference: 'rust@sha256:c993d32d95cc146bd12c84d66f0b924a6a96f3988325f39c144f2f9893dea120' as const,
  configDigest: 'sha256:13783f492bda9b66aed2e6c95c874499f1c5020821704d412113c104bc9cf14f' as const,
  platform: 'linux/amd64' as const,
});

const STREAMS = Object.freeze([
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

const PHASES = Object.freeze([
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

export const POSTGRES_QUALIFICATION_IMAGE_EXPECTATIONS = Object.freeze([
  Object.freeze({
    patch: '16.9' as const,
    serverVersionNum: 160009 as const,
    reference: 'postgres@sha256:ddfe3e8713e3ee5b8f286082cb12512488dfbf3f5a1ecb0b74a42e6055af0a5f' as const,
    configDigest: 'sha256:5d77af46c4cb952e855106b849c6972b8a1ee2a2518159eb1854702fda47542f' as const,
  }),
  Object.freeze({
    patch: '16.15' as const,
    serverVersionNum: 160015 as const,
    reference: 'postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8' as const,
    configDigest: 'sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2' as const,
  }),
] as const);

export type PostgresQualificationPatch =
  (typeof POSTGRES_QUALIFICATION_IMAGE_EXPECTATIONS)[number]['patch'];

export interface PostgresQualificationImage {
  readonly patch: PostgresQualificationPatch;
  readonly serverVersionNum: 160009 | 160015;
  readonly reference: string;
  readonly configDigest: string;
}

export interface PostgresQualificationBuilderImage {
  readonly reference: typeof POSTGRES_QUALIFICATION_BUILDER_EXPECTATION.reference;
  readonly configDigest: typeof POSTGRES_QUALIFICATION_BUILDER_EXPECTATION.configDigest;
  readonly platform: 'linux/amd64';
}

export interface PostgresQualificationProtocol {
  readonly schemaVersion:
    'semantic-fabric.postgresql-public-observation-qualification-protocol/v1';
  readonly receiptKind: 'postgresql-public-observation-qualification-v1';
  readonly authority: 'observation-profile-only';
  readonly platform: 'linux/amd64';
  readonly builder: PostgresQualificationBuilderImage;
  readonly database: {
    readonly name: 'sf_observation_qualification_v1';
    readonly template: 'template0';
    readonly encoding: 'UTF8';
    readonly localeProvider: 'libc';
    readonly lcCollate: 'C';
    readonly lcCtype: 'C';
  };
  readonly roles: {
    readonly owner: 'sf_observation_owner_v1';
    readonly observer: 'sf_observation_observer_v1';
  };
  readonly container: {
    readonly networkMode: 'none';
    readonly pgdataPath: '/var/lib/postgresql/data';
    readonly socketPath: '/var/run/postgresql';
    readonly fixturePath: '/docker-entrypoint-initdb.d/010-observation-qualification.sql';
    readonly probePath: '/opt/semantic-fabric/bin/postgres-observation-qualification';
    readonly ownerLabelKey:
      'io.github.sparkling.semantic-fabric.postgresql-observation-qualification';
  };
  readonly probe: {
    readonly package: 'sf-conformance';
    readonly bin: 'postgres-observation-qualification';
    readonly artifactPath:
      'target/x86_64-unknown-linux-gnu/release/postgres-observation-qualification';
    readonly maxOutputBytes: 262144;
  };
  readonly replay: {
    readonly requiredRuns: 2;
    readonly readinessTimeoutMs: 60000;
    readonly commandTimeoutMs: 60000;
    readonly cleanupTimeoutMs: 30000;
  };
  readonly images: readonly [PostgresQualificationImage, PostgresQualificationImage];
  readonly streams: typeof STREAMS;
  readonly phases: typeof PHASES;
}

export function parsePostgresQualificationProtocol(
  serialized: string,
): PostgresQualificationProtocol {
  if (typeof serialized !== 'string' || Buffer.byteLength(serialized, 'utf8') > 65_536
    || serialized.includes('\0') || serialized.includes('\r')) {
    throw new TypeError('POSTGRES_QUALIFICATION_PROTOCOL_ENCODING_INVALID');
  }
  const root = closed(parseJsonWithoutDuplicateKeys(serialized, 'PostgreSQL qualification protocol'), [
    'schemaVersion', 'receiptKind', 'authority', 'platform', 'builder', 'database', 'roles',
    'container', 'probe', 'replay', 'images', 'streams', 'phases',
  ], 'protocol');
  const database = closed(root.database, [
    'name', 'template', 'encoding', 'localeProvider', 'lcCollate', 'lcCtype',
  ], 'database');
  const builder = closed(root.builder, [
    'reference', 'configDigest', 'platform',
  ], 'builder');
  const roles = closed(root.roles, ['owner', 'observer'], 'roles');
  const container = closed(root.container, [
    'networkMode', 'pgdataPath', 'socketPath', 'fixturePath', 'probePath', 'ownerLabelKey',
  ], 'container');
  const probe = closed(root.probe, [
    'package', 'bin', 'artifactPath', 'maxOutputBytes',
  ], 'probe');
  const replay = closed(root.replay, [
    'requiredRuns', 'readinessTimeoutMs', 'commandTimeoutMs', 'cleanupTimeoutMs',
  ], 'replay');
  const images = parseImages(root.images);
  const protocol: PostgresQualificationProtocol = {
    schemaVersion: exact(root.schemaVersion,
      'semantic-fabric.postgresql-public-observation-qualification-protocol/v1', 'schema'),
    receiptKind: exact(root.receiptKind,
      'postgresql-public-observation-qualification-v1', 'receipt kind'),
    authority: exact(root.authority, 'observation-profile-only', 'authority'),
    platform: exact(root.platform, 'linux/amd64', 'platform'),
    builder: {
      reference: exact(
        builder.reference, POSTGRES_QUALIFICATION_BUILDER_EXPECTATION.reference,
        'builder reference',
      ),
      configDigest: exact(
        builder.configDigest, POSTGRES_QUALIFICATION_BUILDER_EXPECTATION.configDigest,
        'builder config digest',
      ),
      platform: exact(builder.platform, 'linux/amd64', 'builder platform'),
    },
    database: {
      name: exact(database.name, 'sf_observation_qualification_v1', 'database name'),
      template: exact(database.template, 'template0', 'database template'),
      encoding: exact(database.encoding, 'UTF8', 'database encoding'),
      localeProvider: exact(database.localeProvider, 'libc', 'locale provider'),
      lcCollate: exact(database.lcCollate, 'C', 'LC_COLLATE'),
      lcCtype: exact(database.lcCtype, 'C', 'LC_CTYPE'),
    },
    roles: {
      owner: exact(roles.owner, 'sf_observation_owner_v1', 'owner role'),
      observer: exact(roles.observer, 'sf_observation_observer_v1', 'observer role'),
    },
    container: {
      networkMode: exact(container.networkMode, 'none', 'network mode'),
      pgdataPath: exact(container.pgdataPath, '/var/lib/postgresql/data', 'PGDATA path'),
      socketPath: exact(container.socketPath, '/var/run/postgresql', 'socket path'),
      fixturePath: exact(container.fixturePath,
        '/docker-entrypoint-initdb.d/010-observation-qualification.sql', 'fixture path'),
      probePath: exact(container.probePath,
        '/opt/semantic-fabric/bin/postgres-observation-qualification', 'probe path'),
      ownerLabelKey: exact(container.ownerLabelKey,
        'io.github.sparkling.semantic-fabric.postgresql-observation-qualification',
        'owner label'),
    },
    probe: {
      package: exact(probe.package, 'sf-conformance', 'probe package'),
      bin: exact(probe.bin, 'postgres-observation-qualification', 'probe bin'),
      artifactPath: exact(probe.artifactPath,
        'target/x86_64-unknown-linux-gnu/release/postgres-observation-qualification',
        'probe artifact path'),
      maxOutputBytes: integer(probe.maxOutputBytes, 262_144, 'probe output bound'),
    },
    replay: {
      requiredRuns: integer(replay.requiredRuns, 2, 'required runs'),
      readinessTimeoutMs: integer(replay.readinessTimeoutMs, 60_000, 'readiness timeout'),
      commandTimeoutMs: integer(replay.commandTimeoutMs, 60_000, 'command timeout'),
      cleanupTimeoutMs: integer(replay.cleanupTimeoutMs, 30_000, 'cleanup timeout'),
    },
    images,
    streams: exactArray(root.streams, STREAMS, 'streams'),
    phases: exactArray(root.phases, PHASES, 'phases'),
  };
  return deepFreeze(protocol);
}

export function protocolImage(
  protocol: PostgresQualificationProtocol,
  patch: PostgresQualificationPatch,
): PostgresQualificationImage {
  const image = protocol.images.find((entry) => entry.patch === patch);
  if (image === undefined) throw new TypeError('POSTGRES_QUALIFICATION_PATCH_INVALID');
  return image;
}

export function expectedQualificationImageIdentity(image: PostgresQualificationImage) {
  const normalized = {
    configDigest: image.configDigest,
    os: 'linux',
    architecture: 'amd64',
    requiredManifest: image.reference,
  };
  return Object.freeze({
    repository: 'postgres' as const,
    manifestDigest: image.reference.slice('postgres@'.length),
    configDigest: image.configDigest,
    platform: 'linux/amd64' as const,
    inspectSha256: createHash('sha256')
      .update(Buffer.from(JSON.stringify(normalized), 'utf8')).digest('hex'),
  });
}

export function expectedQualificationBuilderImageIdentity(
  image: PostgresQualificationBuilderImage,
) {
  const normalized = {
    configDigest: image.configDigest,
    os: 'linux',
    architecture: 'amd64',
    requiredManifest: image.reference,
  };
  return Object.freeze({
    repository: 'rust' as const,
    manifestDigest: image.reference.slice('rust@'.length),
    configDigest: image.configDigest,
    platform: 'linux/amd64' as const,
    inspectSha256: createHash('sha256')
      .update(Buffer.from(JSON.stringify(normalized), 'utf8')).digest('hex'),
  });
}

function parseImages(value: unknown): readonly [PostgresQualificationImage, PostgresQualificationImage] {
  const rows = asDenseArray(value, 'protocol images');
  if (rows.length !== POSTGRES_QUALIFICATION_IMAGE_EXPECTATIONS.length) {
    throw new TypeError('POSTGRES_QUALIFICATION_IMAGE_SET_INVALID');
  }
  const parsed = rows.map((value, index) => {
    const row = closed(value, [
      'patch', 'serverVersionNum', 'reference', 'configDigest',
    ], `image ${index + 1}`);
    const expected = POSTGRES_QUALIFICATION_IMAGE_EXPECTATIONS[index];
    return {
      patch: exact(row.patch, expected.patch, `image ${index + 1} patch`),
      serverVersionNum: integer(
        row.serverVersionNum, expected.serverVersionNum, `image ${index + 1} server version`,
      ),
      reference: exact(row.reference, expected.reference, `image ${index + 1} reference`),
      configDigest: exact(
        row.configDigest, expected.configDigest, `image ${index + 1} config digest`,
      ),
    };
  });
  return parsed as unknown as readonly [PostgresQualificationImage, PostgresQualificationImage];
}

function exactArray<T extends readonly string[]>(
  value: unknown,
  expected: T,
  label: string,
): T {
  const actual = asDenseArray(value, label);
  if (actual.length !== expected.length
    || actual.some((entry, index) => entry !== expected[index])) {
    throw new TypeError(`POSTGRES_QUALIFICATION_${label.toUpperCase()}_INVALID`);
  }
  return expected;
}

function closed(value: unknown, keys: readonly string[], label: string): Record<string, unknown> {
  const record = asClosedRecord(value, `PostgreSQL qualification ${label}`);
  assertExactKeys(record, keys, `PostgreSQL qualification ${label}`);
  return record;
}

function exact<const T extends string>(value: unknown, expected: T, label: string): T {
  if (value !== expected) throw new TypeError(`POSTGRES_QUALIFICATION_${code(label)}_INVALID`);
  return expected;
}

function integer<const T extends number>(value: unknown, expected: T, label: string): T {
  if (asInteger(value, label) !== expected) {
    throw new TypeError(`POSTGRES_QUALIFICATION_${code(label)}_INVALID`);
  }
  return expected;
}

function code(label: string): string {
  return label.toUpperCase().replaceAll(/[^A-Z0-9]+/g, '_');
}

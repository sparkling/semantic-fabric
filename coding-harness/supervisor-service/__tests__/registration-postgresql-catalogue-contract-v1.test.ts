// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  assertPostgresCatalogueDigestV1,
  parsePostgresCatalogueContractV1,
} from '../src/registration-postgresql-catalogue-contract-v1.js';
import { validatePostgresCatalogueCoreV1 }
  from '../src/registration-postgresql-catalogue-core-v1.js';
import { validatePostgresCatalogueQueryV1 }
  from '../src/registration-postgresql-catalogue-query-v1.js';
import { scanPostgresCatalogueBytesV1 }
  from '../src/registration-postgresql-catalogue-scanner-v1.js';
import { validatePostgresCatalogueSecurityV1 }
  from '../src/registration-postgresql-catalogue-security-v1.js';
import { reconstructPostgresCatalogueShapeV1 }
  from '../src/registration-postgresql-catalogue-shape-v1.js';
import { POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1 }
  from '../src/registration-postgresql-row-codecs-v1.js';

const CATALOGUE_PATH = fileURLToPath(
  new URL('../migrations/catalog-contract-v1.json', import.meta.url),
);
const EXPECTED_SHA256 =
  'e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69';
const ROOT_KEYS = Object.freeze([
  'domain', 'schemaVersion', 'schemaName', 'ownerRole', 'limits', 'schemas',
  'domains', 'relations', 'columns', 'constraints', 'indexes', 'foreignKeyTriggers',
  'policies', 'objectAcls', 'columnAcls', 'defaultAcls', 'implicitObjects', 'exactQueries',
]);

describe('PostgreSQL canonical catalogue contract V1', () => {
  it('accepts the exact oracle at every independent validation stage', () => {
    const scanned = scanPostgresCatalogueBytesV1(readFileSync(CATALOGUE_PATH));
    const contract = reconstructPostgresCatalogueShapeV1(JSON.parse(scanned.text));

    expect(() => validatePostgresCatalogueCoreV1(contract)).not.toThrow();
    expect(() => validatePostgresCatalogueSecurityV1(contract)).not.toThrow();
    expect(() => validatePostgresCatalogueQueryV1(contract)).not.toThrow();
  });

  it('parses, replays, hashes, and freezes the reviewed exact oracle', () => {
    const bytes = readFileSync(CATALOGUE_PATH);
    const parsed = parsePostgresCatalogueContractV1(bytes);

    expect(parsed).toMatchObject({
      rawByteLength: 232_822,
      rawSha256: EXPECTED_SHA256,
      scan: { nodes: 9_125, records: 963, maximumDepth: 8 },
      authority: 'none',
      readinessAuthorized: false,
    });
    expect(Object.isFrozen(parsed)).toBe(true);
    expect(Object.isFrozen(parsed.scan)).toBe(true);
    expectDeepFrozen(parsed.contract);
    expect(() => assertPostgresCatalogueDigestV1(parsed, EXPECTED_SHA256)).not.toThrow();
  });

  it('pins the literal root-line framing, order, and physical response-status type', () => {
    const text = readFileSync(CATALOGUE_PATH, 'utf8');
    const lines = text.split('\n');
    const rootKeys = lines.slice(1, -2).map((line) => JSON.parse(
      line.slice(2, line.indexOf(': ')),
    ) as string);
    const parsed = parsePostgresCatalogueContractV1(new TextEncoder().encode(text));
    const columns = parsed.contract.columns as readonly Record<string, unknown>[];

    expect(text.startsWith('{\n')).toBe(true);
    expect(text.endsWith('}\n')).toBe(true);
    expect(text).not.toContain('\r');
    expect(lines).toHaveLength(21);
    expect(rootKeys).toEqual(ROOT_KEYS);
    expect(columns.find((value) => value.relation === 'registration_results'
      && value.name === 'response_status')).toMatchObject({
      typeSchema: 'pg_catalog', typeName: 'int2', baseProjectionType: 'text',
    });
  });

  it('returns independent storage and retains no mutable input byte alias', () => {
    const firstBytes = new Uint8Array(readFileSync(CATALOGUE_PATH));
    const secondBytes = new Uint8Array(firstBytes);
    const first = parsePostgresCatalogueContractV1(firstBytes);
    const second = parsePostgresCatalogueContractV1(secondBytes);

    firstBytes.fill(0);
    expect(first.rawSha256).toBe(EXPECTED_SHA256);
    expect(first.contract).not.toBe(second.contract);
    expect(first.contract).toEqual(second.contract);
  });

  it('rejects unbranded handles and malformed, zero, or mismatching digests', () => {
    const parsed = parsePostgresCatalogueContractV1(readFileSync(CATALOGUE_PATH));
    const impostor = Object.freeze({ ...parsed });

    for (const [value, digest] of [
      [impostor, EXPECTED_SHA256],
      [parsed, '0'.repeat(64)],
      [parsed, EXPECTED_SHA256.toUpperCase()],
      [parsed, 'f'.repeat(64)],
      [parsed, 'not-a-digest'],
    ] as const) {
      expect(() => assertPostgresCatalogueDigestV1(value, digest))
        .toThrowError('PostgreSQL catalogue contract is invalid');
    }
  });

  it('binds the exact query projection to the private row-codec tuple', () => {
    const parsed = parsePostgresCatalogueContractV1(readFileSync(CATALOGUE_PATH));
    const queries = parsed.contract.exactQueries;
    expect(Array.isArray(queries)).toBe(true);
    const query = (queries as readonly Record<string, unknown>[])[0]!;
    const projection = query.projection as readonly Record<string, unknown>[];

    expect(projection.map((value) => value.outputAlias))
      .toEqual(POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1);
    expect(projection.find((value) => value.outputAlias === 'result_response_status_text'))
      .toMatchObject({ column: 'response_status', cast: 'text' });
  });
});

function expectDeepFrozen(value: unknown): void {
  if (value === null || typeof value !== 'object') return;
  expect(Object.isFrozen(value)).toBe(true);
  for (const nested of Object.values(value)) expectDeepFrozen(nested);
}

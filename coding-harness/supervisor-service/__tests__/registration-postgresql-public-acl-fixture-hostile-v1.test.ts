// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import {
  copyPostgresPublicAclFixtureBytesV1,
  parsePostgresPublicAclFixtureV1,
} from './registration-postgresql-public-acl-fixture-reader-v1.js';

const FIXTURE = new Uint8Array(readFileSync(resolve(
  import.meta.dirname,
  'fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json',
)));
const INVALID = 'PostgreSQL PUBLIC ACL fixture is invalid';
const SHA256 = 'a108e05f9cfd6d6485a86fe198a87b3800e21986b5c62e6251519de6577d05be';
const KEYS = Object.freeze([
  'objectClass', 'schemaName', 'objectName', 'subobjectName', 'objectKind',
  'routineIdentityArguments', 'privilege', 'grantable',
]);

describe('PostgreSQL PUBLIC ACL fixture reader V1', () => {
  it('returns a branded immutable non-authorizing handle for the fixture', () => {
    const handle = parsePostgresPublicAclFixtureV1(FIXTURE);

    expect(handle).toMatchObject({
      rawByteLength: 860_988,
      rawSha256: SHA256,
      authority: 'none',
      readinessAuthorized: false,
      scan: { nodes: 36_532, records: 4_059, maximumDepth: 3 },
    });
    expect(handle.records).toHaveLength(4_059);
    expect(handle).not.toHaveProperty('plan');
    expect(handle).not.toHaveProperty('migrationAuthorized');
    expect(handle).not.toHaveProperty('observationAuthorized');
    expect(Object.isFrozen(handle)).toBe(true);
    expect(Object.isFrozen(handle.scan)).toBe(true);
    expect(Object.isFrozen(handle.records)).toBe(true);
    expect(Object.isFrozen(handle.records[0])).toBe(true);
  });

  it('snapshots caller bytes and returns fresh non-aliasing branded copies', () => {
    const mutable = new Uint8Array(FIXTURE);
    const handle = parsePostgresPublicAclFixtureV1(mutable);
    mutable.fill(0);

    const first = copyPostgresPublicAclFixtureBytesV1(handle);
    expect(sha256(first)).toBe(SHA256);
    first.fill(0);
    expect(sha256(copyPostgresPublicAclFixtureBytesV1(handle))).toBe(SHA256);
  });

  it('rejects structural, symbol, accessor, and proxy handle forgeries trap-free', () => {
    const real = parsePostgresPublicAclFixtureV1(FIXTURE);
    const structural = Object.freeze({ ...real });
    const withSymbol = { ...real, [Symbol('hidden')]: false };
    let accessorReads = 0;
    const withAccessor = Object.defineProperty({ ...real }, 'authority', {
      enumerable: true,
      get: () => { accessorReads += 1; return 'none'; },
    });
    let trapCalls = 0;
    const proxy = new Proxy(real, {
      get() { trapCalls += 1; return undefined; },
    });

    for (const value of [structural, withSymbol, withAccessor, proxy]) {
      expect(() => copyPostgresPublicAclFixtureBytesV1(value)).toThrowError(
        new TypeError(INVALID),
      );
    }
    expect(accessorReads).toBe(0);
    expect(trapCalls).toBe(0);
  });

  it('rejects Buffer, proxy, exotic, shared, and non-Uint8Array carriers', () => {
    class ExoticBytes extends Uint8Array {}
    const valid = canonical([record()]);
    const shared = new SharedArrayBuffer(valid.byteLength);
    new Uint8Array(shared).set(valid);
    let carrierTrapCalls = 0;
    const proxy = new Proxy(new Uint8Array(valid), {
      get() { carrierTrapCalls += 1; throw new Error('unexpected get trap'); },
      getPrototypeOf() {
        carrierTrapCalls += 1;
        throw new Error('unexpected getPrototypeOf trap');
      },
    });
    const hostile = [
      Buffer.from(valid),
      proxy,
      new ExoticBytes(valid),
      new Uint8ClampedArray(valid),
      new DataView(valid.buffer, valid.byteOffset, valid.byteLength),
      new Uint8Array(shared),
      valid.buffer,
      new TextDecoder().decode(valid),
    ];

    expect(parsePostgresPublicAclFixtureV1(valid).records).toHaveLength(1);
    const parse = vi.spyOn(JSON, 'parse');
    try {
      for (const value of hostile) expectInvalid(value);
      expect(parse).not.toHaveBeenCalled();
      expect(carrierTrapCalls).toBe(0);
    } finally {
      parse.mockRestore();
    }
  });

  it('rejects a detached Uint8Array carrier', () => {
    const valid = canonical([record()]);
    const buffer = new ArrayBuffer(valid.byteLength);
    const view = new Uint8Array(buffer);
    view.set(valid);

    expect(parsePostgresPublicAclFixtureV1(view).records).toHaveLength(1);
    structuredClone(buffer, { transfer: [buffer] });

    expectInvalid(view);
  });

  it('rejects a resizable Uint8Array carrier when the runtime supports one', () => {
    const resizable = Object.getOwnPropertyDescriptor(
      ArrayBuffer.prototype, 'resizable',
    )?.get;
    if (!resizable) return;
    const valid = canonical([record()]);
    const buffer = Reflect.construct(
      ArrayBuffer, [valid.byteLength, { maxByteLength: valid.byteLength + 16 }],
    ) as ArrayBuffer;
    const view = new Uint8Array(buffer);
    view.set(valid);

    expect(parsePostgresPublicAclFixtureV1(new Uint8Array(valid)).records)
      .toHaveLength(1);
    expect(Reflect.apply(resizable, buffer, [])).toBe(true);
    expectInvalid(view);
  });

  it.each([
    ['escaped ASCII duplicate key', '[{"a":1,"\\u0061":2}]'],
    ['escaped slash duplicate key', '[{"/":1,"\\/":2}]'],
    ['escaped astral duplicate key', '[{"𝄞":1,"\\ud834\\udd1e":2}]'],
    ['escaped NUL', '[{"objectName":"a\\u0000b"}]'],
    ['unpaired encoded high surrogate', '["\\ud800"]'],
    ['unpaired encoded low surrogate', '["\\udc00"]'],
  ])('rejects %s before JSON.parse', (_label, source) => {
    const parse = vi.spyOn(JSON, 'parse');
    try {
      expectInvalid(bytes(source));
      expect(parse).not.toHaveBeenCalled();
    } finally {
      parse.mockRestore();
    }
  });

  it.each([
    ['invalid UTF-8', Uint8Array.from([0x5b, 0xc3, 0x28, 0x5d])],
    ['UTF-8 BOM', Uint8Array.from([0xef, 0xbb, 0xbf, 0x5b, 0x5d])],
    ['literal NUL', bytes('["\u0000"]')],
  ])('rejects %s before JSON.parse', (_label, value) => {
    const parse = vi.spyOn(JSON, 'parse');
    try {
      expectInvalid(value);
      expect(parse).not.toHaveBeenCalled();
    } finally {
      parse.mockRestore();
    }
  });

  it('uses exactly one JSON.parse before semantic rejection', () => {
    const parse = vi.spyOn(JSON, 'parse');
    try {
      expectInvalid(bytes('[0]\n'));
      expect(parse).toHaveBeenCalledTimes(1);
    } finally {
      parse.mockRestore();
    }
  });

  it.each([
    ['missing final LF', fixtureText().slice(0, -1)],
    ['extra final LF', `${fixtureText()}\n`],
    ['CRLF', fixtureText().replace(/\n$/u, '\r\n')],
    ['leading whitespace', ` ${fixtureText()}`],
    ['trailing whitespace', `${fixtureText()} `],
  ])('rejects noncanonical %s bytes', (_label, source) => {
    expectInvalid(bytes(source));
  });

  it('accepts a raw slash and rejects its valid alternate escaped encoding', () => {
    const raw = record({ objectName: 'a/b' });
    const text = `${JSON.stringify([raw])}\n`;

    expect(parsePostgresPublicAclFixtureV1(bytes(text)).records[0]?.objectName)
      .toBe('a/b');
    expectInvalid(bytes(text.replace('a/b', 'a\\/b')));
  });

  it('accepts raw astral text and rejects its alternate escaped encoding', () => {
    const raw = record({ objectName: '𝄞' });
    const text = `${JSON.stringify([raw])}\n`;
    expect(parsePostgresPublicAclFixtureV1(bytes(text)).records[0]?.objectName)
      .toBe('𝄞');
    expectInvalid(bytes(text.replace('𝄞', '\\ud834\\udd1e')));
  });

  it.each(validKindRecords())('accepts the closed $objectClass/$objectKind kind', (value) => {
    expect(parsePostgresPublicAclFixtureV1(canonical([value])).records).toHaveLength(1);
  });

  it.each(validPrivilegeRecords())(
    'accepts $privilege for $objectClass/$objectKind',
    (value) => {
      expect(parsePostgresPublicAclFixtureV1(canonical([value])).records).toHaveLength(1);
    },
  );

  it.each([
    ['unknown class', record({ objectClass: 'database' })],
    ['cross-wired kind', record({ objectKind: 'language' })],
    ['cross-wired privilege', record({ privilege: 'EXECUTE' })],
    ['grantable atom', record({ grantable: true })],
    ['missing schema', record({ schemaName: null })],
    ['unexpected subobject', record({ subobjectName: 'column' })],
    ['unexpected routine arguments', record({ routineIdentityArguments: '' })],
    ['empty identifier', record({ objectName: '' })],
    ['overlong identifier', record({ objectName: 'a'.repeat(64) })],
  ])('rejects the semantic mutant %s', (_label, value) => {
    expectInvalid(canonical([value]));
  });

  it('rejects missing, extra, and reordered record members', () => {
    const missing = record();
    delete (missing as unknown as Record<string, unknown>).grantable;
    const extra = { ...record(), extra: false };
    const reordered = Object.fromEntries([...KEYS].reverse().map((key) => [
      key, record()[key as keyof PublicAclRecord],
    ]));

    for (const value of [missing, extra, reordered]) expectInvalid(canonical([value]));
  });

  it('rejects row reordering and duplicate records without sorting or deduplication', () => {
    const first = record({ objectName: 'a' });
    const second = record({ objectName: 'b' });

    expect(parsePostgresPublicAclFixtureV1(canonical([first, second])).records)
      .toHaveLength(2);
    expectInvalid(canonical([second, first]));
    expectInvalid(canonical([first, first]));
  });
});

interface PublicAclRecord {
  objectClass: string;
  schemaName: string | null;
  objectName: string;
  subobjectName: string | null;
  objectKind: string;
  routineIdentityArguments: string | null;
  privilege: string;
  grantable: boolean;
}

function bytes(value: string): Uint8Array {
  return new TextEncoder().encode(value);
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

function canonical(values: readonly unknown[]): Uint8Array {
  return bytes(`${JSON.stringify(values)}\n`);
}

function fixtureText(): string {
  return new TextDecoder().decode(FIXTURE);
}

function expectInvalid(value: unknown): void {
  expect(() => parsePostgresPublicAclFixtureV1(value)).toThrowError(
    new TypeError(INVALID),
  );
}

function record(overrides: Partial<PublicAclRecord> = {}): PublicAclRecord {
  return {
    objectClass: 'relation', schemaName: 'pg_catalog', objectName: 'example',
    subobjectName: null, objectKind: 'table', routineIdentityArguments: null,
    privilege: 'SELECT', grantable: false, ...overrides,
  };
}

function validKindRecords(): PublicAclRecord[] {
  const values: PublicAclRecord[] = [];
  const kinds: Record<string, readonly string[]> = {
    schema: ['schema'],
    relation: ['table', 'partitioned-table', 'view', 'materialized-view',
      'foreign-table', 'sequence'],
    column: ['table', 'partitioned-table', 'view', 'materialized-view', 'foreign-table'],
    routine: ['function', 'procedure', 'aggregate', 'window-function'],
    type: ['base', 'composite', 'domain', 'enum', 'pseudo', 'range', 'multirange', 'array'],
    language: ['language'],
    'foreign-data-wrapper': ['foreign-data-wrapper'],
    'foreign-server': ['foreign-server'],
  };
  for (const [objectClass, objectKinds] of Object.entries(kinds)) {
    for (const objectKind of objectKinds) values.push(recordFor(objectClass, objectKind));
  }
  return values;
}

function validPrivilegeRecords(): PublicAclRecord[] {
  return [
    ...['CREATE', 'USAGE'].map((privilege) => recordFor('schema', 'schema', privilege)),
    ...['SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE', 'REFERENCES', 'TRIGGER']
      .map((privilege) => recordFor('relation', 'table', privilege)),
    ...['SELECT', 'UPDATE', 'USAGE']
      .map((privilege) => recordFor('relation', 'sequence', privilege)),
    ...['INSERT', 'SELECT', 'UPDATE', 'REFERENCES']
      .map((privilege) => recordFor('column', 'table', privilege)),
    recordFor('routine', 'function', 'EXECUTE'),
    recordFor('type', 'base', 'USAGE'),
  ];
}

function recordFor(
  objectClass: string, objectKind: string, privilege?: string,
): PublicAclRecord {
  return record({
    objectClass,
    objectKind,
    privilege: privilege ?? defaultPrivilege(objectClass),
    schemaName: ['column', 'relation', 'routine', 'type'].includes(objectClass)
      ? 'pg_catalog' : null,
    subobjectName: objectClass === 'column' ? 'column_name' : null,
    routineIdentityArguments: objectClass === 'routine' ? '' : null,
  });
}

function defaultPrivilege(objectClass: string): string {
  if (objectClass === 'relation' || objectClass === 'column') return 'SELECT';
  if (objectClass === 'routine') return 'EXECUTE';
  return 'USAGE';
}

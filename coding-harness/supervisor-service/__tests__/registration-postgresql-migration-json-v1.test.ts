// SPDX-License-Identifier: MIT

import { Buffer } from 'node:buffer';
import { createHash } from 'node:crypto';
import { describe, expect, it, vi } from 'vitest';
import {
  POSTGRES_AUTHORITY_SEED_JSON_LIMITS_V1,
  POSTGRES_MIGRATION_MANIFEST_JSON_LIMITS_V1,
  POSTGRES_PROVISIONING_JSON_LIMITS_V1,
  type PostgresMigrationJsonKindV1,
  type PostgresMigrationJsonLimitsV1,
  exactOrderedMigrationJsonKeysV1,
  parsePostgresMigrationJsonBytesV1,
} from '../src/registration-postgresql-migration-json-v1.js';

const encoder = new TextEncoder();
const LIMIT_CASES = [
  ['provisioning', POSTGRES_PROVISIONING_JSON_LIMITS_V1],
  ['manifest', POSTGRES_MIGRATION_MANIFEST_JSON_LIMITS_V1],
  ['seed', POSTGRES_AUTHORITY_SEED_JSON_LIMITS_V1],
] as const;

function pretty(value: unknown): Uint8Array {
  return encoder.encode(`${JSON.stringify(value, null, 2)}\n`);
}

describe('PostgreSQL migration hostile JSON boundary V1', () => {
  it('pins the three compiled allocation ceilings', () => {
    expect(POSTGRES_PROVISIONING_JSON_LIMITS_V1).toEqual({
      maximumBytes: 65_536,
      maximumDepth: 8,
      maximumNodes: 2_048,
      maximumRecords: 512,
      maximumCollectionWidth: 64,
      maximumObjectKeys: 16,
      maximumStringBytes: 4_096,
    });
    expect(POSTGRES_MIGRATION_MANIFEST_JSON_LIMITS_V1).toEqual({
      maximumBytes: 16_384,
      maximumDepth: 5,
      maximumNodes: 256,
      maximumRecords: 16,
      maximumCollectionWidth: 2,
      maximumObjectKeys: 11,
      maximumStringBytes: 1_024,
    });
    expect(POSTGRES_AUTHORITY_SEED_JSON_LIMITS_V1).toEqual({
      maximumBytes: 262_144,
      maximumDepth: 4,
      maximumNodes: 56,
      maximumRecords: 3,
      maximumCollectionWidth: 0,
      maximumObjectKeys: 14,
      maximumStringBytes: 196_608,
    });
  });

  it.each(['provisioning', 'manifest', 'seed'] as const)(
    'accepts one exact %s pretty-JSON record and snapshots its digest', (kind) => {
      const bytes = pretty({ domain: `fixture-${kind}`, schemaVersion: 1 });
      const expectedDigest = createHash('sha256').update(bytes).digest('hex');
      const parsed = parsePostgresMigrationJsonBytesV1(bytes, kind);
      bytes.fill(0x78);
      expect(parsed.record).toEqual({ domain: `fixture-${kind}`, schemaVersion: 1 });
      expect(parsed.sha256).toBe(expectedDigest);
      expect(parsed.metrics).toEqual({ nodes: 3, records: 0, maximumDepth: 2 });
      expect(Object.isFrozen(parsed)).toBe(true);
      expect(Object.isFrozen(parsed.record)).toBe(true);
    },
  );

  it('accepts canonical Unicode escapes in values and decoded object keys', () => {
    const parsed = parsePostgresMigrationJsonBytesV1(
      pretty({ '\u0000': '\u0000', escaped: '\b\f\n\r\t' }),
      'manifest',
    );
    expect(parsed.record).toEqual({ '\u0000': '\u0000', escaped: '\b\f\n\r\t' });
  });

  it('counts every decoded byte across alternating Unicode escapes', () => {
    const exact = '\u0000x'.repeat(512);
    expect(parsePostgresMigrationJsonBytesV1(pretty({ a: exact }), 'manifest').record)
      .toEqual({ a: exact });
    expect(() => parsePostgresMigrationJsonBytesV1(
      pretty({ a: `${exact}\u0000` }), 'manifest',
    )).toThrow('PostgreSQL migration manifest JSON is invalid');
  });

  it.each([
    ['two-byte', '\u00e9'.repeat(512), `${'\u00e9'.repeat(512)}x`],
    ['three-byte', `${'\u20ac'.repeat(341)}x`, `${'\u20ac'.repeat(341)}xx`],
    ['four-byte', '\ud83d\ude00'.repeat(256), `${'\ud83d\ude00'.repeat(256)}x`],
  ] as const)('enforces the decoded UTF-8 ceiling for %s code points', (
    _label, exact, over,
  ) => {
    expect(parsePostgresMigrationJsonBytesV1(pretty({ a: exact }), 'manifest').record)
      .toEqual({ a: exact });
    expect(() => parsePostgresMigrationJsonBytesV1(pretty({ a: over }), 'manifest'))
      .toThrow('PostgreSQL migration manifest JSON is invalid');
  });

  it('rejects a runtime-invalid kind without reflecting caller-controlled text', () => {
    const invoke = parsePostgresMigrationJsonBytesV1 as unknown as
      (value: unknown, kind: unknown) => unknown;
    let message = '';
    try { invoke(pretty({ a: 1 }), 'attacker-secret'); }
    catch (error) { message = String(error); }
    expect(message).toBe('TypeError: PostgreSQL migration JSON kind is invalid');
    expect(message).not.toContain('attacker-secret');
  });

  it.each([
    ['compact', encoder.encode('{"a":1}\n')],
    ['missing final LF', encoder.encode('{\n  "a": 1\n}')],
    ['CRLF', encoder.encode('{\r\n  "a": 1\r\n}\r\n')],
    ['BOM', Uint8Array.from([0xef, 0xbb, 0xbf, ...pretty({ a: 1 })])],
    ['NUL', Uint8Array.from([...pretty({ a: 1 }), 0])],
    ['trailing data', encoder.encode('{\n  "a": 1\n}\ntrue')],
    ['negative zero', encoder.encode('{\n  "a": -0\n}\n')],
    ['exponent', encoder.encode('{\n  "a": 1e0\n}\n')],
    ['fraction', encoder.encode('{\n  "a": 1.0\n}\n')],
    ['unsafe integer', encoder.encode('{\n  "a": 9007199254740992\n}\n')],
  ] as const)('rejects %s before a semantic parser runs', (_label, bytes) => {
    expect(() => parsePostgresMigrationJsonBytesV1(bytes, 'manifest'))
      .toThrow('PostgreSQL migration manifest JSON is invalid');
  });

  it.each([
    ['invalid UTF-8', Uint8Array.from([0x7b, 0x22, 0x61, 0x22, 0x3a, 0xc3, 0x7d])],
    ['lone low surrogate', encoder.encode('{\n  "a": "\\udc00"\n}\n')],
    ['lone high surrogate', encoder.encode('{\n  "a": "\\ud800"\n}\n')],
  ] as const)('rejects %s without echoing bytes', (_label, bytes) => {
    let message = '';
    try { parsePostgresMigrationJsonBytesV1(bytes, 'manifest'); }
    catch (error) { message = String(error); }
    expect(message).toBe('TypeError: PostgreSQL migration manifest JSON is invalid');
    expect(message).not.toContain('ud800');
  });

  it.each([
    ['literal duplicate', '{\n  "a": 1,\n  "a": 2\n}\n'],
    ['decoded duplicate', '{\n  "a": 1,\n  "\\u0061": 2\n}\n'],
    ['nested duplicate', '{\n  "a": {\n    "b": 1,\n    "\\u0062": 2\n  }\n}\n'],
  ] as const)('rejects %s keys before JSON.parse can erase them', (_label, text) => {
    const parser = vi.spyOn(JSON, 'parse');
    try {
      expect(() => parsePostgresMigrationJsonBytesV1(encoder.encode(text), 'manifest'))
        .toThrow('PostgreSQL migration manifest JSON is invalid');
      expect(parser).not.toHaveBeenCalled();
    } finally {
      parser.mockRestore();
    }
  });

  it.each(LIMIT_CASES)('enforces exact and plus-one %s byte ceilings', (kind, limits) => {
    const exact = exactByteFixture(kind, limits.maximumBytes);
    const over = exactByteFixture(kind, limits.maximumBytes + 1);
    expect(exact).toHaveLength(limits.maximumBytes);
    expect(over).toHaveLength(limits.maximumBytes + 1);
    expect(() => parsePostgresMigrationJsonBytesV1(exact, kind)).not.toThrow();
    expect(() => parsePostgresMigrationJsonBytesV1(over, kind))
      .toThrow(`PostgreSQL migration ${kind} JSON is invalid`);
  });

  it.each(LIMIT_CASES)('enforces exact and plus-one %s string ceilings', (kind, limits) => {
    expect(() => parsePostgresMigrationJsonBytesV1(
      pretty({ a: 'x'.repeat(limits.maximumStringBytes) }), kind,
    )).not.toThrow();
    expect(() => parsePostgresMigrationJsonBytesV1(
      pretty({ a: 'x'.repeat(limits.maximumStringBytes + 1) }), kind,
    )).toThrow(`PostgreSQL migration ${kind} JSON is invalid`);
  });

  it.each(LIMIT_CASES)('enforces exact and plus-one %s array widths', (kind, limits) => {
    expect(() => parsePostgresMigrationJsonBytesV1(pretty({ a: Array.from(
      { length: limits.maximumCollectionWidth }, (_, index) => index,
    ) }), kind)).not.toThrow();
    expect(() => parsePostgresMigrationJsonBytesV1(pretty({ a: Array.from(
      { length: limits.maximumCollectionWidth + 1 }, (_, index) => index,
    ) }), kind)).toThrow(`PostgreSQL migration ${kind} JSON is invalid`);
  });

  it.each(LIMIT_CASES)('enforces exact and plus-one %s object-key ceilings', (kind, limits) => {
    const fixture = (keys: number): JsonRecord => Object.fromEntries(
      Array.from({ length: keys }, (_, index) => [`k${index}`, index]),
    );
    expect(() => parsePostgresMigrationJsonBytesV1(
      pretty(fixture(limits.maximumObjectKeys)), kind,
    )).not.toThrow();
    expect(() => parsePostgresMigrationJsonBytesV1(
      pretty(fixture(limits.maximumObjectKeys + 1)), kind,
    )).toThrow(`PostgreSQL migration ${kind} JSON is invalid`);
  });

  it.each(LIMIT_CASES)('enforces exact and plus-one %s depth ceilings', (kind, limits) => {
    expect(parsePostgresMigrationJsonBytesV1(
      pretty(depthFixture(limits.maximumDepth)), kind,
    ).metrics.maximumDepth).toBe(limits.maximumDepth);
    expect(() => parsePostgresMigrationJsonBytesV1(
      pretty(depthFixture(limits.maximumDepth + 1)), kind,
    )).toThrow(`PostgreSQL migration ${kind} JSON is invalid`);
  });

  it.each(LIMIT_CASES)('enforces exact and plus-one %s record ceilings', (kind, limits) => {
    expect(parsePostgresMigrationJsonBytesV1(
      pretty(recordFixture(limits.maximumRecords, limits.maximumObjectKeys)), kind,
    ).metrics.records).toBe(limits.maximumRecords);
    expect(() => parsePostgresMigrationJsonBytesV1(
      pretty(recordFixture(limits.maximumRecords + 1, limits.maximumObjectKeys)), kind,
    )).toThrow(`PostgreSQL migration ${kind} JSON is invalid`);
  });

  it.each(LIMIT_CASES)('enforces exact and plus-one %s node ceilings', (kind, limits) => {
    expect(parsePostgresMigrationJsonBytesV1(
      pretty(nodeFixture(limits.maximumNodes, limits)), kind,
    ).metrics.nodes).toBe(limits.maximumNodes);
    expect(() => parsePostgresMigrationJsonBytesV1(
      pretty(nodeFixture(limits.maximumNodes + 1, limits)), kind,
    )).toThrow(`PostgreSQL migration ${kind} JSON is invalid`);
  });

  it('rejects nonintrinsic, resizable, shared, detached, and Proxy views without traps', () => {
    class Bytes extends Uint8Array {}
    let traps = 0;
    const proxy = new Proxy(pretty({ a: 1 }), {
      getPrototypeOf: () => { traps += 1; return Uint8Array.prototype; },
    });
    const candidates: unknown[] = [
      Buffer.from(pretty({ a: 1 })), new Bytes(pretty({ a: 1 })), proxy,
      sharedBytes(), detachedBytes(), pretty({ a: 1 }).buffer, 'bytes', null,
    ];
    const resizable = resizableBytes();
    if (resizable !== null) candidates.push(resizable);
    for (const value of candidates) {
      expect(() => parsePostgresMigrationJsonBytesV1(value, 'manifest')).toThrow(
        'PostgreSQL migration manifest JSON is invalid',
      );
    }
    expect(traps).toBe(0);
  });

  it('checks exact member order and rejects symbols/accessors without invoking them', () => {
    expect(() => exactOrderedMigrationJsonKeysV1(
      { second: 2, first: 1 }, ['first', 'second'], 'fixture',
    )).toThrow('fixture has invalid ordered keys');
    const symbol = { first: 1 } as Record<PropertyKey, unknown>;
    symbol[Symbol('hidden')] = 2;
    expect(() => exactOrderedMigrationJsonKeysV1(
      symbol as Record<string, unknown>, ['first'], 'fixture',
    )).toThrow('fixture has invalid ordered keys');
    let reads = 0;
    const accessor = {} as Record<string, unknown>;
    Object.defineProperty(accessor, 'first', {
      enumerable: true, get: () => { reads += 1; return 1; },
    });
    expect(() => exactOrderedMigrationJsonKeysV1(accessor, ['first'], 'fixture'))
      .toThrow('fixture has invalid ordered keys');
    expect(reads).toBe(0);
  });
});

function resizableBytes(): Uint8Array | null {
  try {
    const source = pretty({ a: 1 });
    const buffer = Reflect.construct(
      ArrayBuffer, [source.byteLength, { maxByteLength: source.byteLength + 64 }],
    ) as ArrayBuffer;
    if (Reflect.get(buffer, 'resizable') !== true) return null;
    const view = new Uint8Array(buffer);
    view.set(source);
    return view;
  } catch {
    return null;
  }
}

function sharedBytes(): Uint8Array {
  const source = pretty({ a: 1 });
  const view = new Uint8Array(new SharedArrayBuffer(source.byteLength));
  view.set(source);
  return view;
}

function detachedBytes(): Uint8Array {
  const view = pretty({ a: 1 });
  const buffer = view.buffer as ArrayBuffer;
  structuredClone(buffer, { transfer: [buffer] });
  return view;
}

type JsonRecord = Record<string, unknown>;

function exactByteFixture(kind: PostgresMigrationJsonKindV1, target: number): Uint8Array {
  let root: JsonRecord;
  let paddingRecord: JsonRecord;
  let paddingKey: string;
  let maximumStringBytes: number;
  if (kind === 'provisioning') {
    root = Object.fromEntries(Array.from({ length: 16 }, (_, index) => [
      `k${index}`, index < 15 ? 'x'.repeat(4_096) : '',
    ]));
    paddingRecord = root;
    paddingKey = 'k15';
    maximumStringBytes = 4_096;
  } else if (kind === 'manifest') {
    root = {};
    for (let index = 0; index < 8; index += 1) {
      root[`r${index}`] = {
        a: 'x'.repeat(1_024), b: index < 7 ? 'x'.repeat(1_024) : '',
      };
    }
    paddingRecord = root.r7 as JsonRecord;
    paddingKey = 'b';
    maximumStringBytes = 1_024;
  } else {
    root = { a: 'x'.repeat(196_608), b: '' };
    paddingRecord = root;
    paddingKey = 'b';
    maximumStringBytes = 196_608;
  }
  const padding = target - pretty(root).byteLength;
  if (padding < 0 || padding > maximumStringBytes) throw new TypeError('invalid fixture');
  paddingRecord[paddingKey] = 'x'.repeat(padding);
  const bytes = pretty(root);
  if (bytes.byteLength !== target) throw new TypeError('invalid fixture');
  return bytes;
}

function depthFixture(depth: number): JsonRecord {
  let value: unknown = 0;
  for (let current = 1; current < depth; current += 1) value = { a: value };
  return value as JsonRecord;
}

function recordFixture(records: number, maximumObjectKeys: number): JsonRecord {
  const root: JsonRecord = {};
  const queue = [root];
  for (let created = 0; created < records; created += 1) {
    const parent = queue.find((record) => Object.keys(record).length < maximumObjectKeys);
    if (!parent) throw new TypeError('invalid fixture');
    const child: JsonRecord = {};
    parent[`r${Object.keys(parent).length}`] = child;
    queue.push(child);
  }
  return root;
}

interface MutableContainer {
  readonly value: JsonRecord | unknown[];
  readonly depth: number;
}

function nodeFixture(nodes: number, limits: PostgresMigrationJsonLimitsV1): JsonRecord {
  const root: JsonRecord = {};
  const containers: MutableContainer[] = [{ value: root, depth: 1 }];
  let created = 1;
  let records = 0;
  while (created < nodes) {
    const parent = containers.find(({ value, depth }) => depth < limits.maximumDepth
      && (Array.isArray(value)
        ? value.length < limits.maximumCollectionWidth
        : Object.keys(value).length < limits.maximumObjectKeys));
    if (!parent) throw new TypeError('invalid fixture');
    let child: unknown = 0;
    if (records < limits.maximumRecords) {
      child = {};
      records += 1;
    } else if (limits.maximumCollectionWidth > 0) child = [];
    if (Array.isArray(parent.value)) parent.value.push(child);
    else parent.value[`n${Object.keys(parent.value).length}`] = child;
    if (child !== null && typeof child === 'object') {
      containers.push({ value: child as JsonRecord | unknown[], depth: parent.depth + 1 });
    }
    created += 1;
  }
  return root;
}

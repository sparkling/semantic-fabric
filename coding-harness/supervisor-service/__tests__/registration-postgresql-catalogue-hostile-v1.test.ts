// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import {
  assertPostgresCatalogueDigestV1,
  parsePostgresCatalogueContractV1,
} from '../src/registration-postgresql-catalogue-contract-v1.js';
import {
  scanPostgresCatalogueBytesV1,
} from '../src/registration-postgresql-catalogue-scanner-v1.js';
import {
  reconstructPostgresCatalogueShapeV1,
} from '../src/registration-postgresql-catalogue-shape-v1.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const contractBytes = readFileSync(resolve(root, 'migrations/catalog-contract-v1.json'));
const INVALID = 'PostgreSQL catalogue contract is invalid';
const CONTRACT_SHA256 = 'e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69';

function bytes(value: string): Uint8Array {
  return new TextEncoder().encode(value);
}

function expectScanInvalid(value: unknown): void {
  expect(() => scanPostgresCatalogueBytesV1(value)).toThrowError(
    new TypeError(INVALID),
  );
}

function expectContractInvalid(value: unknown): void {
  expect(() => parsePostgresCatalogueContractV1(value)).toThrowError(
    new TypeError(INVALID),
  );
}

describe('PostgreSQL catalogue hostile wire scan V1', () => {
  it('accepts the reviewed bytes with exact bounded scan metrics', () => {
    expect(scanPostgresCatalogueBytesV1(contractBytes)).toMatchObject({
      text: contractBytes.toString('utf8'),
      nodes: 9_125,
      records: 963,
      maximumDepth: 8,
    });
  });

  it.each([
    ['escaped ASCII', '{"a":1,"\\u0061":2}'],
    ['escaped control', '{"\\n":1,"\\u000a":2}'],
    ['escaped slash', '{"/":1,"\\/":2}'],
    ['escaped surrogate pair', '{"𝄞":1,"\\ud834\\udd1e":2}'],
  ])('rejects %s duplicate decoded keys before JSON.parse', (_label, source) => {
    const parse = vi.spyOn(JSON, 'parse');
    try {
      expectContractInvalid(bytes(source));
      expect(parse).not.toHaveBeenCalled();
    } finally {
      parse.mockRestore();
    }
  });

  it.each([
    ['invalid UTF-8', Uint8Array.from([0x7b, 0xc3, 0x28, 0x7d])],
    ['UTF-8 BOM', Uint8Array.from([0xef, 0xbb, 0xbf, 0x7b, 0x7d])],
    ['embedded NUL', bytes('{"x":"\u0000"}')],
    ['unpaired encoded high surrogate', bytes('{"x":"\\ud800"}')],
    ['unpaired encoded low surrogate', bytes('{"x":"\\udc00"}')],
  ])('rejects %s before semantic reconstruction', (_label, value) => {
    expectContractInvalid(value);
  });

  it.each([
    ['leading zero', '{"n":01}'],
    ['leading plus', '{"n":+1}'],
    ['missing integer', '{"n":.1}'],
    ['missing fractional digit', '{"n":1.}'],
    ['missing exponent digit', '{"n":1e}'],
    ['negative zero', '{"n":-0}'],
    ['non-integer', '{"n":1.1}'],
    ['unsafe integer', '{"n":9007199254740992}'],
    ['overlong token', `{"n":${'1'.repeat(65)}}`],
  ])('rejects %s number syntax or value', (_label, source) => {
    expectScanInvalid(bytes(source));
  });

  it('snapshots an exact byte carrier before later caller mutation', () => {
    const mutable = new Uint8Array(contractBytes);
    const scanned = scanPostgresCatalogueBytesV1(mutable);
    mutable.fill(0);

    expect(scanned.text).toBe(contractBytes.toString('utf8'));
    expect(Object.isFrozen(scanned)).toBe(true);
  });

  it('rejects proxy, exotic, shared, and non-Uint8Array public byte carriers', () => {
    class ExoticBytes extends Uint8Array {}
    const hostile = [
      new Proxy(new Uint8Array(contractBytes), {}),
      new ExoticBytes(contractBytes),
      new Uint16Array(4),
      new DataView(new ArrayBuffer(8)),
      new Uint8Array(new SharedArrayBuffer(8)),
      contractBytes.buffer,
      contractBytes.toString('utf8'),
    ];

    for (const value of hostile) expectContractInvalid(value);
  });
});

describe('PostgreSQL catalogue canonical wire and authority boundary V1', () => {
  it.each([
    ['CRLF', contractBytes.toString('utf8').replaceAll('\n', '\r\n')],
    ['missing final LF', contractBytes.toString('utf8').slice(0, -1)],
    ['extra final LF', `${contractBytes.toString('utf8')}\n`],
    ['leading whitespace', ` ${contractBytes.toString('utf8')}`],
    ['trailing whitespace', `${contractBytes.toString('utf8')} `],
    ['compact JSON', JSON.stringify(JSON.parse(contractBytes.toString('utf8')))],
    ['alternate slash escape', contractBytes.toString('utf8').replace(
      'semantic-fabric/programme-capture', 'semantic-fabric\\/programme-capture',
    )],
    ['alternate root-key escape', contractBytes.toString('utf8').replace(
      '"domain":', '"\\u0064omain":',
    )],
    ['alternate exponent number', contractBytes.toString('utf8').replace(
      '"schemaVersion": 1,', '"schemaVersion": 1e0,',
    )],
    ['alternate fractional number', contractBytes.toString('utf8').replace(
      '"schemaVersion": 1,', '"schemaVersion": 1.0,',
    )],
    ['reordered root keys', reorderFirstRootMembers(contractBytes.toString('utf8'))],
  ])('rejects %s rather than normalizing it', (_label, source) => {
    expectContractInvalid(bytes(source));
  });

  it('returns a branded, deeply immutable, explicitly non-authorizing handle', () => {
    const handle = parsePostgresCatalogueContractV1(contractBytes);
    const relations = handle.contract.relations as unknown[];

    expect(handle).toMatchObject({
      rawByteLength: 232_822,
      rawSha256: CONTRACT_SHA256,
      authority: 'none',
      readinessAuthorized: false,
    });
    expect(handle).not.toHaveProperty('migrationAuthorized');
    expect(handle).not.toHaveProperty('observationAuthorized');
    expect(handle).not.toHaveProperty('persistenceAuthorized');
    expect(Object.isFrozen(handle)).toBe(true);
    expect(Object.isFrozen(handle.scan)).toBe(true);
    expect(Object.isFrozen(handle.contract)).toBe(true);
    expect(Object.isFrozen(relations)).toBe(true);
    expect(Object.isFrozen(relations[0])).toBe(true);
    expect(() => { (relations[0] as Record<string, unknown>).name = 'mutated'; })
      .toThrow(TypeError);
    expect(() => assertPostgresCatalogueDigestV1(handle, CONTRACT_SHA256)).not.toThrow();
  });

  it('does not accept a structurally forged or proxy-bearing parsed handle', () => {
    const real = parsePostgresCatalogueContractV1(contractBytes);
    const forged = Object.freeze({ ...real });
    let trapCalls = 0;
    const proxy = new Proxy(real, {
      get() { trapCalls += 1; return undefined; },
    });

    expect(() => assertPostgresCatalogueDigestV1(forged, CONTRACT_SHA256))
      .toThrowError(new TypeError(INVALID));
    expect(() => assertPostgresCatalogueDigestV1(proxy, CONTRACT_SHA256))
      .toThrowError(new TypeError(INVALID));
    expect(trapCalls).toBe(0);
  });
});

describe('PostgreSQL catalogue reconstructed value defenses V1', () => {
  it('reconstructs the reviewed value as a recursively immutable plain graph', () => {
    const rebuilt = reconstructPostgresCatalogueShapeV1(
      JSON.parse(contractBytes.toString('utf8')),
    );
    const relations = rebuilt.relations as unknown[];

    expect(Object.isFrozen(rebuilt)).toBe(true);
    expect(Object.isFrozen(relations)).toBe(true);
    expect(Object.isFrozen(relations[0])).toBe(true);
    expect(() => { (relations[0] as Record<string, unknown>).name = 'mutated'; })
      .toThrow(TypeError);
  });

  it('rejects root and nested proxies without invoking their traps', () => {
    const rootTarget = JSON.parse(contractBytes.toString('utf8'));
    let trapCalls = 0;
    const rootProxy = new Proxy(rootTarget, {
      ownKeys() { trapCalls += 1; return Reflect.ownKeys(rootTarget); },
    });
    expect(() => reconstructPostgresCatalogueShapeV1(rootProxy)).toThrow(TypeError);

    const nested = JSON.parse(contractBytes.toString('utf8'));
    const firstRelation = nested.relations[0];
    nested.relations[0] = new Proxy(firstRelation, {
      get() { trapCalls += 1; return undefined; },
    });
    expect(() => reconstructPostgresCatalogueShapeV1(nested)).toThrow(TypeError);
    expect(trapCalls).toBe(0);
  });

  it('rejects accessors, symbols, exotic prototypes, and typed-array fields', () => {
    const mutations = [
      (value: any) => Object.defineProperty(value, 'domain', {
        enumerable: true, get: () => value.domain,
      }),
      (value: any) => { value[Symbol('hidden')] = false; },
      (value: any) => Object.setPrototypeOf(value, null),
      (value: any) => { value.relations = new Uint8Array(value.relations.length); },
    ];

    for (const mutate of mutations) {
      const value = JSON.parse(contractBytes.toString('utf8'));
      mutate(value);
      expect(() => reconstructPostgresCatalogueShapeV1(value)).toThrow(TypeError);
    }
  });
});

function reorderFirstRootMembers(source: string): string {
  const lines = source.split('\n');
  [lines[1], lines[2]] = [lines[2]!, lines[1]!];
  return lines.join('\n');
}

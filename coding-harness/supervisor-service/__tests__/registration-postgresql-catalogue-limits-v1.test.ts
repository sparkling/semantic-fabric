// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import { parsePostgresCatalogueContractV1 }
  from '../src/registration-postgresql-catalogue-contract-v1.js';
import {
  POSTGRES_CATALOGUE_LIMITS_V1,
  scanPostgresCatalogueBytesV1,
} from '../src/registration-postgresql-catalogue-scanner-v1.js';
import { reconstructPostgresCatalogueShapeV1 }
  from '../src/registration-postgresql-catalogue-shape-v1.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const contractText = readFileSync(
  resolve(root, 'migrations/catalog-contract-v1.json'), 'utf8',
);
const INVALID = 'PostgreSQL catalogue contract is invalid';

const ADR_LIMITS = Object.freeze({
  maximumBytes: 1_048_576,
  maximumDepth: 16,
  maximumNodes: 16_384,
  maximumRecords: 4_096,
  maximumCollectionWidth: 1_024,
  maximumObjectKeys: 32,
  maximumStringBytes: 196_608,
  maximumIdentifierBytes: 63,
});

type LimitName = keyof typeof ADR_LIMITS;

function bytes(value: string): Uint8Array {
  return new TextEncoder().encode(value);
}

function expectScanInvalid(value: Uint8Array): void {
  expect(() => scanPostgresCatalogueBytesV1(value)).toThrowError(
    new TypeError(INVALID),
  );
}

function expectContractInvalid(value: Uint8Array): void {
  expect(() => parsePostgresCatalogueContractV1(value)).toThrowError(
    new TypeError(INVALID),
  );
}

describe('PostgreSQL catalogue fixed parser ceilings V1', () => {
  it('pins the frozen compile-time ceilings to the literal ADR-0045 values', () => {
    expect(POSTGRES_CATALOGUE_LIMITS_V1).toEqual(ADR_LIMITS);
    expect(Object.isFrozen(POSTGRES_CATALOGUE_LIMITS_V1)).toBe(true);
  });

  it('accepts exactly maximumBytes and rejects maximumBytes plus one', () => {
    const exact = bytes(`{}${' '.repeat(1_048_574)}`);
    const excessive = bytes(`{}${' '.repeat(1_048_575)}`);

    expect(exact.byteLength).toBe(1_048_576);
    expect(scanPostgresCatalogueBytesV1(exact)).toMatchObject({
      nodes: 1, records: 0, maximumDepth: 1,
    });
    expect(excessive.byteLength).toBe(1_048_577);
    expectScanInvalid(excessive);
  });

  it('accepts exactly maximumDepth and rejects maximumDepth plus one', () => {
    const exact = bytes(nestedPrimitiveAtDepth(16));
    const excessive = bytes(nestedPrimitiveAtDepth(17));

    expect(scanPostgresCatalogueBytesV1(exact).maximumDepth).toBe(16);
    expectScanInvalid(excessive);
  });

  it('accepts exactly maximumNodes and rejects maximumNodes plus one', () => {
    const exact = scanPostgresCatalogueBytesV1(bytes(nodeTree(16_384)));

    expect(exact.nodes).toBe(16_384);
    expect(exact.records).toBe(0);
    expectScanInvalid(bytes(nodeTree(16_385)));
  });

  it('accepts exactly maximumRecords and rejects maximumRecords plus one', () => {
    const exact = scanPostgresCatalogueBytesV1(bytes(recordTree(4_096)));

    expect(exact.records).toBe(4_096);
    expect(exact.nodes).toBe(4_102);
    expectScanInvalid(bytes(recordTree(4_097)));
  });

  it('accepts exactly maximumCollectionWidth and rejects it plus one', () => {
    const exact = scanPostgresCatalogueBytesV1(bytes(
      `{"x":[${zeros(1_024)}]}`,
    ));

    expect(exact.nodes).toBe(1_026);
    expectScanInvalid(bytes(`{"x":[${zeros(1_025)}]}`));
  });

  it('accepts exactly maximumObjectKeys and rejects it plus one', () => {
    const exact = scanPostgresCatalogueBytesV1(bytes(objectWithKeys(32)));

    expect(exact.nodes).toBe(33);
    expectScanInvalid(bytes(objectWithKeys(33)));
  });

  it('counts decoded UTF-8 string bytes at exact maximum and plus one', () => {
    const exactText = 'é'.repeat(98_304);
    const exact = scanPostgresCatalogueBytesV1(bytes(`{"x":"${exactText}"}`));

    expect(bytes(exactText).byteLength).toBe(196_608);
    expect(exact.nodes).toBe(2);
    expectScanInvalid(bytes(`{"x":"${exactText}a"}`));
  });

  it('accepts a 63-byte ASCII identifier and rejects 64 bytes', () => {
    const exact = JSON.parse(contractText) as Record<string, unknown>;
    exact.schemaName = 'a'.repeat(63);
    expect(reconstructPostgresCatalogueShapeV1(exact).schemaName).toBe('a'.repeat(63));

    const excessive = JSON.parse(contractText) as Record<string, unknown>;
    excessive.schemaName = 'a'.repeat(64);
    expect(() => reconstructPostgresCatalogueShapeV1(excessive)).toThrow(TypeError);
  });
});

describe('PostgreSQL catalogue encoded limits cannot tune parser ceilings V1', () => {
  it.each((Object.entries(ADR_LIMITS) as [LimitName, number][]))(
    'rejects both lowered and raised encoded %s after the fixed scan',
    (name, expected) => {
      const parse = vi.spyOn(JSON, 'parse');
      try {
        expectContractInvalid(encodedLimit(name, expected - 1));
        expect(parse).toHaveBeenCalledTimes(1);
        parse.mockClear();

        expectContractInvalid(encodedLimit(name, expected + 1));
        expect(parse).toHaveBeenCalledTimes(1);
      } finally {
        parse.mockRestore();
      }
    },
  );
});

function nestedPrimitiveAtDepth(depth: number): string {
  const arrayCount = depth - 2;
  return `{"x":${'['.repeat(arrayCount)}0${']'.repeat(arrayCount)}}`;
}

function nodeTree(totalNodes: number): string {
  const innerArrays = 16;
  let primitives = totalNodes - 2 - innerArrays;
  const groups: string[] = [];
  for (let index = 0; index < innerArrays; index += 1) {
    const width = Math.min(1_024, primitives);
    groups.push(`[${zeros(width)}]`);
    primitives -= width;
  }
  if (primitives !== 0) throw new TypeError('test fixture node count is invalid');
  return `{"x":[${groups.join(',')}]}`;
}

function recordTree(records: number): string {
  const groups: string[] = [];
  let remaining = records;
  while (remaining > 0) {
    const width = Math.min(1_024, remaining);
    groups.push(`[${objects(width)}]`);
    remaining -= width;
  }
  return `{"x":[${groups.join(',')}]}`;
}

function zeros(count: number): string {
  return count === 0 ? '' : `${'0,'.repeat(count - 1)}0`;
}

function objects(count: number): string {
  return count === 0 ? '' : `${'{},'.repeat(count - 1)}{}`;
}

function objectWithKeys(count: number): string {
  return `{${Array.from({ length: count }, (_, index) => `"k${index}":0`).join(',')}}`;
}

function encodedLimit(name: LimitName, replacement: number): Uint8Array {
  const expected = ADR_LIMITS[name];
  const marker = `"${name}":${expected}`;
  const mutated = contractText.replace(marker, `"${name}":${replacement}`);
  if (mutated === contractText) throw new TypeError(`missing test marker ${name}`);
  return bytes(mutated);
}

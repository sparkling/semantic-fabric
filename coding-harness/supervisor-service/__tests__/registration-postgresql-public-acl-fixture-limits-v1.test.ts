// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  POSTGRES_PUBLIC_ACL_FIXTURE_LIMITS_V1,
  scanPostgresPublicAclFixtureBytesV1,
} from './registration-postgresql-public-acl-fixture-scanner-v1.js';

const INVALID = 'PostgreSQL PUBLIC ACL fixture is invalid';
const ADR_LIMITS = Object.freeze({
  maximumBytes: 1_048_576,
  maximumDepth: 3,
  maximumNodes: 65_536,
  maximumRecords: 8_192,
  maximumRootWidth: 8_192,
  maximumObjectKeys: 8,
  maximumStringBytes: 196_608,
});

describe('PostgreSQL PUBLIC ACL fixture scanner limits V1', () => {
  it('pins every compile-time ceiling to the literal ADR-0047 values', () => {
    expect(POSTGRES_PUBLIC_ACL_FIXTURE_LIMITS_V1).toEqual(ADR_LIMITS);
    expect(Object.isFrozen(POSTGRES_PUBLIC_ACL_FIXTURE_LIMITS_V1)).toBe(true);
  });

  it('accepts exactly maximumBytes and rejects maximumBytes plus one', () => {
    const exact = bytes(sixStrings(65_516));
    const excessive = bytes(sixStrings(65_517));

    expect(exact.byteLength).toBe(1_048_576);
    expect(scanPostgresPublicAclFixtureBytesV1(exact)).toMatchObject({
      nodes: 7, records: 0, maximumDepth: 2,
    });
    expect(excessive.byteLength).toBe(1_048_577);
    expectInvalid(excessive);
  });

  it('accepts exactly maximumDepth and rejects maximumDepth plus one', () => {
    expect(scanPostgresPublicAclFixtureBytesV1(bytes('[{"x":{}}]')).maximumDepth)
      .toBe(3);
    expectInvalid(bytes('[{"x":{"y":{}}}]'));
  });

  it('accepts exactly maximumNodes and rejects maximumNodes plus one', () => {
    const exact = scanPostgresPublicAclFixtureBytesV1(bytes(nodeTree(false)));

    expect(exact).toMatchObject({
      nodes: 65_536, records: 8_192, maximumDepth: 3,
    });
    expectInvalid(bytes(nodeTree(true)));
  });

  it('accepts exactly maximumRecords', () => {
    const exact = scanPostgresPublicAclFixtureBytesV1(bytes(
      `[${objects(8_192)}]`,
    ));

    expect(exact).toMatchObject({
      nodes: 8_193, records: 8_192, maximumDepth: 2,
    });
  });

  it('isolates maximumRecords plus one without exceeding root width', () => {
    const source = `[{"x":{}},${objects(8_191)}]`;
    expectInvalid(bytes(source));
  });

  it('accepts exactly maximumRootWidth and rejects it plus one', () => {
    const exact = scanPostgresPublicAclFixtureBytesV1(bytes(`[${zeros(8_192)}]`));

    expect(exact).toMatchObject({
      nodes: 8_193, records: 0, maximumDepth: 2,
    });
    expectInvalid(bytes(`[${zeros(8_193)}]`));
  });

  it('accepts exactly maximumObjectKeys and rejects it plus one', () => {
    const exact = scanPostgresPublicAclFixtureBytesV1(bytes(
      `[${objectWithKeys(8)}]`,
    ));

    expect(exact).toMatchObject({ nodes: 10, records: 1, maximumDepth: 3 });
    expectInvalid(bytes(`[${objectWithKeys(9)}]`));
  });

  it('counts exact and maximum-plus-one decoded ASCII string bytes', () => {
    expect(scanPostgresPublicAclFixtureBytesV1(bytes(
      `["${'a'.repeat(196_608)}"]`,
    )).nodes).toBe(2);
    expectInvalid(bytes(`["${'a'.repeat(196_609)}"]`));
  });

  it('counts exact and maximum-plus-one raw multibyte string bytes', () => {
    expect(scanPostgresPublicAclFixtureBytesV1(bytes(
      `["${'é'.repeat(98_304)}"]`,
    )).nodes).toBe(2);
    expectInvalid(bytes(`["${'é'.repeat(98_304)}a"]`));
  });

  it('counts exact and maximum-plus-one escaped Unicode string bytes', () => {
    expect(scanPostgresPublicAclFixtureBytesV1(bytes(
      `["${'\\u00e9'.repeat(98_304)}"]`,
    )).nodes).toBe(2);
    expectInvalid(bytes(`["${'\\u00e9'.repeat(98_304)}\\u0061"]`));
  });

  it('accepts both raw and escaped well-formed surrogate pairs at scan time', () => {
    expect(scanPostgresPublicAclFixtureBytesV1(bytes('["𝄞"]')).nodes).toBe(2);
    expect(scanPostgresPublicAclFixtureBytesV1(bytes('["\\ud834\\udd1e"]')).nodes)
      .toBe(2);
  });

  it('applies the decoded string ceiling independently to object keys', () => {
    expect(scanPostgresPublicAclFixtureBytesV1(bytes(
      `[{"${'k'.repeat(196_608)}":null}]`,
    )).nodes).toBe(3);
    expectInvalid(bytes(`[{"${'k'.repeat(196_609)}":null}]`));
  });
});

function bytes(value: string): Uint8Array {
  return new TextEncoder().encode(value);
}

function expectInvalid(value: Uint8Array): void {
  expect(() => scanPostgresPublicAclFixtureBytesV1(value)).toThrowError(
    new TypeError(INVALID),
  );
}

function sixStrings(lastLength: number): string {
  const values = [...Array.from({ length: 5 }, () => 'a'.repeat(196_608)),
    'a'.repeat(lastLength)];
  return `[${values.map((value) => `"${value}"`).join(',')}]\n`;
}

function nodeTree(excessive: boolean): string {
  const values = Array.from({ length: 8_192 }, (_, index) =>
    objectWithKeys(index === 8_191 ? (excessive ? 7 : 6) : 7));
  return `[${values.join(',')}]`;
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

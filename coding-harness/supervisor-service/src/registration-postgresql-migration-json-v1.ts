// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import {
  closedRecordV1,
  deepFreezeV1,
  rawSha256HexV1,
  snapshotBytesV1,
  snapshotClosedGraphV1,
} from './registration-postgresql-canonical-v1.js';

export interface PostgresMigrationJsonLimitsV1 {
  readonly maximumBytes: number;
  readonly maximumDepth: number;
  readonly maximumNodes: number;
  readonly maximumRecords: number;
  readonly maximumCollectionWidth: number;
  readonly maximumObjectKeys: number;
  readonly maximumStringBytes: number;
}

export const POSTGRES_PROVISIONING_JSON_LIMITS_V1 = limits({
  maximumBytes: 65_536,
  maximumDepth: 8,
  maximumNodes: 2_048,
  maximumRecords: 512,
  maximumCollectionWidth: 64,
  maximumObjectKeys: 16,
  maximumStringBytes: 4_096,
});

export const POSTGRES_MIGRATION_MANIFEST_JSON_LIMITS_V1 = limits({
  maximumBytes: 16_384,
  maximumDepth: 5,
  maximumNodes: 256,
  maximumRecords: 16,
  maximumCollectionWidth: 2,
  maximumObjectKeys: 11,
  maximumStringBytes: 1_024,
});

export const POSTGRES_AUTHORITY_SEED_JSON_LIMITS_V1 = limits({
  maximumBytes: 262_144,
  maximumDepth: 4,
  maximumNodes: 56,
  maximumRecords: 3,
  maximumCollectionWidth: 0,
  maximumObjectKeys: 14,
  maximumStringBytes: 196_608,
});

export type PostgresMigrationJsonKindV1 = 'provisioning' | 'manifest' | 'seed';

export interface ParsedPostgresMigrationJsonV1 {
  readonly record: Readonly<Record<string, unknown>>;
  readonly sha256: string;
  readonly metrics: Readonly<{
    nodes: number;
    records: number;
    maximumDepth: number;
  }>;
}

export function parsePostgresMigrationJsonBytesV1(
  value: unknown,
  kind: PostgresMigrationJsonKindV1,
): ParsedPostgresMigrationJsonV1 {
  if (kind !== 'provisioning' && kind !== 'manifest' && kind !== 'seed') {
    throw new TypeError('PostgreSQL migration JSON kind is invalid');
  }
  const label = `PostgreSQL migration ${kind} JSON`;
  try {
    const scanned = scan(value, limitsFor(kind));
    const parsed = JSON.parse(scanned.text) as unknown;
    const snapshot = snapshotClosedGraphV1(parsed, label);
    const record = closedRecordV1(snapshot, label);
    if (`${JSON.stringify(record, null, 2)}\n` !== scanned.text) throw new TypeError();
    return deepFreezeV1({
      record,
      sha256: rawSha256HexV1(scanned.text),
      metrics: Object.freeze({
        nodes: scanned.nodes,
        records: scanned.records,
        maximumDepth: scanned.maximumDepth,
      }),
    });
  } catch {
    throw new TypeError(`${label} is invalid`);
  }
}

export function exactOrderedMigrationJsonKeysV1(
  value: unknown,
  expected: readonly string[],
  label: string,
): asserts value is Record<string, unknown> {
  try {
    const record = closedRecordV1(value, label, expected.length);
    if (JSON.stringify(Object.keys(record)) !== JSON.stringify(expected)) throw new TypeError();
  } catch {
    throw new TypeError(`${label} has invalid ordered keys`);
  }
}

interface ScanResultV1 {
  readonly text: string;
  readonly nodes: number;
  readonly records: number;
  readonly maximumDepth: number;
}

function scan(value: unknown, bounds: PostgresMigrationJsonLimitsV1): ScanResultV1 {
  if (isProxy(value) || value === null || typeof value !== 'object'
    || Object.getPrototypeOf(value) !== Uint8Array.prototype) throw new TypeError();
  const bytes = snapshotBytesV1(value, 'PostgreSQL migration JSON', bounds.maximumBytes);
  if (bytes.length === 0) throw new TypeError();
  let text = '';
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
  catch { throw new TypeError(); }
  if (!equalBytes(bytes, new TextEncoder().encode(text))) throw new TypeError();
  const scanner = new Scanner(text, bounds);
  return Object.freeze({ text, ...scanner.scan() });
}

class Scanner {
  private offset = 0;
  private nodes = 0;
  private records = 0;
  private deepest = 0;

  constructor(
    private readonly source: string,
    private readonly bounds: PostgresMigrationJsonLimitsV1,
  ) {}

  scan(): Omit<ScanResultV1, 'text'> {
    this.space();
    if (this.peek() !== '{') throw new TypeError();
    this.value(1, true);
    this.space();
    if (this.offset !== this.source.length) throw new TypeError();
    return Object.freeze({
      nodes: this.nodes,
      records: this.records,
      maximumDepth: this.deepest,
    });
  }

  private value(depth: number, root = false): void {
    this.depth(depth);
    const token = this.peek();
    this.node();
    if (token === '{') this.object(depth, root);
    else if (token === '[') this.array(depth);
    else if (token === '"') this.string(false);
    else if (token === 't') this.literal('true');
    else if (token === 'f') this.literal('false');
    else if (token === 'n') this.literal('null');
    else this.number();
  }

  private object(depth: number, root: boolean): void {
    if (!root && ++this.records > this.bounds.maximumRecords) throw new TypeError();
    this.take('{');
    this.space();
    if (this.peek() === '}') { this.offset += 1; return; }
    const keys = new Set<string>();
    let width = 0;
    while (true) {
      if (this.peek() !== '"' || width >= this.bounds.maximumObjectKeys) {
        throw new TypeError();
      }
      const key = this.string(true);
      if (keys.has(key)) throw new TypeError();
      keys.add(key);
      width += 1;
      this.space();
      this.take(':');
      this.space();
      this.value(depth + 1);
      this.space();
      if (this.peek() === '}') { this.offset += 1; return; }
      this.take(',');
      this.space();
    }
  }

  private array(depth: number): void {
    this.take('[');
    this.space();
    if (this.peek() === ']') { this.offset += 1; return; }
    let width = 0;
    while (true) {
      if (++width > this.bounds.maximumCollectionWidth) throw new TypeError();
      this.value(depth + 1);
      this.space();
      if (this.peek() === ']') { this.offset += 1; return; }
      this.take(',');
      this.space();
    }
  }

  private string(returnDecoded: boolean): string {
    this.take('"');
    let decodedBytes = 0;
    const decoded = returnDecoded ? [] as string[] : undefined;
    while (this.offset < this.source.length) {
      const code = this.source.charCodeAt(this.offset);
      if (code === 0x22) { this.offset += 1; return decoded?.join('') ?? ''; }
      if (code < 0x20) throw new TypeError();
      if (code === 0x5c) {
        this.offset += 1;
        const escape = this.source[this.offset];
        if (escape === 'u') {
          const hex = this.source.slice(this.offset + 1, this.offset + 5);
          if (!/^[0-9A-Fa-f]{4}$/.test(hex)) throw new TypeError();
          const escaped = Number.parseInt(hex, 16);
          if (escaped >= 0xd800 && escaped <= 0xdbff) {
            const lowHex = this.source.slice(this.offset + 7, this.offset + 11);
            const low = Number.parseInt(lowHex, 16);
            if (this.source.slice(this.offset + 5, this.offset + 7) !== '\\u'
              || !/^[0-9A-Fa-f]{4}$/.test(lowHex) || low < 0xdc00 || low > 0xdfff) {
              throw new TypeError();
            }
            decodedBytes += 4;
            decoded?.push(String.fromCodePoint(
              0x10000 + ((escaped - 0xd800) << 10) + (low - 0xdc00),
            ));
            this.offset += 11;
          } else {
            if (escaped >= 0xdc00 && escaped <= 0xdfff) throw new TypeError();
            decodedBytes += utf8CodeUnitBytes(escaped);
            decoded?.push(String.fromCharCode(escaped));
            this.offset += 5;
          }
          if (decodedBytes > this.bounds.maximumStringBytes) throw new TypeError();
          continue;
        } else {
          if (!escape || !'"\\/bfnrt'.includes(escape)) throw new TypeError();
          decodedBytes += 1;
          decoded?.push(decodeSimpleEscape(escape));
        }
      } else if (code >= 0xd800 && code <= 0xdbff) {
        const low = this.source.charCodeAt(this.offset + 1);
        if (low < 0xdc00 || low > 0xdfff) throw new TypeError();
        decodedBytes += 4;
        decoded?.push(this.source.slice(this.offset, this.offset + 2));
        this.offset += 1;
      } else {
        if (code >= 0xdc00 && code <= 0xdfff) throw new TypeError();
        decodedBytes += utf8CodeUnitBytes(code);
        decoded?.push(String.fromCharCode(code));
      }
      this.offset += 1;
      if (decodedBytes > this.bounds.maximumStringBytes) throw new TypeError();
    }
    throw new TypeError();
  }

  private number(): void {
    const start = this.offset;
    if (this.peek() === '-') this.offset += 1;
    if (this.peek() === '0') this.offset += 1;
    else {
      if (!/[1-9]/.test(this.peek() ?? '')) throw new TypeError();
      while (/[0-9]/.test(this.peek() ?? '')) this.offset += 1;
    }
    if (this.peek() === '.') {
      this.offset += 1;
      if (!/[0-9]/.test(this.peek() ?? '')) throw new TypeError();
      while (/[0-9]/.test(this.peek() ?? '')) this.offset += 1;
    }
    if (this.peek() === 'e' || this.peek() === 'E') {
      this.offset += 1;
      if (this.peek() === '+' || this.peek() === '-') this.offset += 1;
      if (!/[0-9]/.test(this.peek() ?? '')) throw new TypeError();
      while (/[0-9]/.test(this.peek() ?? '')) this.offset += 1;
    }
    if (this.offset - start > 64) throw new TypeError();
    const value = Number(this.source.slice(start, this.offset));
    if (!Number.isSafeInteger(value) || Object.is(value, -0)) throw new TypeError();
  }

  private literal(expected: 'true' | 'false' | 'null'): void {
    if (!this.source.startsWith(expected, this.offset)) throw new TypeError();
    this.offset += expected.length;
  }

  private node(): void {
    if (++this.nodes > this.bounds.maximumNodes) throw new TypeError();
  }

  private depth(value: number): void {
    if (value > this.bounds.maximumDepth) throw new TypeError();
    this.deepest = Math.max(this.deepest, value);
  }

  private take(expected: string): void {
    if (this.source[this.offset] !== expected) throw new TypeError();
    this.offset += 1;
  }

  private peek(): string | undefined { return this.source[this.offset]; }

  private space(): void {
    while ([' ', '\t', '\r', '\n'].includes(this.source[this.offset] ?? '')) this.offset += 1;
  }
}

function limitsFor(kind: PostgresMigrationJsonKindV1): PostgresMigrationJsonLimitsV1 {
  if (kind === 'provisioning') return POSTGRES_PROVISIONING_JSON_LIMITS_V1;
  if (kind === 'manifest') return POSTGRES_MIGRATION_MANIFEST_JSON_LIMITS_V1;
  if (kind === 'seed') return POSTGRES_AUTHORITY_SEED_JSON_LIMITS_V1;
  throw new TypeError();
}

function limits(value: PostgresMigrationJsonLimitsV1): PostgresMigrationJsonLimitsV1 {
  return Object.freeze(value);
}

function utf8CodeUnitBytes(code: number): number {
  if (code <= 0x7f) return 1;
  if (code <= 0x7ff) return 2;
  return 3;
}

function decodeSimpleEscape(value: string): string {
  if (value === 'b') return '\b';
  if (value === 'f') return '\f';
  if (value === 'n') return '\n';
  if (value === 'r') return '\r';
  if (value === 't') return '\t';
  return value;
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length
    && left.every((byte, index) => byte === right[index]);
}

// SPDX-License-Identifier: MIT

import { snapshotBytesV1 } from './registration-postgresql-canonical-v1.js';

export const POSTGRES_CATALOGUE_LIMITS_V1 = Object.freeze({
  maximumBytes: 1_048_576,
  maximumDepth: 16,
  maximumNodes: 16_384,
  maximumRecords: 4_096,
  maximumCollectionWidth: 1_024,
  maximumObjectKeys: 32,
  maximumStringBytes: 196_608,
  maximumIdentifierBytes: 63,
});

export interface PostgresCatalogueScanV1 {
  readonly text: string;
  readonly nodes: number;
  readonly records: number;
  readonly maximumDepth: number;
}

const INVALID = 'PostgreSQL catalogue contract is invalid';

/** Scan all allocation ceilings and duplicate keys before JSON.parse. */
export function scanPostgresCatalogueBytesV1(value: unknown): PostgresCatalogueScanV1 {
  try {
    const bytes = snapshotBytesV1(
      value, 'PostgreSQL catalogue contract', POSTGRES_CATALOGUE_LIMITS_V1.maximumBytes,
    );
    if (bytes.length === 0) throw new TypeError();
    let text = '';
    try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
    catch { throw new TypeError(); }
    if (!equalBytes(bytes, new TextEncoder().encode(text))) throw new TypeError();
    const scanner = new Scanner(text);
    const metrics = scanner.scan();
    return Object.freeze({
      text,
      ...metrics,
    });
  } catch {
    throw new TypeError(INVALID);
  }
}

class Scanner {
  private offset = 0;
  private nodes = 0;
  private records = 0;
  private deepest = 0;

  constructor(private readonly source: string) {}

  scan(): Readonly<{ nodes: number; records: number; maximumDepth: number }> {
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
    if (token === '{') {
      this.node();
      this.object(depth, root);
      return;
    }
    if (token === '[') {
      this.node();
      this.array(depth);
      return;
    }
    this.node();
    if (token === '"') this.string(false);
    else if (token === 't') this.literal('true');
    else if (token === 'f') this.literal('false');
    else if (token === 'n') this.literal('null');
    else this.number();
  }

  private object(depth: number, root: boolean): void {
    if (!root && ++this.records > POSTGRES_CATALOGUE_LIMITS_V1.maximumRecords) {
      throw new TypeError();
    }
    this.take('{');
    this.space();
    if (this.peek() === '}') {
      this.offset += 1;
      return;
    }
    const keys = new Set<string>();
    let width = 0;
    while (true) {
      if (this.peek() !== '"') throw new TypeError();
      if (width >= POSTGRES_CATALOGUE_LIMITS_V1.maximumObjectKeys) throw new TypeError();
      const key = this.string(true);
      if (keys.has(key)) throw new TypeError();
      width += 1;
      keys.add(key);
      this.space();
      this.take(':');
      this.space();
      this.value(depth + 1);
      this.space();
      if (this.peek() === '}') {
        this.offset += 1;
        return;
      }
      this.take(',');
      this.space();
    }
  }

  private array(depth: number): void {
    this.take('[');
    this.space();
    if (this.peek() === ']') {
      this.offset += 1;
      return;
    }
    let width = 0;
    while (true) {
      if (++width > POSTGRES_CATALOGUE_LIMITS_V1.maximumCollectionWidth) {
        throw new TypeError();
      }
      this.value(depth + 1);
      this.space();
      if (this.peek() === ']') {
        this.offset += 1;
        return;
      }
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
      if (code === 0x22) {
        this.offset += 1;
        return decoded?.join('') ?? '';
      }
      if (code < 0x20) throw new TypeError();
      if (code === 0x5c) {
        this.offset += 1;
        const escape = this.source[this.offset];
        if (escape === 'u') {
          const hex = this.source.slice(this.offset + 1, this.offset + 5);
          if (!/^[0-9A-Fa-f]{4}$/.test(hex)) throw new TypeError();
          const escaped = Number.parseInt(hex, 16);
          if (escaped >= 0xd800 && escaped <= 0xdbff) {
            if (this.source.slice(this.offset + 5, this.offset + 7) !== '\\u') {
              throw new TypeError();
            }
            const lowHex = this.source.slice(this.offset + 7, this.offset + 11);
            const low = Number.parseInt(lowHex, 16);
            if (!/^[0-9A-Fa-f]{4}$/.test(lowHex) || low < 0xdc00 || low > 0xdfff) {
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
          if (decodedBytes > POSTGRES_CATALOGUE_LIMITS_V1.maximumStringBytes) {
            throw new TypeError();
          }
          continue;
        }
        if (!escape || !'"\\/bfnrt'.includes(escape)) throw new TypeError();
        decodedBytes += 1;
        decoded?.push(decodeSimpleEscape(escape));
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
      if (decodedBytes > POSTGRES_CATALOGUE_LIMITS_V1.maximumStringBytes) {
        throw new TypeError();
      }
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
    if (++this.nodes > POSTGRES_CATALOGUE_LIMITS_V1.maximumNodes) throw new TypeError();
  }

  private depth(value: number): void {
    if (value > POSTGRES_CATALOGUE_LIMITS_V1.maximumDepth) throw new TypeError();
    this.deepest = Math.max(this.deepest, value);
  }

  private take(expected: string): void {
    if (this.source[this.offset] !== expected) throw new TypeError();
    this.offset += 1;
  }

  private peek(): string | undefined {
    return this.source[this.offset];
  }

  private space(): void {
    while (isJsonWhitespace(this.source[this.offset])) this.offset += 1;
  }
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

function isJsonWhitespace(value: string | undefined): boolean {
  return value === ' ' || value === '\t' || value === '\r' || value === '\n';
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length
    && left.every((byte, index) => byte === right[index]);
}

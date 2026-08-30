// SPDX-License-Identifier: MIT

import { createHash, createPublicKey, verify as verifySignature } from 'node:crypto';
import { isProxy } from 'node:util/types';

export const POSTGRES_PROJECT_SCOPE_ROLE_V1 = 'sf_supervisor_project_scope_v1' as const;
export const MAX_UINT64_V1 = 18_446_744_073_709_551_615n;

const MAX_GRAPH_DEPTH = 32;
const MAX_GRAPH_NODES = 8_192;
const MAX_GRAPH_BYTES = 1_048_576;
const U8 = Uint8Array;
const U8_SET = Uint8Array.prototype.set;
const APPLY = Reflect.apply;
const TYPED_ARRAY = Object.getPrototypeOf(Uint8Array.prototype) as object;
const BUFFER_GETTER = Object.getOwnPropertyDescriptor(TYPED_ARRAY, 'buffer')?.get;
const OFFSET_GETTER = Object.getOwnPropertyDescriptor(TYPED_ARRAY, 'byteOffset')?.get;
const LENGTH_GETTER = Object.getOwnPropertyDescriptor(TYPED_ARRAY, 'byteLength')?.get;
const TAG_GETTER = Object.getOwnPropertyDescriptor(TYPED_ARRAY, Symbol.toStringTag)?.get;
const RESIZABLE_GETTER = Object.getOwnPropertyDescriptor(
  ArrayBuffer.prototype, 'resizable',
)?.get;
const ED25519_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

export function exactKeysV1(
  value: Record<string, unknown>, expected: readonly string[], label: string,
): void {
  const actual = Object.keys(value);
  if (actual.length !== expected.length || actual.some((key) => !expected.includes(key))) {
    throw new TypeError(`${label} has invalid keys`);
  }
}

export function closedRecordV1(
  value: unknown, label: string, maximumEntries = MAX_GRAPH_NODES,
): Record<string, unknown> {
  if (!Number.isSafeInteger(maximumEntries) || maximumEntries < 0) {
    throw new TypeError(`${label} has an invalid entry bound`);
  }
  if (isProxy(value) || value === null || typeof value !== 'object' || Array.isArray(value)
    || Object.getPrototypeOf(value) !== Object.prototype) {
    throw new TypeError(`${label} must be a plain record`);
  }
  const keys = Reflect.ownKeys(value);
  if (keys.length > maximumEntries) {
    throw new TypeError(`${label} is too deeply nested or large`);
  }
  if (keys.some((key) => typeof key !== 'string')) {
    throw new TypeError(`${label} must be a plain record`);
  }
  for (const key of keys) {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (!descriptor?.enumerable || !('value' in descriptor)) {
      throw new TypeError(`${label} must contain enumerable data fields only`);
    }
  }
  return value as Record<string, unknown>;
}

export function denseArrayV1(
  value: unknown, label: string, maximumEntries = MAX_GRAPH_NODES,
): unknown[] {
  if (!Number.isSafeInteger(maximumEntries) || maximumEntries < 0) {
    throw new TypeError(`${label} has an invalid entry bound`);
  }
  if (isProxy(value) || !Array.isArray(value)
    || Object.getPrototypeOf(value) !== Array.prototype) {
    throw new TypeError(`${label} must be a dense plain array`);
  }
  if (value.length > maximumEntries) {
    throw new TypeError(`${label} is too deeply nested or large`);
  }
  if (Reflect.ownKeys(value).length !== value.length + 1) {
    throw new TypeError(`${label} must be a dense plain array`);
  }
  for (let index = 0; index < value.length; index += 1) {
    const descriptor = Object.getOwnPropertyDescriptor(value, index);
    if (!descriptor?.enumerable || !('value' in descriptor)) {
      throw new TypeError(`${label} must contain enumerable data entries only`);
    }
  }
  return value;
}

/** Copy bytes using intrinsic typed-array accessors, without iteration or species hooks. */
export function snapshotBytesV1(
  value: unknown, label: string, maximumBytes: number, exactBytes?: number,
): Uint8Array {
  if (!Number.isSafeInteger(maximumBytes) || maximumBytes < 0
    || (exactBytes !== undefined && (!Number.isSafeInteger(exactBytes) || exactBytes < 0))
    || !BUFFER_GETTER || !OFFSET_GETTER || !LENGTH_GETTER || !TAG_GETTER) {
    throw new TypeError(`${label} has an invalid byte bound`);
  }
  try {
    if (isProxy(value) || APPLY(TAG_GETTER, value, []) !== 'Uint8Array') throw new TypeError();
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Uint8Array.prototype && prototype !== Buffer.prototype) {
      throw new TypeError();
    }
    const buffer = APPLY(BUFFER_GETTER, value, []);
    if (!(buffer instanceof ArrayBuffer)
      || (RESIZABLE_GETTER && APPLY(RESIZABLE_GETTER, buffer, []) === true)) {
      throw new TypeError();
    }
    const offset = APPLY(OFFSET_GETTER, value, []) as number;
    const length = APPLY(LENGTH_GETTER, value, []) as number;
    if (length > maximumBytes || (exactBytes !== undefined && length !== exactBytes)) {
      throw new TypeError();
    }
    const source = new U8(buffer, offset, length);
    const copy = new U8(length);
    APPLY(U8_SET, copy, [source]);
    return copy;
  } catch {
    throw new TypeError(`${label} must be an exact bounded Uint8Array`);
  }
}

/** Snapshot a bounded data graph; Uint8Array leaves are independently copied. */
export function snapshotClosedGraphV1<T>(value: T, label: string): T {
  let nodes = 0;
  let bytes = 0;
  const active = new Set<object>();
  const visit = (current: unknown, depth: number): unknown => {
    if (depth > MAX_GRAPH_DEPTH || ++nodes > MAX_GRAPH_NODES) {
      throw new TypeError(`${label} is too deeply nested or large`);
    }
    if (current === null || typeof current === 'string' || typeof current === 'boolean') {
      return current;
    }
    if (typeof current === 'number') {
      if (!Number.isSafeInteger(current)) throw new TypeError(`${label} has an unsafe number`);
      return current;
    }
    if (typeof current !== 'object' || isProxy(current) || active.has(current)) {
      throw new TypeError(`${label} must be an acyclic data graph`);
    }
    let isByteArray = false;
    try { isByteArray = APPLY(TAG_GETTER!, current, []) === 'Uint8Array'; }
    catch { /* records and arrays are not typed arrays */ }
    if (isByteArray) {
      const copy = snapshotBytesV1(current, label, MAX_GRAPH_BYTES - bytes);
      bytes += copy.byteLength;
      return copy;
    }
    active.add(current);
    let output: unknown;
    const remainingNodes = MAX_GRAPH_NODES - nodes;
    if (Array.isArray(current)) {
      const entries = denseArrayV1(current, label, remainingNodes);
      output = entries.map((entry) => visit(entry, depth + 1));
    } else {
      const record = closedRecordV1(current, label, remainingNodes);
      const copy: Record<string, unknown> = {};
      for (const key of Object.keys(record)) copy[key] = visit(record[key], depth + 1);
      output = copy;
    }
    active.delete(current);
    return deepFreezeV1(output);
  };
  return visit(value, 0) as T;
}

export function deepFreezeV1<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !(value instanceof Uint8Array)
    && !Object.isFrozen(value)) {
    for (const nested of Object.values(value as Record<string, unknown>)) deepFreezeV1(nested);
    Object.freeze(value);
  }
  return value;
}

export function parseDigestV1(value: unknown, label: string): string {
  if (typeof value !== 'string' || !/^[a-f0-9]{64}$/.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

export function digestHexFromBytesV1(value: unknown, label: string): string {
  const bytes = snapshotBytesV1(value, label, 32, 32);
  return parseDigestV1(Buffer.from(bytes).toString('hex'), label);
}

export function digestBytesFromHexV1(value: unknown, label: string): Uint8Array {
  return new Uint8Array(Buffer.from(parseDigestV1(value, label), 'hex'));
}

export function parseOpaqueIdV1(value: unknown, label: string): string {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]{8,128}$/.test(value)) {
    throw new TypeError(`${label} must be a bounded opaque ID`);
  }
  return value;
}

export function parseUint64V1(value: unknown, label: string, minimum = 0n): string {
  if (typeof value !== 'string' || !/^(?:0|[1-9][0-9]{0,19})$/.test(value)
    || BigInt(value) < minimum || BigInt(value) > MAX_UINT64_V1) {
    throw new TypeError(`${label} must be a canonical uint64 decimal`);
  }
  return value;
}

export function successorUint64V1(value: unknown, label: string): string {
  const parsed = parseUint64V1(value, label);
  if (BigInt(parsed) === MAX_UINT64_V1) throw new TypeError(`${label} cannot advance`);
  return String(BigInt(parsed) + 1n);
}

export function canonicalJsonV1(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJsonV1).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    if (value instanceof Uint8Array) throw new TypeError('canonical JSON cannot contain bytes');
    const input = closedRecordV1(value, 'canonical JSON record');
    return `{${Object.keys(input).sort().map((key) =>
      `${JSON.stringify(key)}:${canonicalJsonV1(input[key])}`).join(',')}}`;
  }
  const encoded = JSON.stringify(value);
  if (encoded === undefined) throw new TypeError('value is not canonical JSON');
  return encoded;
}

export function canonicalDigestHexV1(value: unknown): string {
  return rawSha256HexV1(canonicalJsonV1(value));
}

export function rawSha256HexV1(value: string | Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

export function rawSha256BytesV1(value: string | Uint8Array): Uint8Array {
  return new Uint8Array(createHash('sha256').update(value).digest());
}

export function utf8BytesV1(value: string, label: string, maximumBytes: number): Uint8Array {
  if (typeof value !== 'string') throw new TypeError(`${label} must be text`);
  const bytes = new TextEncoder().encode(value);
  let decoded = '';
  try { decoded = new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
  catch { throw new TypeError(`${label} must be canonical UTF-8`); }
  if (bytes.length === 0 || bytes.length > maximumBytes || decoded !== value) {
    throw new TypeError(`${label} has invalid byte bounds`);
  }
  return bytes;
}

export function utf8TextV1(value: unknown, label: string, maximumBytes: number): string {
  const bytes = snapshotBytesV1(value, label, maximumBytes);
  if (bytes.length === 0) throw new TypeError(`${label} must not be empty`);
  let text = '';
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
  catch { throw new TypeError(`${label} must be canonical UTF-8`); }
  if (!equalBytesV1(bytes, new TextEncoder().encode(text))) {
    throw new TypeError(`${label} must be byte-identical UTF-8`);
  }
  return text;
}

export function parseCanonicalPrettyJsonBytesV1(
  value: unknown, label: string, maximumBytes: number,
): Record<string, unknown> {
  const text = utf8TextV1(value, label, maximumBytes);
  let parsed: unknown;
  try { parsed = JSON.parse(text); }
  catch { throw new TypeError(`${label} must be JSON`); }
  const snapshot = snapshotClosedGraphV1(parsed, label);
  const record = closedRecordV1(snapshot, label);
  if (`${JSON.stringify(record, null, 2)}\n` !== text) {
    throw new TypeError(`${label} must be exact pretty canonical JSON plus LF`);
  }
  return record;
}

export function equalBytesV1(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

export function validateEd25519SpkiV1(
  value: unknown, expectedFingerprint: string,
): Uint8Array {
  const bytes = snapshotBytesV1(value, 'service Ed25519 SPKI', 44, 44);
  if (!Buffer.from(bytes.subarray(0, ED25519_PREFIX.length)).equals(ED25519_PREFIX)
    || rawSha256HexV1(bytes) !== parseDigestV1(expectedFingerprint, 'service key fingerprint')) {
    throw new TypeError('service Ed25519 SPKI is invalid');
  }
  let key;
  try { key = createPublicKey({ key: Buffer.from(bytes), format: 'der', type: 'spki' }); }
  catch { throw new TypeError('service Ed25519 SPKI is invalid'); }
  const canonical = key.export({ format: 'der', type: 'spki' }) as Buffer;
  if (key.asymmetricKeyType !== 'ed25519' || !equalBytesV1(bytes, canonical)) {
    throw new TypeError('service Ed25519 SPKI is invalid');
  }
  return bytes;
}

export function verifyEd25519V1(
  spki: Uint8Array, payload: Uint8Array, signature: Uint8Array,
): void {
  const key = createPublicKey({ key: Buffer.from(spki), format: 'der', type: 'spki' });
  if (!verifySignature(null, payload, key, signature)) {
    throw new Error('registration materializer signature is invalid');
  }
}

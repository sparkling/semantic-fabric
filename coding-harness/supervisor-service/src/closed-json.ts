// SPDX-License-Identifier: MIT

const MAX_GRAPH_DEPTH = 24;
const MAX_GRAPH_NODES = 2_048;

export class ClosedJsonHashError extends Error {
  constructor() {
    super('closed JSON SHA-256 service is unavailable');
    this.name = 'ClosedJsonHashError';
  }
}

export function assertExactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
  label: string,
): void {
  const actual = Object.keys(value);
  if (actual.length !== expected.length
    || actual.some((key) => !expected.includes(key))) {
    throw new TypeError(`${label} has invalid keys`);
  }
}

export function asRecord(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)
    || Object.getPrototypeOf(value) !== Object.prototype) {
    throw new TypeError(`${label} must be a plain record`);
  }
  return value as Record<string, unknown>;
}

export function cloneClosedRecord(value: unknown, label: string): Record<string, unknown> {
  assertDataGraph(value, label);
  let copy: unknown;
  try { copy = structuredClone(value); }
  catch { throw new TypeError(`${label} must not contain a Proxy or uncloneable value`); }
  assertDataGraph(copy, label);
  return deepFreeze(asRecord(copy, label));
}

export function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const nested of Object.values(value as Record<string, unknown>)) deepFreeze(nested);
    Object.freeze(value);
  }
  return value;
}

export function parseCanonicalPrettyJson(
  serialized: string,
  maxBytes: number,
  label: string,
): Record<string, unknown> {
  if (typeof serialized !== 'string') throw new TypeError(`${label} must be text`);
  if (serialized.length === 0 || serialized.length > maxBytes) {
    throw new TypeError(`${label} byte bounds are invalid`);
  }
  const encoded = new TextEncoder().encode(serialized);
  let decoded = '';
  try { decoded = new TextDecoder('utf-8', { fatal: true }).decode(encoded); }
  catch { throw new TypeError(`${label} must be canonical UTF-8`); }
  if (encoded.byteLength > maxBytes || decoded !== serialized) {
    throw new TypeError(`${label} byte bounds are invalid`);
  }
  let parsed: unknown;
  try { parsed = JSON.parse(serialized); }
  catch { throw new TypeError(`${label} must be JSON`); }
  const record = asRecord(parsed, label);
  if (`${JSON.stringify(record, null, 2)}\n` !== serialized) {
    throw new TypeError(`${label} must be exact pretty canonical JSON plus LF`);
  }
  assertDataGraph(record, label);
  return deepFreeze(record);
}

export function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !/^[a-f0-9]{64}$/.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

export function parseOpaqueId(value: unknown, label: string): string {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]{8,128}$/.test(value)) {
    throw new TypeError(`${label} must be a bounded opaque ID`);
  }
  return value;
}

export function parseUint64(value: unknown, label: string): string {
  if (typeof value !== 'string' || !/^(?:0|[1-9][0-9]{0,19})$/.test(value)) {
    throw new TypeError(`${label} must be a canonical uint64 decimal`);
  }
  const maximum = '18446744073709551615';
  if (value.length > maximum.length
    || (value.length === maximum.length && value > maximum)) {
    throw new TypeError(`${label} must be a canonical uint64 decimal`);
  }
  return value;
}

export async function sha256Text(value: string): Promise<string> {
  try {
    const bytes = new TextEncoder().encode(value);
    const digest = await globalThis.crypto.subtle.digest('SHA-256', bytes);
    return [...new Uint8Array(digest)]
      .map((byte) => byte.toString(16).padStart(2, '0')).join('');
  } catch { throw new ClosedJsonHashError(); }
}

export async function sha256CanonicalValue(value: unknown): Promise<string> {
  return sha256Text(canonical(value));
}

function canonical(value: unknown): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  const record = value as Record<string, unknown>;
  return `{${Object.keys(record).sort().map(
    (key) => `${JSON.stringify(key)}:${canonical(record[key])}`,
  ).join(',')}}`;
}

function assertDataGraph(value: unknown, label: string): void {
  let nodes = 0;
  const seen = new Set<object>();
  const visit = (current: unknown, depth: number): void => {
    if (depth > MAX_GRAPH_DEPTH || ++nodes > MAX_GRAPH_NODES) {
      throw new TypeError(`${label} is too deeply nested or large`);
    }
    if (current === null || typeof current === 'string' || typeof current === 'boolean') return;
    if (typeof current === 'number') {
      if (!Number.isSafeInteger(current)) throw new TypeError(`${label} contains an unsafe number`);
      return;
    }
    if (typeof current !== 'object' || Array.isArray(current) || seen.has(current)) {
      throw new TypeError(`${label} must be an acyclic record graph`);
    }
    seen.add(current);
    if (Object.getPrototypeOf(current) !== Object.prototype
      || Object.getOwnPropertySymbols(current).length !== 0) {
      throw new TypeError(`${label} contains a non-plain record`);
    }
    const descriptors = Object.getOwnPropertyDescriptors(current);
    for (const descriptor of Object.values(descriptors)) {
      if (!descriptor.enumerable || !('value' in descriptor)) {
        throw new TypeError(`${label} contains a hidden field or accessor`);
      }
      visit(descriptor.value, depth + 1);
    }
    seen.delete(current);
  };
  visit(value, 0);
}

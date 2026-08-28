// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { closeSync, constants, fstatSync, openSync, readSync } from 'node:fs';
import {
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import { digestValue } from './receipts.js';

export const PROGRAMME_CAPTURE_HOST_FIELD_NAMES_V1 = [
  'kernelRelease', 'cpuModel', 'microcode', 'onlineCpus', 'allowedCpus',
  'isolatedCpus', 'scalingGovernor', 'turbo', 'swapTotalKib', 'memTotalKib',
  'load1Milli', 'numaAllowed', 'cgroup',
] as const;
export type ProgrammeCaptureHostFieldNameV1 =
  typeof PROGRAMME_CAPTURE_HOST_FIELD_NAMES_V1[number];
const ERROR_CODES = ['denied', 'invalid', 'missing', 'oversize', 'unstable'] as const;
type SurfaceError = typeof ERROR_CODES[number];

export type ProgrammeCaptureHostObservedFieldV1 = Readonly<{
  status: 'observed'; value: string; sourceDigest: string;
}> | Readonly<{ status: 'unavailable'; error: SurfaceError }>;

export interface ProgrammeCaptureHostSnapshotV1 {
  readonly fields: Readonly<Record<
    ProgrammeCaptureHostFieldNameV1,
    ProgrammeCaptureHostObservedFieldV1
  >>;
  readonly controlDigest: string;
  readonly snapshotDigest: string;
}

export type ProgrammeCaptureHostObserverKindV1 =
  | 'controller-read-only-proc-sysfs-v1'
  | 'diagnostic-injected-surface-v1';

export interface ProgrammeCaptureHostObservationV1 {
  readonly schemaVersion: 1;
  readonly evidenceKind: 'programme-capture-host-observation-v1';
  readonly observerKind: ProgrammeCaptureHostObserverKindV1;
  readonly platform: string;
  readonly architecture: string;
  readonly startedAt: string;
  readonly completedAt: string;
  readonly samples: readonly [ProgrammeCaptureHostSnapshotV1, ProgrammeCaptureHostSnapshotV1];
  readonly observationDigest: string;
}

export interface ProgrammeCaptureHostSurfaceSourceV1 {
  readonly platform: string;
  readonly architecture: string;
  now(): string;
  read(path: string, maximumBytes: number): Uint8Array;
}

const FIXED_HOST_OBSERVATIONS_V1 = new WeakSet<object>();
const INTRINSIC_WEAK_SET_ADD = WeakSet.prototype.add;
const INTRINSIC_WEAK_SET_HAS = WeakSet.prototype.has;
const INTRINSIC_REFLECT_APPLY = Reflect.apply;

export function collectProgrammeCaptureHostObservationV1(
): ProgrammeCaptureHostObservationV1 {
  const observation = collectFromSource(systemSource(), 'controller-read-only-proc-sysfs-v1');
  INTRINSIC_REFLECT_APPLY(INTRINSIC_WEAK_SET_ADD, FIXED_HOST_OBSERVATIONS_V1, [observation]);
  return observation;
}

export function collectProgrammeCaptureHostDiagnosticObservationV1(
  source: ProgrammeCaptureHostSurfaceSourceV1,
): ProgrammeCaptureHostObservationV1 {
  return collectFromSource(source, 'diagnostic-injected-surface-v1');
}

export function isFixedProgrammeCaptureHostObservationV1(value: unknown): boolean {
  return typeof value === 'object' && value !== null
    && INTRINSIC_REFLECT_APPLY(INTRINSIC_WEAK_SET_HAS, FIXED_HOST_OBSERVATIONS_V1, [value]);
}

function collectFromSource(
  source: ProgrammeCaptureHostSurfaceSourceV1,
  observerKind: ProgrammeCaptureHostObserverKindV1,
): ProgrammeCaptureHostObservationV1 {
  const startedAt = timestamp(source.now(), 'startedAt');
  const first = collectSnapshot(source), second = collectSnapshot(source);
  const completedAt = timestamp(source.now(), 'completedAt');
  if (completedAt < startedAt) throw new Error('HARNESS_CAPTURE_HOST_CLOCK_REVERSED');
  const body = {
    schemaVersion: 1 as const,
    evidenceKind: 'programme-capture-host-observation-v1' as const,
    observerKind,
    platform: boundedText(source.platform, 'platform', 64),
    architecture: boundedText(source.architecture, 'architecture', 64),
    startedAt,
    completedAt,
    samples: [first, second] as const,
  };
  return parseProgrammeCaptureHostObservationV1({
    ...body,
    observationDigest: digestValue(body),
  });
}

export function parseProgrammeCaptureHostObservationV1(
  value: unknown,
): ProgrammeCaptureHostObservationV1 {
  const input = asClosedRecord(value, 'programme capture host observation');
  assertExactKeys(input, [
    'schemaVersion', 'evidenceKind', 'observerKind', 'platform', 'architecture',
    'startedAt', 'completedAt', 'samples', 'observationDigest',
  ], 'programme capture host observation');
  if (input.schemaVersion !== 1
    || input.evidenceKind !== 'programme-capture-host-observation-v1'
    || (input.observerKind !== 'controller-read-only-proc-sysfs-v1'
      && input.observerKind !== 'diagnostic-injected-surface-v1')) {
    throw new TypeError('HARNESS_CAPTURE_HOST_OBSERVATION_IDENTITY_INVALID');
  }
  const samples = asDenseArray(input.samples, 'programme capture host observation.samples');
  if (samples.length !== 2) throw new TypeError('HARNESS_CAPTURE_HOST_SAMPLE_COUNT_INVALID');
  const body = {
    schemaVersion: 1 as const,
    evidenceKind: 'programme-capture-host-observation-v1' as const,
    observerKind: input.observerKind as ProgrammeCaptureHostObserverKindV1,
    platform: boundedText(input.platform, 'platform', 64),
    architecture: boundedText(input.architecture, 'architecture', 64),
    startedAt: timestamp(input.startedAt, 'startedAt'),
    completedAt: timestamp(input.completedAt, 'completedAt'),
    samples: samples.map(parseSnapshot) as unknown as readonly [
      ProgrammeCaptureHostSnapshotV1, ProgrammeCaptureHostSnapshotV1,
    ],
  };
  if (body.completedAt < body.startedAt) throw new Error('HARNESS_CAPTURE_HOST_CLOCK_REVERSED');
  const observationDigest = nonzeroDigest(input.observationDigest, 'host observation');
  if (observationDigest !== digestValue(body)) {
    throw new Error('HARNESS_CAPTURE_HOST_OBSERVATION_DIGEST_MISMATCH');
  }
  return deepFreeze({ ...body, observationDigest });
}

export function programmeCaptureHostObservedValuesV1(
  snapshot: ProgrammeCaptureHostSnapshotV1,
): Partial<Record<ProgrammeCaptureHostFieldNameV1, string>> {
  return Object.fromEntries(PROGRAMME_CAPTURE_HOST_FIELD_NAMES_V1.flatMap((name) => {
    const field = snapshot.fields[name];
    return field.status === 'observed' ? [[name, field.value]] : [];
  }));
}

export function parseProgrammeCaptureCpuListV1(
  value: string | undefined,
  allowEmpty = false,
): Set<number> {
  if (value === undefined || (value === '' && !allowEmpty)) throw new Error();
  const output = new Set<number>();
  if (value === '') return output;
  for (const part of value.split(',')) {
    const match = /^(0|[1-9][0-9]*)(?:-(0|[1-9][0-9]*))?$/.exec(part);
    if (match === null) throw new Error();
    const start = Number(match[1]), end = Number(match[2] ?? match[1]);
    if (start > end || end > 65_535) throw new Error();
    for (let cpu = start; cpu <= end; cpu += 1) {
      if (output.has(cpu)) throw new Error();
      output.add(cpu);
    }
  }
  return output;
}

function collectSnapshot(source: ProgrammeCaptureHostSurfaceSourceV1): ProgrammeCaptureHostSnapshotV1 {
  const status = readField(source, '/proc/self/status', 65_536);
  const cpuinfo = readField(source, '/proc/cpuinfo', 4 * 1024 * 1024);
  const meminfo = readField(source, '/proc/meminfo', 65_536);
  const allowed = derivedField(status, (text) => procField(text, 'Cpus_allowed_list'), true);
  const fields: Record<ProgrammeCaptureHostFieldNameV1, ProgrammeCaptureHostObservedFieldV1> = {
    kernelRelease: normalizedField(readField(source, '/proc/sys/kernel/osrelease', 65_536)),
    cpuModel: derivedField(cpuinfo, (text) => cpuInfoValues(text, 'model name', 'Hardware')),
    microcode: derivedField(cpuinfo, (text) => cpuInfoValues(text, 'microcode')),
    onlineCpus: normalizedField(readField(source, '/sys/devices/system/cpu/online', 65_536)),
    allowedCpus: allowed,
    isolatedCpus: normalizedField(readField(
      source, '/sys/devices/system/cpu/isolated', 65_536,
    ), true),
    scalingGovernor: governorField(source, allowed),
    turbo: turboField(source),
    swapTotalKib: derivedField(meminfo, (text) => meminfoValue(text, 'SwapTotal')),
    memTotalKib: derivedField(meminfo, (text) => meminfoValue(text, 'MemTotal')),
    load1Milli: derivedField(readField(source, '/proc/loadavg', 65_536), loadMilli),
    numaAllowed: derivedField(status, (text) => procField(text, 'Mems_allowed_list')),
    cgroup: normalizedField(readField(source, '/proc/self/cgroup', 65_536)),
  };
  const controlDigest = digestValue(controlProjection(fields));
  const body = { fields, controlDigest };
  return deepFreeze({ ...body, snapshotDigest: digestValue(body) });
}

interface RawField { readonly bytes?: Uint8Array; readonly error?: SurfaceError }

function readField(
  source: ProgrammeCaptureHostSurfaceSourceV1,
  path: string,
  maximumBytes: number,
): RawField {
  try {
    const bytes = source.read(path, maximumBytes);
    if (!(bytes instanceof Uint8Array) || bytes.byteLength > maximumBytes) {
      return { error: 'oversize' };
    }
    return { bytes };
  } catch (error) {
    const code = error !== null && typeof error === 'object' && 'code' in error
      ? String((error as { code?: unknown }).code) : '';
    if (code === 'ENOENT') return { error: 'missing' };
    if (code === 'EACCES' || code === 'EPERM') return { error: 'denied' };
    if (String((error as Error)?.message).includes('OVERSIZE')) return { error: 'oversize' };
    if (String((error as Error)?.message).includes('UNSTABLE')) return { error: 'unstable' };
    return { error: 'invalid' };
  }
}

function normalizedField(raw: RawField, allowEmpty = false): ProgrammeCaptureHostObservedFieldV1 {
  return derivedField(raw, (text) => {
    const value = normalize(text.trim());
    if (!allowEmpty && value === '') throw new Error();
    return value;
  }, allowEmpty);
}

function derivedField(
  raw: RawField,
  derive: (text: string) => string,
  allowEmpty = false,
): ProgrammeCaptureHostObservedFieldV1 {
  if (raw.error !== undefined || raw.bytes === undefined) {
    return { status: 'unavailable', error: raw.error ?? 'invalid' };
  }
  try {
    const text = new TextDecoder('utf-8', { fatal: true }).decode(raw.bytes);
    if (!Buffer.from(text, 'utf8').equals(Buffer.from(raw.bytes)) || text.includes('\0')) {
      throw new Error();
    }
    const value = derive(text);
    return {
      status: 'observed',
      value: boundedText(value, 'observed field', 4_096, allowEmpty),
      sourceDigest: createHash('sha256').update(raw.bytes).digest('hex'),
    };
  } catch {
    return { status: 'unavailable', error: 'invalid' };
  }
}

function governorField(
  source: ProgrammeCaptureHostSurfaceSourceV1,
  allowed: ProgrammeCaptureHostObservedFieldV1,
): ProgrammeCaptureHostObservedFieldV1 {
  if (allowed.status !== 'observed') return { status: 'unavailable', error: 'invalid' };
  let cpus: Set<number>;
  try { cpus = parseProgrammeCaptureCpuListV1(allowed.value); }
  catch { return { status: 'unavailable', error: 'invalid' }; }
  if (cpus.size === 0 || cpus.size > 4_096) return { status: 'unavailable', error: 'invalid' };
  const observations = [...cpus].map((cpu) => readField(
    source, `/sys/devices/system/cpu/cpu${cpu}/cpufreq/scaling_governor`, 64,
  ));
  if (observations.some(({ bytes }) => bytes === undefined)) return {
    status: 'unavailable',
    error: observations.find(({ error }) => error)?.error ?? 'invalid',
  };
  const values = observations.map((raw) => normalizedField(raw));
  if (values.some(({ status }) => status !== 'observed')) {
    return { status: 'unavailable', error: 'invalid' };
  }
  const governors = [...new Set(values.map((value) =>
    value.status === 'observed' ? value.value : ''))].sort(compareUtf8);
  return { status: 'observed', value: governors.join(','), sourceDigest: digestValue(values) };
}

function turboField(source: ProgrammeCaptureHostSurfaceSourceV1): ProgrammeCaptureHostObservedFieldV1 {
  const raw = [
    ['intel-no-turbo', readField(source, '/sys/devices/system/cpu/intel_pstate/no_turbo', 64)],
    ['cpufreq-boost', readField(source, '/sys/devices/system/cpu/cpufreq/boost', 64)],
  ] as const;
  const hardError = raw.find((entry) => entry[1].error !== undefined
    && entry[1].error !== 'missing');
  if (hardError !== undefined) {
    return { status: 'unavailable', error: hardError[1].error ?? 'invalid' };
  }
  const present = raw.filter((entry) => entry[1].bytes !== undefined);
  if (present.length === 0) return { status: 'unavailable', error: 'missing' };
  try {
    const states = present.map(([kind, field]) => {
      const value = new TextDecoder('utf-8', { fatal: true }).decode(field.bytes).trim();
      if (kind === 'intel-no-turbo') return value === '1' ? 'disabled' : value === '0' ? 'enabled' : fail();
      return value === '0' ? 'disabled' : value === '1' ? 'enabled' : fail();
    });
    return {
      status: 'observed',
      value: new Set(states).size === 1 ? states[0] as string : 'inconsistent',
      sourceDigest: digestValue(present.map(([kind, field]) => ({
        kind, sha256: createHash('sha256').update(field.bytes!).digest('hex'),
      }))),
    };
  } catch { return { status: 'unavailable', error: 'invalid' }; }
}

function parseSnapshot(value: unknown): ProgrammeCaptureHostSnapshotV1 {
  const input = asClosedRecord(value, 'programme capture host snapshot');
  assertExactKeys(
    input, ['fields', 'controlDigest', 'snapshotDigest'], 'programme capture host snapshot',
  );
  const raw = asClosedRecord(input.fields, 'programme capture host snapshot.fields');
  assertExactKeys(raw, PROGRAMME_CAPTURE_HOST_FIELD_NAMES_V1, 'programme capture host snapshot.fields');
  const fields = Object.fromEntries(PROGRAMME_CAPTURE_HOST_FIELD_NAMES_V1.map((name) => [
    name, parseObservedField(raw[name], name),
  ])) as unknown as ProgrammeCaptureHostSnapshotV1['fields'];
  const controlDigest = nonzeroDigest(input.controlDigest, 'host control');
  if (controlDigest !== digestValue(controlProjection(fields))) {
    throw new Error('HARNESS_CAPTURE_HOST_CONTROL_DIGEST_MISMATCH');
  }
  const body = { fields, controlDigest };
  const snapshotDigest = nonzeroDigest(input.snapshotDigest, 'host snapshot');
  if (snapshotDigest !== digestValue(body)) {
    throw new Error('HARNESS_CAPTURE_HOST_SNAPSHOT_DIGEST_MISMATCH');
  }
  return deepFreeze({ ...body, snapshotDigest });
}

function parseObservedField(value: unknown, label: string): ProgrammeCaptureHostObservedFieldV1 {
  const input = asClosedRecord(value, `programme capture host field ${label}`);
  if (input.status === 'unavailable') {
    assertExactKeys(input, ['status', 'error'], `programme capture host field ${label}`);
    if (!ERROR_CODES.includes(input.error as SurfaceError)) {
      throw new TypeError('HARNESS_CAPTURE_HOST_FIELD_ERROR_INVALID');
    }
    return deepFreeze({ status: 'unavailable', error: input.error as SurfaceError });
  }
  assertExactKeys(input, ['status', 'value', 'sourceDigest'], `programme capture host field ${label}`);
  if (input.status !== 'observed') throw new TypeError('HARNESS_CAPTURE_HOST_FIELD_STATUS_INVALID');
  return deepFreeze({
    status: 'observed',
    value: boundedText(
      input.value, label, 4_096, label === 'isolatedCpus' || label === 'allowedCpus',
    ),
    sourceDigest: nonzeroDigest(input.sourceDigest, `${label} source`),
  });
}

function procField(text: string, key: string): string {
  const prefix = `${key}:`, line = text.split('\n').find((entry) => entry.startsWith(prefix));
  if (line === undefined) throw new Error();
  return normalize(line.slice(prefix.length).trim());
}

function cpuInfoValues(text: string, ...keys: string[]): string {
  const values = new Set<string>();
  for (const line of text.split('\n')) {
    const separator = line.indexOf(':');
    if (separator < 0 || !keys.includes(line.slice(0, separator).trim())) continue;
    const value = normalize(line.slice(separator + 1).trim());
    if (value !== '') values.add(value);
  }
  if (values.size === 0) throw new Error();
  return [...values].sort(compareUtf8).join(' | ');
}

function meminfoValue(text: string, key: string): string {
  const value = procField(text, key).split(/\s+/);
  if (value.length !== 2 || value[1] !== 'kB' || !/^(?:0|[1-9][0-9]*)$/.test(value[0]!)) {
    throw new Error();
  }
  const number = Number(value[0]);
  if (!Number.isSafeInteger(number)) throw new Error();
  return String(number);
}

function loadMilli(text: string): string {
  const token = text.trim().split(/\s+/)[0] ?? '';
  const match = /^(0|[1-9][0-9]*)(?:\.([0-9]{1,3}))?$/.exec(token);
  if (match === null) throw new Error();
  const value = Number(match[1]) * 1_000 + Number((match[2] ?? '').padEnd(3, '0'));
  if (!Number.isSafeInteger(value)) throw new Error();
  return String(value);
}

function timestamp(value: unknown, label: string): string {
  if (typeof value !== 'string'
    || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(value)) {
    throw new TypeError(`HARNESS_CAPTURE_HOST_${label.toUpperCase()}_INVALID`);
  }
  try {
    if (new Date(value).toISOString() !== value) throw new Error();
  } catch {
    throw new TypeError(`HARNESS_CAPTURE_HOST_${label.toUpperCase()}_INVALID`);
  }
  return value;
}

function boundedText(
  value: unknown,
  label: string,
  maximum: number,
  allowEmpty = false,
): string {
  if (typeof value !== 'string' || (!allowEmpty && value === '')
    || Buffer.byteLength(value, 'utf8') > maximum
    || value.includes('\0') || value.includes('\r') || value.includes('\n')) {
    throw new TypeError(`HARNESS_CAPTURE_HOST_${label.toUpperCase()}_INVALID`);
  }
  return value;
}

function nonzeroDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`HARNESS_CAPTURE_HOST_${label.toUpperCase()}_DIGEST_INVALID`);
  }
  return value;
}

function normalize(value: string): string { return value.split(/\s+/).filter(Boolean).join(' '); }
function compareUtf8(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}
function fail(): never { throw new Error(); }

function systemSource(): ProgrammeCaptureHostSurfaceSourceV1 {
  return { platform: process.platform, architecture: rustArchitecture(process.arch),
    now: () => new Date().toISOString(), read: stableRead };
}

export function rustArchitecture(value: string): string {
  const names: Readonly<Record<string, string>> = {
    arm: 'arm', arm64: 'aarch64', ia32: 'x86', loong64: 'loongarch64',
    mips: 'mips', mipsel: 'mips', ppc: 'powerpc', ppc64: 'powerpc64',
    riscv64: 'riscv64', s390x: 's390x', x64: 'x86_64',
  };
  return names[value] ?? 'unmapped';
}

function controlProjection(
  fields: Readonly<Record<ProgrammeCaptureHostFieldNameV1, ProgrammeCaptureHostObservedFieldV1>>,
): unknown {
  return Object.fromEntries(PROGRAMME_CAPTURE_HOST_FIELD_NAMES_V1.map((name) => {
    const field = fields[name];
    return [name, field.status === 'observed'
      ? { status: field.status, value: field.value }
      : { status: field.status, error: field.error }];
  }));
}

function stableRead(path: string, maximumBytes: number): Uint8Array {
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true }), chunks: Buffer[] = [];
    if (!before.isFile() || before.isSymbolicLink()) throw new Error('UNSTABLE');
    let total = 0;
    while (total <= maximumBytes) {
      const chunk = Buffer.alloc(Math.min(65_536, maximumBytes + 1 - total));
      const count = readSync(descriptor, chunk, 0, chunk.length, null);
      if (count === 0) break;
      chunks.push(chunk.subarray(0, count)); total += count;
    }
    if (total > maximumBytes) throw new Error('OVERSIZE');
    if (!sameFile(before, fstatSync(descriptor, { bigint: true }))) throw new Error('UNSTABLE');
    return Buffer.concat(chunks, total);
  } finally { closeSync(descriptor); }
}

function sameFile(left: import('node:fs').BigIntStats, right: import('node:fs').BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.uid === right.uid && left.gid === right.gid;
}

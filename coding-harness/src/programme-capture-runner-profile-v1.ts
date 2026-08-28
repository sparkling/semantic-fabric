// SPDX-License-Identifier: MIT

import { deepFreeze, snapshotUint8Array } from './contracts.js';

const MAGIC = 'sf-performance-runner-profile-v1';
export const PROGRAMME_CAPTURE_RUNNER_PROFILE_MAX_BYTES_V1 = 65_536;

export interface ProgrammeCaptureRunnerProfileV1 {
  readonly profileId: string;
  readonly controlled: boolean;
  readonly os: string;
  readonly architecture: string;
  readonly kernelRelease: string;
  readonly cpuModel: string;
  readonly onlineCpus: string;
  readonly allowedCpus: string;
  readonly isolatedCpus: string;
  readonly scalingGovernor: string;
  readonly turbo: string;
  readonly swapTotalKib: number;
  readonly memTotalKib: number;
  readonly load1LimitMilli: number;
  readonly buildProfile: string;
}

export function parseProgrammeCaptureRunnerProfileV1(
  value: Uint8Array,
): ProgrammeCaptureRunnerProfileV1 {
  const bytes = snapshotUint8Array(
    value, 'programme capture runner profile', PROGRAMME_CAPTURE_RUNNER_PROFILE_MAX_BYTES_V1,
  );
  let text: string;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    throw new TypeError('programme capture runner profile is not UTF-8');
  }
  if (!Buffer.from(text, 'utf8').equals(Buffer.from(bytes))) {
    throw new TypeError('programme capture runner profile is not canonical UTF-8');
  }
  if (!text.endsWith('\n') || text.includes('\r')) {
    throw new TypeError('programme capture runner profile must use canonical LF termination');
  }
  const lines = text.slice(0, -1).split('\n');
  if (lines.length !== 16 || lines[0] !== MAGIC) {
    throw new TypeError('programme capture runner profile header or field count is invalid');
  }
  const profile = deepFreeze({
    profileId: field(lines, 1, 'profile-id'),
    controlled: booleanField(lines, 2, 'controlled'),
    os: field(lines, 3, 'os'),
    architecture: field(lines, 4, 'architecture'),
    kernelRelease: field(lines, 5, 'kernel-release'),
    cpuModel: field(lines, 6, 'cpu-model'),
    onlineCpus: field(lines, 7, 'online-cpus'),
    allowedCpus: field(lines, 8, 'allowed-cpus'),
    isolatedCpus: field(lines, 9, 'isolated-cpus'),
    scalingGovernor: field(lines, 10, 'scaling-governor'),
    turbo: field(lines, 11, 'turbo'),
    swapTotalKib: integerField(lines, 12, 'swap-total-kib'),
    memTotalKib: integerField(lines, 13, 'mem-total-kib'),
    load1LimitMilli: integerField(lines, 14, 'load1-limit-milli'),
    buildProfile: field(lines, 15, 'build-profile'),
  });
  if (renderProgrammeCaptureRunnerProfileV1(profile) !== text) {
    throw new TypeError('programme capture runner profile is not canonical');
  }
  return profile;
}

export function renderProgrammeCaptureRunnerProfileV1(
  profile: ProgrammeCaptureRunnerProfileV1,
): string {
  validateText('profile id', profile.profileId, true);
  for (const [label, value] of [
    ['os', profile.os],
    ['architecture', profile.architecture],
    ['kernel release', profile.kernelRelease],
    ['cpu model', profile.cpuModel],
    ['online CPUs', profile.onlineCpus],
    ['allowed CPUs', profile.allowedCpus],
    ['scaling governor', profile.scalingGovernor],
    ['turbo', profile.turbo],
    ['build profile', profile.buildProfile],
  ] as const) validateText(label, value, false);
  if (typeof profile.isolatedCpus !== 'string'
    || Buffer.byteLength(profile.isolatedCpus, 'utf8') > 512
    || profile.isolatedCpus.includes('\t')
    || profile.isolatedCpus.includes('\n')
    || profile.isolatedCpus.includes('\r')) {
    throw new TypeError('invalid isolated CPUs');
  }
  for (const [label, value] of [
    ['swap total KiB', profile.swapTotalKib],
    ['memory total KiB', profile.memTotalKib],
    ['load1 limit milli', profile.load1LimitMilli],
  ] as const) safeUnsignedInteger(value, label);
  if (typeof profile.controlled !== 'boolean') {
    throw new TypeError('invalid controlled');
  }
  const output = [
    MAGIC,
    `profile-id\t${profile.profileId}`,
    `controlled\t${profile.controlled}`,
    `os\t${profile.os}`,
    `architecture\t${profile.architecture}`,
    `kernel-release\t${profile.kernelRelease}`,
    `cpu-model\t${profile.cpuModel}`,
    `online-cpus\t${profile.onlineCpus}`,
    `allowed-cpus\t${profile.allowedCpus}`,
    `isolated-cpus\t${profile.isolatedCpus}`,
    `scaling-governor\t${profile.scalingGovernor}`,
    `turbo\t${profile.turbo}`,
    `swap-total-kib\t${profile.swapTotalKib}`,
    `mem-total-kib\t${profile.memTotalKib}`,
    `load1-limit-milli\t${profile.load1LimitMilli}`,
    `build-profile\t${profile.buildProfile}`,
    '',
  ].join('\n');
  if (Buffer.byteLength(output, 'utf8') > PROGRAMME_CAPTURE_RUNNER_PROFILE_MAX_BYTES_V1) {
    throw new TypeError('programme capture runner profile exceeds byte bound');
  }
  return output;
}

function field(lines: readonly string[], index: number, key: string): string {
  const parts = lines[index]?.split('\t') ?? [];
  if (parts.length !== 2 || parts[0] !== key) {
    throw new TypeError(`programme capture runner profile ${key} field is invalid`);
  }
  return parts[1] as string;
}

function booleanField(lines: readonly string[], index: number, key: string): boolean {
  const value = field(lines, index, key);
  if (value === 'true') return true;
  if (value === 'false') return false;
  throw new TypeError(`programme capture runner profile ${key} is invalid`);
}

function integerField(lines: readonly string[], index: number, key: string): number {
  const text = field(lines, index, key);
  if (!/^(?:0|[1-9][0-9]*)$/.test(text)) {
    throw new TypeError(`programme capture runner profile ${key} is invalid`);
  }
  const value = Number(text);
  safeUnsignedInteger(value, key);
  return value;
}

function safeUnsignedInteger(value: number, label: string): void {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new TypeError(`programme capture runner profile ${label} is invalid`);
  }
}

function validateText(label: string, value: string, identifier: boolean): void {
  const valid = typeof value === 'string'
    && value.length > 0
    && Buffer.byteLength(value, 'utf8') <= 512
    && !value.includes('\t')
    && !value.includes('\n')
    && !value.includes('\r')
    && (!identifier || /^[a-z0-9._-]+$/.test(value));
  if (!valid) throw new TypeError(`programme capture runner profile ${label} is invalid`);
}

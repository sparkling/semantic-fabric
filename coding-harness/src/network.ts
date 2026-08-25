// SPDX-License-Identifier: MIT

import { basename, isAbsolute, resolve } from 'node:path';
import type { HarnessConfig } from './contracts.js';
import {
  asNonEmptyString,
  asRecord,
  asUniqueStrings,
  assertExactKeys,
  deepFreeze,
  normalizePublicHttpsOrigin,
} from './contracts.js';
import { assertNativeSubscriptionEnvironment } from './models/environment.js';
import type { NativeAuthentication, NativeHost } from './models/types.js';
import { createSystemOfflineIsolator } from './sandbox.js';

export {
  createSystemOfflineIsolator,
  type ReadOnlyMount,
  type SystemOfflineIsolatorOptions,
} from './sandbox.js';

export interface BoundaryCommand {
  readonly executable: string;
  readonly args: readonly string[];
  readonly cwd: string;
  readonly env: Readonly<Record<string, string>>;
  readonly writablePaths: readonly string[];
}

export interface OfflineIsolationResult {
  readonly enforcement: 'os-network-namespace';
  readonly mechanism: string;
  readonly command: BoundaryCommand;
}

export interface OfflineProcessIsolator {
  isolate(command: BoundaryCommand): unknown;
  assertStable(): void;
}

export interface OfflineCandidateGrant extends OfflineIsolationResult {
  readonly mode: 'offline';
  readonly channel: 'candidate-command';
  readonly stage: 'candidate-execution';
  readonly allowedOrigins: readonly [];
}

export interface NativeModelNetworkGrant {
  readonly mode: 'first-party-model';
  readonly channel: 'native-subscription-client';
  readonly host: NativeHost;
  readonly authentication: NativeAuthentication;
  readonly provider: 'openai' | 'anthropic';
  readonly allowedOrigins: readonly string[];
  readonly authenticationEvidence: 'native-client-first-party-auth';
  readonly fallback: 'none';
}

export interface NativeModelProcessGrant extends NativeModelNetworkGrant, RegistryPinResult {
  readonly enforcement: 'origin-pinned-process-boundary';
}

export interface NativeModelOriginPinningBoundary {
  pin(command: BoundaryCommand, origins: readonly string[]): unknown;
}

export interface RegistryPinResult {
  readonly enforcement: 'origin-pinned-process-boundary';
  readonly mechanism: string;
  readonly pinnedOrigins: readonly string[];
  readonly command: BoundaryCommand;
}

export interface RegistryOriginPinningBoundary {
  pin(command: BoundaryCommand, origins: readonly string[]): unknown;
}

export interface DependencyResolutionGrant extends RegistryPinResult {
  readonly mode: 'dependency-resolution';
  readonly channel: 'dependency-registry';
  readonly stage: 'dependency-resolution';
  readonly registry: string;
  readonly allowedOrigins: readonly string[];
}

const NATIVE_PROFILES = Object.freeze({
  codex: Object.freeze({
    authentication: 'chatgpt-subscription',
    provider: 'openai',
    origins: Object.freeze(['https://api.openai.com', 'https://chatgpt.com']),
  }),
  'claude-code': Object.freeze({
    authentication: 'claude-subscription',
    provider: 'anthropic',
    origins: Object.freeze(['https://api.anthropic.com', 'https://claude.ai']),
  }),
} as const);

const KNOWN_NATIVE_ORIGINS = new Set<string>(
  Object.values(NATIVE_PROFILES).flatMap((profile) => profile.origins),
);

const NPM_CI_PREFIX = Object.freeze(['ci', '--ignore-scripts', '--registry'] as const);
const NPM_CI_SUFFIX = Object.freeze(['--audit=false', '--fund=false'] as const);
const ENVIRONMENT_NAME = /^[A-Z_][A-Z0-9_]*$/;
const MECHANISM_NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/;

export function isolateOfflineCandidateCommand(
  value: unknown,
  isolator: OfflineProcessIsolator,
): OfflineCandidateGrant {
  const input = asRecord(value, 'offline candidate request');
  assertExactKeys(
    input,
    ['mode', 'channel', 'stage', 'deterministic', 'allowedOrigins', 'command'],
    'offline candidate request',
  );
  if (input.mode !== 'offline') throw new TypeError('candidate execution must use offline mode');
  if (input.channel !== 'candidate-command') {
    throw new TypeError('offline mode is restricted to the candidate-command channel');
  }
  if (input.stage !== 'candidate-execution' || input.deterministic !== true) {
    throw new TypeError('candidate execution must be an explicit deterministic stage');
  }
  assertNoOrigins(input.allowedOrigins, 'offline candidate request.allowedOrigins');
  const command = parseBoundaryCommand(input.command, 'offline candidate request.command');
  assertNoRouteEnvironment(command.env, 'offline candidate request.command.env');

  if (isolator === undefined) throw new Error('HARNESS_OFFLINE_ISOLATOR_REQUIRED');
  isolator.assertStable();
  const isolated = parseOfflineIsolation(isolator.isolate(command));
  isolator.assertStable();
  assertBoundaryCommandBinding(
    command,
    isolated.command,
    true,
    'HARNESS_OFFLINE_ISOLATOR_DID_NOT_WRAP_COMMAND',
  );
  assertNoRouteEnvironment(isolated.command.env, 'offline isolated command.env');
  return deepFreeze({
    mode: 'offline',
    channel: 'candidate-command',
    stage: 'candidate-execution',
    allowedOrigins: [] as [],
    ...isolated,
  });
}

export function admitNativeFirstPartyModelTraffic(
  value: unknown,
  config: HarnessConfig,
): NativeModelNetworkGrant {
  const input = asRecord(value, 'native model network request');
  assertExactKeys(
    input,
    ['mode', 'channel', 'host', 'authentication', 'allowedOrigins', 'environment', 'transport'],
    'native model network request',
  );
  if (input.mode !== 'first-party-model') throw new TypeError('native model traffic requires first-party-model mode');
  if (input.channel !== 'native-subscription-client') {
    throw new TypeError('model traffic is restricted to the native-subscription-client channel');
  }
  const host = parseNativeHost(input.host);
  const profile = NATIVE_PROFILES[host];
  if (input.authentication !== profile.authentication) {
    throw new TypeError(`native authentication does not match ${host}`);
  }
  if (config.firstPartyOrigins.some((origin) => !KNOWN_NATIVE_ORIGINS.has(origin))) {
    throw new Error('HARNESS_UNRECOGNIZED_CONFIGURED_MODEL_ORIGIN');
  }
  const expectedOrigins = profile.origins.filter((origin) => config.firstPartyOrigins.includes(origin));
  if (expectedOrigins.length === 0) throw new Error(`HARNESS_${host.toUpperCase().replace('-', '_')}_ORIGINS_UNAVAILABLE`);
  const origins = parseOrigins(input.allowedOrigins, 'native model network request.allowedOrigins');
  assertSameOrigins(origins, expectedOrigins, 'native model traffic must use the exact configured host origins');

  const environment = parseEnvironment(input.environment, 'native model network request.environment');
  assertNativeSubscriptionEnvironment(host, environment);
  assertNativeTransport(input.transport, host);
  return deepFreeze({
    mode: 'first-party-model',
    channel: 'native-subscription-client',
    host,
    authentication: profile.authentication,
    provider: profile.provider,
    allowedOrigins: [...expectedOrigins],
    authenticationEvidence: 'native-client-first-party-auth',
    fallback: 'none',
  });
}

export function isolateNativeFirstPartyModelTraffic(
  value: unknown,
  config: HarnessConfig,
  boundary?: NativeModelOriginPinningBoundary,
): NativeModelProcessGrant {
  const input = asRecord(value, 'native model process request');
  assertExactKeys(
    input,
    [
      'mode', 'channel', 'host', 'authentication', 'allowedOrigins', 'environment',
      'transport', 'command',
    ],
    'native model process request',
  );
  const admission = admitNativeFirstPartyModelTraffic({
    mode: input.mode,
    channel: input.channel,
    host: input.host,
    authentication: input.authentication,
    allowedOrigins: input.allowedOrigins,
    environment: input.environment,
    transport: input.transport,
  }, config);
  const command = parseBoundaryCommand(input.command, 'native model process request.command');
  assertNoRouteEnvironment(command.env, 'native model process request.command.env');
  if (boundary === undefined) throw new Error('HARNESS_NATIVE_ORIGIN_BOUNDARY_REQUIRED');
  const pinned = parseRegistryPin(boundary.pin(command, admission.allowedOrigins));
  assertSameOrigins(
    pinned.pinnedOrigins,
    admission.allowedOrigins,
    'native origin-pinning boundary admitted an unexpected origin',
  );
  assertBoundaryCommandBinding(
    command,
    pinned.command,
    true,
    'HARNESS_NATIVE_ORIGIN_COMMAND_MISMATCH',
  );
  assertNoRouteEnvironment(pinned.command.env, 'native pinned command.env');
  return deepFreeze({ ...admission, ...pinned });
}

export function isolateDependencyResolution(
  value: unknown,
  config: HarnessConfig,
  boundary?: RegistryOriginPinningBoundary,
): DependencyResolutionGrant {
  const input = asRecord(value, 'dependency resolution request');
  assertExactKeys(
    input,
    ['mode', 'channel', 'stage', 'registry', 'allowedOrigins', 'command'],
    'dependency resolution request',
  );
  if (input.mode !== 'dependency-resolution') throw new TypeError('dependency resolution requires its own network mode');
  if (input.channel !== 'dependency-registry' || input.stage !== 'dependency-resolution') {
    throw new TypeError('dependency traffic is restricted to the explicit dependency-registry stage');
  }
  const registry = asNonEmptyString(input.registry, 'dependency resolution request.registry');
  if (registry !== config.approvedRegistry) throw new TypeError('dependency registry does not match the approved registry');
  const registryOrigin = new URL(config.approvedRegistry).origin;
  const origins = parseOrigins(input.allowedOrigins, 'dependency resolution request.allowedOrigins');
  assertSameOrigins(origins, [registryOrigin], 'dependency resolution must be registry-only');
  const command = parseBoundaryCommand(input.command, 'dependency resolution request.command');
  assertNoRouteEnvironment(command.env, 'dependency resolution request.command.env');
  assertDeterministicNpmCi(command, config.approvedRegistry);
  if (boundary === undefined) throw new Error('HARNESS_REGISTRY_ORIGIN_BOUNDARY_REQUIRED');

  const pinned = parseRegistryPin(boundary.pin(command, [registryOrigin]));
  assertSameOrigins(pinned.pinnedOrigins, [registryOrigin], 'origin-pinning boundary admitted an unexpected origin');
  assertBoundaryCommandBinding(
    command,
    pinned.command,
    true,
    'HARNESS_REGISTRY_ORIGIN_COMMAND_MISMATCH',
  );
  assertNoRouteEnvironment(pinned.command.env, 'dependency pinned command.env');
  return deepFreeze({
    mode: 'dependency-resolution',
    channel: 'dependency-registry',
    stage: 'dependency-resolution',
    registry,
    allowedOrigins: [registryOrigin],
    ...pinned,
  });
}

function parseOfflineIsolation(value: unknown): OfflineIsolationResult {
  const input = asRecord(value, 'offline isolation result');
  assertExactKeys(input, ['enforcement', 'mechanism', 'command'], 'offline isolation result');
  if (input.enforcement !== 'os-network-namespace') throw new Error('HARNESS_OFFLINE_ISOLATION_EVIDENCE_INVALID');
  const mechanism = parseMechanism(input.mechanism, 'offline isolation result.mechanism');
  return {
    enforcement: 'os-network-namespace',
    mechanism,
    command: parseBoundaryCommand(input.command, 'offline isolation result.command'),
  };
}

function parseRegistryPin(value: unknown): RegistryPinResult {
  const input = asRecord(value, 'registry pin result');
  assertExactKeys(input, ['enforcement', 'mechanism', 'pinnedOrigins', 'command'], 'registry pin result');
  if (input.enforcement !== 'origin-pinned-process-boundary') {
    throw new Error('HARNESS_REGISTRY_ORIGIN_EVIDENCE_INVALID');
  }
  return {
    enforcement: 'origin-pinned-process-boundary',
    mechanism: parseMechanism(input.mechanism, 'registry pin result.mechanism'),
    pinnedOrigins: parseOrigins(input.pinnedOrigins, 'registry pin result.pinnedOrigins'),
    command: parseBoundaryCommand(input.command, 'registry pin result.command'),
  };
}

export function parseBoundaryCommand(value: unknown, label: string): BoundaryCommand {
  const input = asRecord(value, label);
  assertExactKeys(input, ['executable', 'args', 'cwd', 'env', 'writablePaths'], label);
  const executable = byteString(input.executable, `${label}.executable`, false);
  if (!Array.isArray(input.args)) throw new TypeError(`${label}.args must be an array`);
  const args = input.args.map((entry, index) => byteString(entry, `${label}.args[${index}]`, true));
  const cwd = byteString(input.cwd, `${label}.cwd`, false);
  if (!isAbsolute(cwd) || resolve(cwd) !== cwd) throw new TypeError(`${label}.cwd must be an absolute normalized path`);
  if (!Array.isArray(input.writablePaths)) throw new TypeError(`${label}.writablePaths must be an array`);
  const writablePaths = input.writablePaths.map((entry, index) => {
    const path = byteString(entry, `${label}.writablePaths[${index}]`, false);
    if (!isAbsolute(path) || resolve(path) !== path) {
      throw new TypeError(`${label}.writablePaths[${index}] must be an absolute normalized path`);
    }
    return path;
  });
  if (new Set(writablePaths).size !== writablePaths.length) {
    throw new TypeError(`${label}.writablePaths must not contain duplicates`);
  }
  return deepFreeze({
    executable,
    args,
    cwd,
    env: parseEnvironment(input.env, `${label}.env`),
    writablePaths,
  });
}

function parseEnvironment(value: unknown, label: string): Readonly<Record<string, string>> {
  const input = asRecord(value, label);
  const environment: Record<string, string> = {};
  for (const [name, raw] of Object.entries(input)) {
    if (!ENVIRONMENT_NAME.test(name) || typeof raw !== 'string' || raw.includes('\0')) {
      throw new TypeError(`${label}.${name} is invalid`);
    }
    environment[name] = raw;
  }
  return deepFreeze(environment);
}

function assertNativeTransport(value: unknown, host: NativeHost): void {
  const input = asRecord(value, 'native model network request.transport');
  assertExactKeys(input, ['client', 'provider', 'fallback', 'baseUrl', 'proxy', 'gateway'], 'native model network request.transport');
  const profile = NATIVE_PROFILES[host];
  if (
    input.client !== host
    || input.provider !== profile.provider
    || input.fallback !== 'none'
    || input.baseUrl !== null
    || input.proxy !== null
    || input.gateway !== null
  ) throw new Error('HARNESS_INDIRECT_MODEL_TRANSPORT_PROHIBITED');
}

function assertDeterministicNpmCi(command: BoundaryCommand, registry: string): void {
  if (basename(command.executable).replace(/\.cmd$/i, '') !== 'npm') {
    throw new TypeError('dependency resolution must use npm ci');
  }
  const expected = [...NPM_CI_PREFIX, registry, ...NPM_CI_SUFFIX];
  if (JSON.stringify(command.args) !== JSON.stringify(expected)) {
    throw new TypeError(`dependency command must be: npm ${expected.join(' ')}`);
  }
}

export function assertNoRouteEnvironment(environment: Readonly<Record<string, string>>, label: string): void {
  for (const name of Object.keys(environment)) {
    const normalized = name.toUpperCase();
    if (
      normalized.includes('OPENROUTER')
      || normalized.includes('REQUESTY')
      || normalized.includes('PROXY')
      || normalized.includes('BASE_URL')
      || normalized.includes('API_BASE')
      || normalized.includes('REGISTRY')
    ) throw new Error(`${label}.${name} may redirect network traffic`);
  }
}

function parseOrigins(value: unknown, label: string): string[] {
  return asUniqueStrings(value, label).map((origin, index) => normalizePublicHttpsOrigin(origin, `${label}[${index}]`));
}

function assertNoOrigins(value: unknown, label: string): void {
  if (!Array.isArray(value) || value.length !== 0) throw new TypeError(`${label} must be an empty array`);
}

function assertSameOrigins(actual: readonly string[], expected: readonly string[], message: string): void {
  const left = [...actual].sort();
  const right = [...expected].sort();
  if (JSON.stringify(left) !== JSON.stringify(right)) throw new Error(message);
}

function parseNativeHost(value: unknown): NativeHost {
  if (value !== 'codex' && value !== 'claude-code') throw new TypeError('native model host is invalid');
  return value;
}

function parseMechanism(value: unknown, label: string): string {
  const mechanism = asNonEmptyString(value, label);
  if (!MECHANISM_NAME.test(mechanism)) throw new TypeError(`${label} is invalid`);
  return mechanism;
}

function byteString(value: unknown, label: string, allowEmpty: boolean): string {
  if (typeof value !== 'string' || value.includes('\0') || (!allowEmpty && value.length === 0)) {
    throw new TypeError(`${label} must be a ${allowEmpty ? '' : 'non-empty '}NUL-free string`);
  }
  return value;
}

function sameLaunch(left: BoundaryCommand, right: BoundaryCommand): boolean {
  return left.executable === right.executable && JSON.stringify(left.args) === JSON.stringify(right.args);
}

export function assertBoundaryCommandBinding(
  source: BoundaryCommand,
  bounded: BoundaryCommand,
  wrapperRequired: boolean,
  error: string,
): void {
  if (source.cwd !== bounded.cwd || stableEnvironment(source.env) !== stableEnvironment(bounded.env)) {
    throw new Error(error);
  }
  if (JSON.stringify(source.writablePaths) !== JSON.stringify(bounded.writablePaths)) throw new Error(error);
  if (sameLaunch(source, bounded)) {
    if (wrapperRequired) throw new Error(error);
    return;
  }
  const suffix = [source.executable, ...source.args];
  if (bounded.args.length < suffix.length) throw new Error(error);
  const actualSuffix = bounded.args.slice(-suffix.length);
  if (JSON.stringify(actualSuffix) !== JSON.stringify(suffix)) throw new Error(error);
}

function stableEnvironment(environment: Readonly<Record<string, string>>): string {
  return JSON.stringify(Object.entries(environment).sort(([left], [right]) => left.localeCompare(right)));
}

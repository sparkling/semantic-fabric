// SPDX-License-Identifier: MIT

import { isIP } from 'node:net';
import { isAbsolute, posix, resolve } from 'node:path';

export const DEVELOPMENT_AUTHORITY = 'development-only-no-promotion' as const;
export const APPROVED_NPM_REGISTRY = 'https://registry.npmjs.org/' as const;
export const SHA256_PATTERN = /^[a-f0-9]{64}$/;

export type NetworkMode = 'offline' | 'first-party-model' | 'dependency-resolution';

export interface HarnessConfig {
  schemaVersion: 1;
  authority: typeof DEVELOPMENT_AUTHORITY;
  approvedRegistry: string;
  firstPartyOrigins: string[];
  allowedTools: string[];
  requiredProtectedPaths: string[];
  environment: {
    allow: string[];
    denyExact: string[];
    denyPrefixes: string[];
    denySuffixes: string[];
  };
  limits: {
    maxTimeoutMs: number;
    maxOutputBytes: number;
    maxNewFileLines: number;
    terminationGraceMs: number;
  };
  evolution: {
    minimumTrainingTasks: 5;
    minimumSealedHoldouts: 5;
  };
}

export interface StructuredCommand {
  tool: string;
  executable: string;
  argv: string[];
  cwd: string;
  env: Record<string, string>;
  timeoutMs: number;
  maxOutputBytes: number;
}

export interface TaskContract {
  schemaVersion: 1;
  taskId: string;
  runId: string;
  workspaceRoot: string;
  readablePaths: string[];
  mutablePaths: string[];
  protectedPaths: string[];
  tools: string[];
  commands: StructuredCommand[];
  network: {
    mode: NetworkMode;
    allowedOrigins: string[];
  };
  authority: typeof DEVELOPMENT_AUTHORITY;
}

const ENVIRONMENT_NAME = /^[A-Z_][A-Z0-9_]*$/;
const SHELL_METACHARACTERS = /[;&|`$<>\r\n\0]/;
const INTRINSIC_UINT8_ARRAY = Uint8Array;
const INTRINSIC_UINT8_ARRAY_SET = Uint8Array.prototype.set;
const INTRINSIC_REFLECT_APPLY = Reflect.apply;
const TYPED_ARRAY_PROTOTYPE = Object.getPrototypeOf(Uint8Array.prototype) as object;
const TYPED_ARRAY_BUFFER_GETTER =
  Object.getOwnPropertyDescriptor(TYPED_ARRAY_PROTOTYPE, 'buffer')?.get;
const TYPED_ARRAY_BYTE_OFFSET_GETTER =
  Object.getOwnPropertyDescriptor(TYPED_ARRAY_PROTOTYPE, 'byteOffset')?.get;
const TYPED_ARRAY_BYTE_LENGTH_GETTER =
  Object.getOwnPropertyDescriptor(TYPED_ARRAY_PROTOTYPE, 'byteLength')?.get;
const TYPED_ARRAY_TAG_GETTER =
  Object.getOwnPropertyDescriptor(TYPED_ARRAY_PROTOTYPE, Symbol.toStringTag)?.get;

export function asRecord(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

export function asClosedRecord(value: unknown, label: string): Record<string, unknown> {
  const record = asRecord(value, label);
  const prototype = Object.getPrototypeOf(record);
  const keys = Reflect.ownKeys(record);
  const invalidKey = keys.some((key) => {
    const descriptor = Object.getOwnPropertyDescriptor(record, key);
    return typeof key !== 'string' || descriptor?.enumerable !== true
      || !Object.hasOwn(descriptor, 'value');
  });
  if ((prototype !== Object.prototype && prototype !== null) || invalidKey) {
    throw new TypeError(`${label} must be a plain own-key object`);
  }
  return Object.assign(Object.create(null), record) as Record<string, unknown>;
}

export function asDenseArray(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)
    || Object.getPrototypeOf(value) !== Array.prototype
    || Reflect.ownKeys(value).length !== value.length + 1) {
    throw new TypeError(`${label} must be a dense array without extra properties`);
  }
  for (let index = 0; index < value.length; index += 1) {
    const descriptor = Object.getOwnPropertyDescriptor(value, index);
    if (descriptor?.enumerable !== true || !Object.hasOwn(descriptor, 'value')) {
      throw new TypeError(`${label} must be a dense array without extra properties`);
    }
  }
  return value;
}

export function assertExactKeys(
  value: Record<string, unknown>,
  keys: readonly string[],
  label: string,
): void {
  const expected = new Set(keys);
  const unknown = Object.keys(value).filter((key) => !expected.has(key));
  const missing = keys.filter((key) => !(key in value));
  if (unknown.length || missing.length) {
    throw new TypeError(
      `${label} has invalid keys (missing: ${missing.join(', ') || 'none'}; unknown: ${unknown.join(', ') || 'none'})`,
    );
  }
}

export function asNonEmptyString(value: unknown, label: string): string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new TypeError(`${label} must be a non-empty string`);
  }
  return value;
}

export function asInteger(value: unknown, label: string, minimum = 0): number {
  if (!Number.isSafeInteger(value) || (value as number) < minimum) {
    throw new TypeError(`${label} must be a safe integer >= ${minimum}`);
  }
  return value as number;
}

/** Copy bytes without consulting input properties, iteration, or species constructors. */
export function snapshotUint8Array(
  value: unknown,
  label: string,
  maximumBytes: number,
): Uint8Array {
  if (!Number.isSafeInteger(maximumBytes) || maximumBytes < 0
    || !TYPED_ARRAY_BUFFER_GETTER || !TYPED_ARRAY_BYTE_OFFSET_GETTER
    || !TYPED_ARRAY_BYTE_LENGTH_GETTER || !TYPED_ARRAY_TAG_GETTER) {
    throw new TypeError(`${label} has an invalid byte bound`);
  }
  try {
    if (INTRINSIC_REFLECT_APPLY(TYPED_ARRAY_TAG_GETTER, value, []) !== 'Uint8Array') {
      throw new TypeError();
    }
    const buffer = INTRINSIC_REFLECT_APPLY(TYPED_ARRAY_BUFFER_GETTER, value, []);
    const offset = INTRINSIC_REFLECT_APPLY(TYPED_ARRAY_BYTE_OFFSET_GETTER, value, []);
    const length = INTRINSIC_REFLECT_APPLY(TYPED_ARRAY_BYTE_LENGTH_GETTER, value, []);
    if (length > maximumBytes) throw new TypeError();
    const source = new INTRINSIC_UINT8_ARRAY(buffer, offset, length);
    const snapshot = new INTRINSIC_UINT8_ARRAY(length);
    INTRINSIC_REFLECT_APPLY(INTRINSIC_UINT8_ARRAY_SET, snapshot, [source]);
    return snapshot;
  } catch {
    throw new TypeError(`${label} must be a Uint8Array within its byte bound`);
  }
}

export function asUniqueStrings(value: unknown, label: string, allowEmpty = false): string[] {
  if (!Array.isArray(value) || (!allowEmpty && value.length === 0)) {
    throw new TypeError(`${label} must be ${allowEmpty ? 'an' : 'a non-empty'} array`);
  }
  const strings = value.map((entry, index) => asNonEmptyString(entry, `${label}[${index}]`));
  if (new Set(strings).size !== strings.length) {
    throw new TypeError(`${label} must not contain duplicates`);
  }
  return strings;
}

export function normalizeWorkspacePath(value: unknown, label: string, allowRoot = false): string {
  const path = asNonEmptyString(value, label);
  if (allowRoot && path === '.') return path;
  if (isAbsolute(path) || path.includes('\\') || path.includes('\0')) {
    throw new TypeError(`${label} must be a repository-relative POSIX path`);
  }
  const segments = path.split('/');
  if (segments.some((segment) => segment === '' || segment === '.' || segment === '..')) {
    throw new TypeError(`${label} contains traversal or ambiguous segments`);
  }
  const normalized = posix.normalize(path);
  if (normalized !== path || normalized.startsWith('../')) {
    throw new TypeError(`${label} must already be normalized`);
  }
  return normalized;
}

export function assertStructuredText(value: unknown, label: string): string {
  const text = asNonEmptyString(value, label);
  if (SHELL_METACHARACTERS.test(text)) {
    throw new TypeError(`${label} contains a forbidden shell metacharacter`);
  }
  return text;
}

export function isForbiddenEnvironmentName(name: string, config: HarnessConfig): boolean {
  const normalized = name.toUpperCase();
  return config.environment.denyExact.includes(normalized)
    || config.environment.denyPrefixes.some((prefix) => normalized.startsWith(prefix))
    || config.environment.denySuffixes.some((suffix) => normalized.endsWith(suffix));
}

export function normalizePublicHttpsOrigin(value: unknown, label: string): string {
  const raw = asNonEmptyString(value, label);
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new TypeError(`${label} must be an absolute URL`);
  }
  if (
    url.protocol !== 'https:'
    || url.username !== ''
    || url.password !== ''
    || url.port !== ''
    || (url.pathname !== '' && url.pathname !== '/')
    || url.search !== ''
    || url.hash !== ''
  ) {
    throw new TypeError(`${label} must be a credential-free HTTPS origin`);
  }
  if (isNonPublicHostname(url.hostname)) {
    throw new TypeError(`${label} must not use a local or private-address origin`);
  }
  return url.origin;
}

function isNonPublicHostname(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, '');
  if (
    host === 'localhost'
    || host.endsWith('.localhost')
    || host.endsWith('.local')
    || host.endsWith('.internal')
    || host.endsWith('.home.arpa')
    || host.endsWith('.invalid')
    || host.endsWith('.test')
    || host.endsWith('.example')
  ) return true;

  const family = isIP(host);
  if (family === 6) return true;
  if (family !== 4) return false;
  const [a, b] = host.split('.').map(Number);
  return a === 0
    || a === 10
    || a === 127
    || (a === 100 && b >= 64 && b <= 127)
    || (a === 169 && b === 254)
    || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && b === 0)
    || (a === 192 && b === 168)
    || (a === 198 && (b === 18 || b === 19))
    || (a === 198 && b === 51)
    || (a === 203 && b === 0)
    || a >= 224;
}

function parseStringRecord(value: unknown, label: string): Record<string, string> {
  const input = asRecord(value, label);
  const output: Record<string, string> = {};
  for (const [name, raw] of Object.entries(input)) {
    if (!ENVIRONMENT_NAME.test(name)) throw new TypeError(`${label}.${name} is not a valid environment name`);
    const text = typeof raw === 'string' ? raw : null;
    if (text === null || SHELL_METACHARACTERS.test(text)) {
      throw new TypeError(`${label}.${name} must be a metacharacter-free string`);
    }
    output[name] = text;
  }
  return output;
}

function parseEnvironmentPolicy(value: unknown): HarnessConfig['environment'] {
  const input = asRecord(value, 'config.environment');
  assertExactKeys(input, ['allow', 'denyExact', 'denyPrefixes', 'denySuffixes'], 'config.environment');
  const result = {
    allow: asUniqueStrings(input.allow, 'config.environment.allow'),
    denyExact: asUniqueStrings(input.denyExact, 'config.environment.denyExact'),
    denyPrefixes: asUniqueStrings(input.denyPrefixes, 'config.environment.denyPrefixes'),
    denySuffixes: asUniqueStrings(input.denySuffixes, 'config.environment.denySuffixes'),
  };
  for (const [kind, names] of Object.entries(result)) {
    for (const name of names) {
      if (!ENVIRONMENT_NAME.test(name) || name !== name.toUpperCase()) {
        throw new TypeError(`config.environment.${kind} entries must be uppercase environment names`);
      }
    }
  }
  return result;
}

export function parseHarnessConfig(value: unknown): HarnessConfig {
  const input = asRecord(value, 'config');
  assertExactKeys(
    input,
    [
      'schemaVersion', 'authority', 'approvedRegistry', 'firstPartyOrigins', 'allowedTools',
      'requiredProtectedPaths', 'environment', 'limits', 'evolution',
    ],
    'config',
  );
  if (input.schemaVersion !== 1) throw new TypeError('config.schemaVersion must be 1');
  if (input.authority !== DEVELOPMENT_AUTHORITY) throw new TypeError('config.authority cannot grant promotion');

  const registry = `${normalizePublicHttpsOrigin(input.approvedRegistry, 'config.approvedRegistry')}/`;
  if (registry !== APPROVED_NPM_REGISTRY) {
    throw new TypeError(`config.approvedRegistry must be ${APPROVED_NPM_REGISTRY}`);
  }
  const firstPartyOrigins = asUniqueStrings(input.firstPartyOrigins, 'config.firstPartyOrigins')
    .map((origin, index) => normalizePublicHttpsOrigin(origin, `config.firstPartyOrigins[${index}]`));
  const environment = parseEnvironmentPolicy(input.environment);
  const allowedTools = asUniqueStrings(input.allowedTools, 'config.allowedTools');

  const requiredProtectedPaths = asUniqueStrings(input.requiredProtectedPaths, 'config.requiredProtectedPaths')
    .map((path, index) => normalizeWorkspacePath(path, `config.requiredProtectedPaths[${index}]`));
  const limitsInput = asRecord(input.limits, 'config.limits');
  assertExactKeys(
    limitsInput,
    ['maxTimeoutMs', 'maxOutputBytes', 'maxNewFileLines', 'terminationGraceMs'],
    'config.limits',
  );
  const limits = {
    maxTimeoutMs: asInteger(limitsInput.maxTimeoutMs, 'config.limits.maxTimeoutMs', 1),
    maxOutputBytes: asInteger(limitsInput.maxOutputBytes, 'config.limits.maxOutputBytes', 1),
    maxNewFileLines: asInteger(limitsInput.maxNewFileLines, 'config.limits.maxNewFileLines', 1),
    terminationGraceMs: asInteger(limitsInput.terminationGraceMs, 'config.limits.terminationGraceMs', 1),
  };
  if (limits.maxNewFileLines > 500) throw new TypeError('config.limits.maxNewFileLines cannot exceed 500');

  const evolutionInput = asRecord(input.evolution, 'config.evolution');
  assertExactKeys(evolutionInput, ['minimumTrainingTasks', 'minimumSealedHoldouts'], 'config.evolution');
  if (evolutionInput.minimumTrainingTasks !== 5 || evolutionInput.minimumSealedHoldouts !== 5) {
    throw new TypeError('config.evolution must preserve the 5+5 minimum gate');
  }

  return deepFreeze({
    schemaVersion: 1,
    authority: DEVELOPMENT_AUTHORITY,
    approvedRegistry: registry,
    firstPartyOrigins,
    allowedTools,
    requiredProtectedPaths,
    environment,
    limits,
    evolution: { minimumTrainingTasks: 5, minimumSealedHoldouts: 5 },
  });
}

export function parseStructuredCommand(
  value: unknown,
  config: HarnessConfig,
  declaredTools: readonly string[],
): StructuredCommand {
  const input = asRecord(value, 'command');
  assertExactKeys(input, ['tool', 'executable', 'argv', 'cwd', 'env', 'timeoutMs', 'maxOutputBytes'], 'command');
  const tool = assertStructuredText(input.tool, 'command.tool');
  if (!config.allowedTools.includes(tool) || !declaredTools.includes(tool)) {
    throw new TypeError(`command.tool "${tool}" is not declared`);
  }
  const executable = assertStructuredText(input.executable, 'command.executable');
  const executableName = executable.split('/').at(-1)?.replace(/\.exe$/i, '');
  if (executableName !== tool) throw new TypeError('command.executable must match command.tool');
  if (!Array.isArray(input.argv)) throw new TypeError('command.argv must be an array');
  const argv = input.argv.map((entry, index) => assertStructuredText(entry, `command.argv[${index}]`));
  const cwd = normalizeWorkspacePath(input.cwd, 'command.cwd', true);
  const env = parseStringRecord(input.env, 'command.env');
  for (const name of Object.keys(env)) {
    if (!config.environment.allow.includes(name) || isForbiddenEnvironmentName(name, config)) {
      throw new TypeError(`command.env.${name} is not allowed`);
    }
  }
  const timeoutMs = asInteger(input.timeoutMs, 'command.timeoutMs', 1);
  const maxOutputBytes = asInteger(input.maxOutputBytes, 'command.maxOutputBytes', 1);
  if (timeoutMs > config.limits.maxTimeoutMs) throw new TypeError('command.timeoutMs exceeds the configured ceiling');
  if (maxOutputBytes > config.limits.maxOutputBytes) {
    throw new TypeError('command.maxOutputBytes exceeds the configured ceiling');
  }
  return deepFreeze({ tool, executable, argv, cwd, env, timeoutMs, maxOutputBytes });
}

export function parseTaskContract(value: unknown, config: HarnessConfig): TaskContract {
  const input = asRecord(value, 'task');
  assertExactKeys(
    input,
    [
      'schemaVersion', 'taskId', 'runId', 'workspaceRoot', 'readablePaths', 'mutablePaths',
      'protectedPaths', 'tools', 'commands', 'network', 'authority',
    ],
    'task',
  );
  if (input.schemaVersion !== 1) throw new TypeError('task.schemaVersion must be 1');
  if (input.authority !== DEVELOPMENT_AUTHORITY) throw new TypeError('task.authority cannot grant promotion');
  const workspaceRoot = asNonEmptyString(input.workspaceRoot, 'task.workspaceRoot');
  if (!isAbsolute(workspaceRoot) || resolve(workspaceRoot) !== workspaceRoot) {
    throw new TypeError('task.workspaceRoot must be an absolute normalized path');
  }

  const parsePaths = (raw: unknown, label: string, allowEmpty = false) =>
    asUniqueStrings(raw, label, allowEmpty).map((path, index) => normalizeWorkspacePath(path, `${label}[${index}]`));
  const readablePaths = parsePaths(input.readablePaths, 'task.readablePaths', true);
  const mutablePaths = parsePaths(input.mutablePaths, 'task.mutablePaths');
  const protectedPaths = parsePaths(input.protectedPaths, 'task.protectedPaths');
  for (const required of config.requiredProtectedPaths) {
    if (!protectedPaths.includes(required)) throw new TypeError(`task.protectedPaths omits required path ${required}`);
  }
  for (const mutable of mutablePaths) {
    if (protectedPaths.some((protectedPath) => pathsOverlap(mutable, protectedPath))) {
      throw new TypeError(`task path ${mutable} overlaps a protected path`);
    }
  }

  const tools = asUniqueStrings(input.tools, 'task.tools');
  for (const tool of tools) {
    assertStructuredText(tool, 'task.tools entry');
    if (!config.allowedTools.includes(tool)) throw new TypeError(`task tool "${tool}" is not configured`);
  }
  if (!Array.isArray(input.commands)) throw new TypeError('task.commands must be an array');
  const commands = input.commands.map((command) => parseStructuredCommand(command, config, tools));

  const networkInput = asRecord(input.network, 'task.network');
  assertExactKeys(networkInput, ['mode', 'allowedOrigins'], 'task.network');
  const mode = networkInput.mode;
  if (mode !== 'offline' && mode !== 'first-party-model' && mode !== 'dependency-resolution') {
    throw new TypeError('task.network.mode is invalid');
  }
  const allowedOrigins = asUniqueStrings(networkInput.allowedOrigins, 'task.network.allowedOrigins', true)
    .map((origin, index) => normalizePublicHttpsOrigin(origin, `task.network.allowedOrigins[${index}]`));
  assertNetworkContract(mode, allowedOrigins, config);

  return deepFreeze({
    schemaVersion: 1,
    taskId: asNonEmptyString(input.taskId, 'task.taskId'),
    runId: asNonEmptyString(input.runId, 'task.runId'),
    workspaceRoot,
    readablePaths,
    mutablePaths,
    protectedPaths,
    tools,
    commands,
    network: { mode, allowedOrigins },
    authority: DEVELOPMENT_AUTHORITY,
  });
}

function assertNetworkContract(mode: NetworkMode, origins: string[], config: HarnessConfig): void {
  if (mode === 'offline' && origins.length !== 0) throw new TypeError('offline tasks cannot allow origins');
  if (mode === 'dependency-resolution') {
    const registryOrigin = new URL(config.approvedRegistry).origin;
    if (origins.length !== 1 || origins[0] !== registryOrigin) {
      throw new TypeError('dependency resolution may use only the approved registry');
    }
  }
  if (mode === 'first-party-model') {
    if (origins.length === 0 || origins.some((origin) => !config.firstPartyOrigins.includes(origin))) {
      throw new TypeError('model execution may use only configured first-party origins');
    }
  }
}

export function pathsOverlap(left: string, right: string): boolean {
  return left === right || left.startsWith(`${right}/`) || right.startsWith(`${left}/`);
}

export function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value as Record<string, unknown>)) deepFreeze(child);
  }
  return value;
}

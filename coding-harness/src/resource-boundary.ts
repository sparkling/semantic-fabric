// SPDX-License-Identifier: MIT

import { execFile } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { accessSync, constants, lstatSync, readFileSync, realpathSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';
import { promisify } from 'node:util';
import { deepFreeze } from './contracts.js';
import {
  assertBoundaryCommandBinding,
  parseBoundaryCommand,
  type BoundaryCommand,
} from './network.js';

const execFileAsync = promisify(execFile);
const RESOURCE_UNIT = /^semantic-fabric-harness-[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\.service$/;

export interface NativeResourceLimits {
  readonly memoryBytes: number;
  readonly processCount: number;
  readonly cpuQuotaPercent: number;
  readonly cpuTimeSeconds: number;
  readonly runtimeSeconds: number;
  readonly fileBytes: number;
  readonly openFiles: number;
}

export interface NativeResourceIsolationResult {
  readonly enforcement: 'systemd-cgroup-v2';
  readonly mechanism: 'systemd-transient-service';
  readonly limits: NativeResourceLimits;
  readonly limitsDigest: string;
  readonly scope: NativeResourceScope;
  readonly command: BoundaryCommand;
}

export interface NativeResourceScope { readonly unit: string }

export interface NativeResourceBoundary {
  wrap(command: BoundaryCommand, limits: NativeResourceLimits): unknown;
  assertStable(): void;
  terminateAndVerify(scope: NativeResourceScope): Promise<void>;
  launchEnvironment?(environment: Readonly<Record<string, string>>): Readonly<Record<string, string>>;
}

export interface SystemdResourceBoundaryOptions {
  readonly executablePath: string;
  readonly systemctlPath: string;
  readonly terminationGraceMs: number;
  readonly cgroupRoot?: string;
  readonly sourceEnvironment?: Readonly<Record<string, string | undefined>>;
}

export class SystemdResourceBoundary implements NativeResourceBoundary {
  readonly #identity: ExecutableIdentity;
  readonly #systemctlIdentity: ExecutableIdentity;
  readonly #cgroupRoot: string;
  readonly #controllerEnvironment: Readonly<Record<string, string>>;
  readonly #terminationGraceMs: number;

  constructor(options: SystemdResourceBoundaryOptions) {
    this.#identity = validateExecutable(options.executablePath);
    this.#systemctlIdentity = validateExecutable(options.systemctlPath);
    this.#cgroupRoot = realpathSync(options.cgroupRoot ?? '/sys/fs/cgroup');
    this.#terminationGraceMs = integer(
      options.terminationGraceMs, 1, 10_000, 'terminationGraceMs',
    );
    this.#controllerEnvironment = controllerEnvironment(
      options.sourceEnvironment ?? process.env,
    );
    assertCgroupV2(this.#cgroupRoot);
  }

  assertStable(): void {
    if (validateExecutable(this.#identity.path).digest !== this.#identity.digest
      || validateExecutable(this.#systemctlIdentity.path).digest !== this.#systemctlIdentity.digest) {
      throw new Error('HARNESS_RESOURCE_BOUNDARY_CHANGED');
    }
    assertCgroupV2(this.#cgroupRoot);
  }

  wrap(command: BoundaryCommand, rawLimits: NativeResourceLimits): NativeResourceIsolationResult {
    this.assertStable();
    const limits = validateResourceLimits(rawLimits);
    const scope = parseResourceScope({
      unit: `semantic-fabric-harness-${randomUUID()}.service`,
    });
    const bounded = deepFreeze({
      ...command,
      executable: this.#identity.path,
      args: [
        '--user', '--quiet', '--wait', '--collect', '--pipe', '--service-type=exec',
        `--unit=${scope.unit}`,
        `--property=MemoryMax=${limits.memoryBytes}`,
        '--property=MemorySwapMax=0',
        `--property=TasksMax=${limits.processCount}`,
        `--property=CPUQuota=${limits.cpuQuotaPercent}%`,
        `--property=LimitCPU=${limits.cpuTimeSeconds}`,
        `--property=RuntimeMaxSec=${limits.runtimeSeconds}s`,
        `--property=LimitFSIZE=${limits.fileBytes}`,
        `--property=LimitNOFILE=${limits.openFiles}`,
        '--property=KillMode=control-group',
        '--property=ExitType=cgroup',
        '--property=KillSignal=SIGTERM',
        '--property=FinalKillSignal=SIGKILL',
        '--property=SendSIGKILL=yes',
        `--property=TimeoutStopSec=${this.#terminationGraceMs}ms`,
        '--property=OOMPolicy=stop',
        `--working-directory=${command.cwd}`,
        '--', command.executable, ...command.args,
      ],
    });
    return deepFreeze({
      enforcement: 'systemd-cgroup-v2',
      mechanism: 'systemd-transient-service',
      limits,
      limitsDigest: digest(limits),
      scope,
      command: bounded,
    });
  }

  async terminateAndVerify(rawScope: NativeResourceScope): Promise<void> {
    this.assertStable();
    const scope = parseResourceScope(rawScope);
    const before = await this.#readState(scope.unit);
    if (releasedState(before)) {
      this.assertStable();
      return;
    }
    try {
      await runSystemctl(
        this.#systemctlIdentity.path,
        ['--user', '--no-ask-password', '--quiet', 'stop', '--', scope.unit],
        this.#controllerEnvironment,
        this.#terminationGraceMs + 5_000,
        true,
      );
    } catch (error) {
      if (!isNonzeroExit(error)) throw new Error('HARNESS_RESOURCE_TERMINATION_FAILED');
    }
    const after = await this.#readState(scope.unit);
    if (!releasedState(after)
      || !cgroupUnpopulated(this.#cgroupRoot, before.ControlGroup, scope.unit)) {
      throw new Error('HARNESS_RESOURCE_TERMINATION_FAILED');
    }
    this.assertStable();
  }

  async #readState(unit: string): Promise<SystemdUnitState> {
    const output = await runSystemctl(
      this.#systemctlIdentity.path,
      [
        '--user', '--no-ask-password', 'show',
        '--property=LoadState', '--property=ActiveState',
        '--property=SubState', '--property=ControlGroup', '--', unit,
      ],
      this.#controllerEnvironment,
      5_000,
      false,
    ).catch(() => { throw new Error('HARNESS_RESOURCE_TERMINATION_FAILED'); });
    return parseSystemdState(output);
  }

  launchEnvironment(environment: Readonly<Record<string, string>>): Readonly<Record<string, string>> {
    return Object.freeze({ ...environment, ...this.#controllerEnvironment });
  }
}

export function isolateNativeResources(
  command: BoundaryCommand,
  rawLimits: NativeResourceLimits,
  boundary?: NativeResourceBoundary,
): NativeResourceIsolationResult {
  if (typeof boundary?.terminateAndVerify !== 'function') {
    throw new Error('HARNESS_NATIVE_RESOURCE_BOUNDARY_REQUIRED');
  }
  boundary.assertStable();
  const expected = validateResourceLimits(rawLimits);
  const input = boundary.wrap(command, expected) as Partial<NativeResourceIsolationResult>;
  if (input.enforcement !== 'systemd-cgroup-v2'
    || input.mechanism !== 'systemd-transient-service'
    || input.limitsDigest !== digest(expected)) {
    throw new Error('HARNESS_NATIVE_RESOURCE_BOUNDARY_INVALID');
  }
  const limits = validateResourceLimits(input.limits as NativeResourceLimits);
  if (digest(limits) !== digest(expected)) {
    throw new Error('HARNESS_NATIVE_RESOURCE_SCOPE_MISMATCH');
  }
  const bounded = parseBoundaryCommand(input.command, 'native resource command');
  const scope = parseResourceScope(input.scope);
  if (boundary instanceof SystemdResourceBoundary
    && bounded.args.filter((argument) => argument === `--unit=${scope.unit}`).length !== 1) {
    throw new Error('HARNESS_NATIVE_RESOURCE_SCOPE_MISMATCH');
  }
  assertBoundaryCommandBinding(
    command,
    bounded,
    true,
    'HARNESS_NATIVE_RESOURCE_COMMAND_MISMATCH',
  );
  boundary.assertStable();
  return deepFreeze({
    enforcement: 'systemd-cgroup-v2',
    mechanism: 'systemd-transient-service',
    limits,
    limitsDigest: input.limitsDigest,
    scope,
    command: bounded,
  });
}

export function limitsForProcessDeadline(
  limits: NativeResourceLimits,
  timeoutMs: number,
): NativeResourceLimits {
  const runtimeSeconds = Math.min(limits.runtimeSeconds, Math.max(1, Math.ceil(timeoutMs / 1_000)));
  return validateResourceLimits({
    ...limits,
    runtimeSeconds,
    cpuTimeSeconds: Math.min(limits.cpuTimeSeconds, runtimeSeconds * 16),
  });
}

export function terminateNativeResourceScope(
  boundary: NativeResourceBoundary,
  isolation: NativeResourceIsolationResult,
): Promise<void> {
  return boundary.terminateAndVerify(isolation.scope);
}

function parseResourceScope(value: unknown): NativeResourceScope {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('HARNESS_RESOURCE_TERMINATION_SCOPE_INVALID');
  }
  const input = value as Record<string, unknown>;
  if (Object.keys(input).length !== 1 || !RESOURCE_UNIT.test(String(input.unit ?? ''))) {
    throw new Error('HARNESS_RESOURCE_TERMINATION_SCOPE_INVALID');
  }
  return deepFreeze({ unit: input.unit as string });
}

async function runSystemctl(
  executable: string,
  args: readonly string[],
  environment: Readonly<Record<string, string>>,
  timeout: number,
  discardOutput: boolean,
): Promise<string> {
  const result = await execFileAsync(executable, [...args], {
    env: environment,
    encoding: 'utf8',
    timeout,
    maxBuffer: 4_096,
    windowsHide: true,
  });
  return discardOutput ? '' : String(result.stdout);
}

function isNonzeroExit(error: unknown): boolean {
  if (error === null || typeof error !== 'object') return false;
  const input = error as { code?: unknown; killed?: unknown; signal?: unknown };
  return typeof input.code === 'number' && input.killed !== true && input.signal == null;
}

interface SystemdUnitState {
  readonly LoadState: string;
  readonly ActiveState: string;
  readonly SubState: string;
  readonly ControlGroup: string;
}

function parseSystemdState(output: string): SystemdUnitState {
  const allowed = new Set(['LoadState', 'ActiveState', 'SubState', 'ControlGroup']);
  const properties = new Map<string, string>();
  for (const line of output.trim().split('\n')) {
    const separator = line.indexOf('=');
    const name = separator < 0 ? '' : line.slice(0, separator);
    if (!allowed.has(name) || properties.has(name)) {
      throw new Error('HARNESS_RESOURCE_TERMINATION_FAILED');
    }
    properties.set(name, line.slice(separator + 1));
  }
  if (properties.size !== allowed.size) {
    throw new Error('HARNESS_RESOURCE_TERMINATION_FAILED');
  }
  return Object.freeze(Object.fromEntries(properties) as unknown as SystemdUnitState);
}

function releasedState(state: SystemdUnitState): boolean {
  return ['loaded', 'not-found'].includes(state.LoadState)
    && state.ActiveState === 'inactive'
    && state.SubState === 'dead'
    && state.ControlGroup === '';
}

function cgroupUnpopulated(root: string, controlGroup: string, unit: string): boolean {
  if (controlGroup === '') return true;
  if (!controlGroup.startsWith('/') || controlGroup.includes('\0')
    || controlGroup.split('/').includes('..') || !controlGroup.endsWith(`/${unit}`)) return false;
  const eventsPath = resolve(root, `.${controlGroup}`, 'cgroup.events');
  if (!eventsPath.startsWith(`${root}/`)) return false;
  try {
    return readFileSync(eventsPath, 'utf8').split('\n').includes('populated 0');
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === 'ENOENT';
  }
}

export function validateResourceLimits(value: NativeResourceLimits): NativeResourceLimits {
  const limits = {
    memoryBytes: integer(value?.memoryBytes, 64 * 1024 * 1024, 64 * 1024 ** 3, 'memoryBytes'),
    processCount: integer(value?.processCount, 4, 4096, 'processCount'),
    cpuQuotaPercent: integer(value?.cpuQuotaPercent, 1, 1600, 'cpuQuotaPercent'),
    cpuTimeSeconds: integer(value?.cpuTimeSeconds, 1, 86_400, 'cpuTimeSeconds'),
    runtimeSeconds: integer(value?.runtimeSeconds, 1, 86_400, 'runtimeSeconds'),
    fileBytes: integer(value?.fileBytes, 1_048_576, 1024 ** 4, 'fileBytes'),
    openFiles: integer(value?.openFiles, 16, 65_536, 'openFiles'),
  };
  if (limits.cpuTimeSeconds > limits.runtimeSeconds * 16) {
    throw new Error('HARNESS_RESOURCE_CPU_TIME_INVALID');
  }
  return deepFreeze(limits);
}

interface ExecutableIdentity { readonly path: string; readonly digest: string }

function validateExecutable(value: string): ExecutableIdentity {
  try {
    if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) throw new Error();
    const stat = lstatSync(value);
    const uid = process.getuid?.() ?? stat.uid;
    accessSync(value, constants.X_OK);
    if (!stat.isFile() || stat.isSymbolicLink() || realpathSync(value) !== value
      || stat.nlink !== 1 || (stat.mode & 0o022) !== 0
      || (stat.uid !== 0 && stat.uid !== uid)) throw new Error();
    return Object.freeze({
      path: value,
      digest: createHash('sha256').update(readFileSync(value)).digest('hex'),
    });
  } catch {
    throw new Error('HARNESS_RESOURCE_BOUNDARY_PATH_INVALID');
  }
}

function assertCgroupV2(root: string): void {
  try {
    const controllers = readFileSync(resolve(root, 'cgroup.controllers'), 'utf8');
    for (const required of ['cpu', 'memory', 'pids']) {
      if (!controllers.split(/\s+/).includes(required)) throw new Error();
    }
  } catch {
    throw new Error('HARNESS_CGROUP_V2_CONTROLLERS_UNAVAILABLE');
  }
}

function integer(value: number, minimum: number, maximum: number, name: string): number {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new TypeError(`HARNESS_RESOURCE_${name.toUpperCase()}_INVALID`);
  }
  return value;
}

function digest(value: unknown): string {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex');
}

function controllerEnvironment(
  source: Readonly<Record<string, string | undefined>>,
): Readonly<Record<string, string>> {
  const output: Record<string, string> = {};
  for (const name of ['DBUS_SESSION_BUS_ADDRESS', 'XDG_RUNTIME_DIR'] as const) {
    const value = source[name];
    if (value === undefined || value.length === 0 || value.includes('\0')) {
      throw new Error(`HARNESS_RESOURCE_CONTROLLER_ENVIRONMENT_REQUIRED:${name}`);
    }
    output[name] = value;
  }
  return Object.freeze(output);
}

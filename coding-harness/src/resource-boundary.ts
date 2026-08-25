// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { accessSync, constants, lstatSync, readFileSync, realpathSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';
import { deepFreeze } from './contracts.js';
import {
  assertBoundaryCommandBinding,
  parseBoundaryCommand,
  type BoundaryCommand,
} from './network.js';

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
  readonly command: BoundaryCommand;
}

export interface NativeResourceBoundary {
  wrap(command: BoundaryCommand, limits: NativeResourceLimits): unknown;
  assertStable(): void;
  launchEnvironment?(environment: Readonly<Record<string, string>>): Readonly<Record<string, string>>;
}

export interface SystemdResourceBoundaryOptions {
  readonly executablePath: string;
  readonly cgroupRoot?: string;
  readonly sourceEnvironment?: Readonly<Record<string, string | undefined>>;
}

export class SystemdResourceBoundary implements NativeResourceBoundary {
  readonly #identity: ExecutableIdentity;
  readonly #cgroupRoot: string;
  readonly #controllerEnvironment: Readonly<Record<string, string>>;

  constructor(options: SystemdResourceBoundaryOptions) {
    this.#identity = validateExecutable(options.executablePath);
    this.#cgroupRoot = options.cgroupRoot ?? '/sys/fs/cgroup';
    this.#controllerEnvironment = controllerEnvironment(
      options.sourceEnvironment ?? process.env,
    );
    assertCgroupV2(this.#cgroupRoot);
  }

  assertStable(): void {
    if (validateExecutable(this.#identity.path).digest !== this.#identity.digest) {
      throw new Error('HARNESS_RESOURCE_BOUNDARY_CHANGED');
    }
    assertCgroupV2(this.#cgroupRoot);
  }

  wrap(command: BoundaryCommand, rawLimits: NativeResourceLimits): NativeResourceIsolationResult {
    this.assertStable();
    const limits = validateResourceLimits(rawLimits);
    const bounded = deepFreeze({
      ...command,
      executable: this.#identity.path,
      args: [
        '--user', '--quiet', '--wait', '--collect', '--pipe', '--service-type=exec',
        `--property=MemoryMax=${limits.memoryBytes}`,
        '--property=MemorySwapMax=0',
        `--property=TasksMax=${limits.processCount}`,
        `--property=CPUQuota=${limits.cpuQuotaPercent}%`,
        `--property=LimitCPU=${limits.cpuTimeSeconds}`,
        `--property=RuntimeMaxSec=${limits.runtimeSeconds}s`,
        `--property=LimitFSIZE=${limits.fileBytes}`,
        `--property=LimitNOFILE=${limits.openFiles}`,
        '--property=KillMode=control-group',
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
      command: bounded,
    });
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
  if (boundary === undefined) throw new Error('HARNESS_NATIVE_RESOURCE_BOUNDARY_REQUIRED');
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
    command: bounded,
  });
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

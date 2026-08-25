// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { constants, accessSync, lstatSync, readFileSync, realpathSync, statSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { deepFreeze } from './contracts.js';
import type {
  BoundaryCommand,
  OfflineIsolationResult,
  OfflineProcessIsolator,
} from './network.js';
import {
  isolateNativeResources,
  type NativeResourceBoundary,
  type NativeResourceLimits,
} from './resource-boundary.js';

export interface ReadOnlyMount {
  readonly source: string;
  readonly destination: string;
}

export interface SystemOfflineIsolatorOptions {
  readonly platform?: NodeJS.Platform;
  readonly executablePath: string;
  readonly writableRoot: string;
  readonly readOnlyMounts: readonly ReadOnlyMount[];
  readonly resourceBoundary: NativeResourceBoundary;
  readonly resourceLimits: NativeResourceLimits;
}

export function createSystemOfflineIsolator(
  options: SystemOfflineIsolatorOptions,
): OfflineProcessIsolator {
  if ((options.platform ?? process.platform) !== 'linux') {
    throw new Error('HARNESS_OFFLINE_OS_SANDBOX_UNAVAILABLE');
  }
  const executable = validateExecutable(options.executablePath);
  const writableRoot = validateDirectory(options.writableRoot, 'HARNESS_OFFLINE_WRITABLE_ROOT_INVALID');
  const mounts = validateMounts(options.readOnlyMounts);
  if (mounts.length === 0) throw new Error('HARNESS_OFFLINE_READ_ONLY_MOUNTS_REQUIRED');
  if (options.resourceBoundary === undefined) {
    throw new Error('HARNESS_OFFLINE_RESOURCE_BOUNDARY_REQUIRED');
  }
  return systemIsolator(
    executable,
    writableRoot,
    mounts,
    options.resourceBoundary,
    options.resourceLimits,
  );
}

function systemIsolator(
  executable: ExecutableIdentity,
  writableRoot: string,
  mounts: readonly ReadOnlyMount[],
  resourceBoundary: NativeResourceBoundary,
  resourceLimits: NativeResourceLimits,
): OfflineProcessIsolator {
  return Object.freeze({
    launchEnvironment(environment: Readonly<Record<string, string>>) {
      return resourceBoundary.launchEnvironment?.(environment) ?? environment;
    },
    assertStable(): void {
      if (validateExecutable(executable.path).digest !== executable.digest) {
        throw new Error('HARNESS_OFFLINE_OS_SANDBOX_CHANGED');
      }
      resourceBoundary.assertStable();
    },
    isolate(command: BoundaryCommand): OfflineIsolationResult {
      if (validateExecutable(executable.path).digest !== executable.digest) {
        throw new Error('HARNESS_OFFLINE_OS_SANDBOX_CHANGED');
      }
      const writable = command.writablePaths.map((path) =>
        validateWritablePath(path, writableRoot));
      const cwd = validateDirectory(command.cwd, 'HARNESS_OFFLINE_CWD_INVALID');
      if (!contains(writableRoot, cwd) || cwd === writableRoot) {
        throw new Error('HARNESS_OFFLINE_CWD_OUTSIDE_RUN_ROOT');
      }
      const commandMount = Object.freeze({ source: cwd, destination: cwd });
      const visibleMounts = [...mounts, commandMount];
      assertVisible(command.executable, visibleMounts, 'HARNESS_OFFLINE_EXECUTABLE_NOT_MOUNTED');
      const directories = parentDirectories([
        '/dev', '/proc', '/tmp', '/run', '/home/harness',
        ...visibleMounts.map(({ destination }) => destination),
        ...writable,
      ]);
      const prefix = [
        '--die-with-parent', '--new-session', '--unshare-all', '--clearenv', '--tmpfs', '/',
        ...directories.flatMap((path) => ['--dir', path]),
        '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/run',
        ...visibleMounts.flatMap(({ source, destination }) => ['--ro-bind', source, destination]),
        ...writable.flatMap((path) => ['--bind', path, path]),
        ...Object.entries(command.env).flatMap(([name, value]) => ['--setenv', name, value]),
        '--chdir', command.cwd, '--',
      ];
      const namespace = deepFreeze({
        ...command,
        executable: executable.path,
        args: [...prefix, command.executable, ...command.args],
      });
      const bounded = isolateNativeResources(namespace, resourceLimits, resourceBoundary);
      return deepFreeze({
        enforcement: 'os-network-namespace',
        mechanism: 'systemd-cgroup-v2-bwrap',
        command: bounded.command,
      });
    },
  });
}

function validateMounts(values: readonly ReadOnlyMount[]): ReadOnlyMount[] {
  const destinations = new Set<string>();
  return values.map((mount, index) => {
    if (mount === null || typeof mount !== 'object') {
      throw new TypeError(`readOnlyMounts[${index}] must be an object`);
    }
    const source = validateExistingPath(mount.source, `readOnlyMounts[${index}].source`);
    const destination = validateAbsolute(mount.destination, `readOnlyMounts[${index}].destination`);
    if (destination === '/' || destinations.has(destination)) {
      throw new Error('HARNESS_OFFLINE_READ_ONLY_MOUNT_INVALID');
    }
    destinations.add(destination);
    if (!statSync(source).isDirectory()) {
      throw new Error('HARNESS_OFFLINE_READ_ONLY_MOUNT_MUST_BE_DIRECTORY');
    }
    return Object.freeze({ source, destination });
  });
}

function validateWritablePath(value: string, writableRoot: string): string {
  const path = validateExistingPath(value, 'writable path');
  const delta = relative(writableRoot, path);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error('HARNESS_OFFLINE_WRITABLE_PATH_OUTSIDE_ROOT');
  }
  const stat = lstatSync(path);
  if (!stat.isFile() && !stat.isDirectory()) throw new Error('HARNESS_OFFLINE_WRITABLE_PATH_INVALID');
  if (stat.isFile() && stat.nlink > 1) throw new Error('HARNESS_OFFLINE_WRITABLE_PATH_HARDLINKED');
  return path;
}

function assertVisible(value: string, mounts: readonly ReadOnlyMount[], error: string): void {
  const visible = mounts.some(({ destination }) => contains(destination, value));
  if (!visible) throw new Error(error);
}

function parentDirectories(paths: readonly string[]): string[] {
  const output = new Set<string>();
  for (const path of paths) {
    let current = path;
    if (!path.endsWith(sep) && (() => { try { return statSync(path).isFile(); } catch { return false; } })()) {
      current = dirname(path);
    }
    while (current !== '/') {
      output.add(current);
      current = dirname(current);
    }
  }
  return [...output].sort((left, right) => left.split(sep).length - right.split(sep).length);
}

function contains(root: string, path: string): boolean {
  const delta = relative(root, path);
  return delta === '' || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

function validateDirectory(value: string, error: string): string {
  const path = validateExistingPath(value, error);
  if (!statSync(path).isDirectory()) throw new Error(error);
  return path;
}

function validateExistingPath(value: string, label: string): string {
  const path = validateAbsolute(value, label);
  const stat = lstatSync(path);
  if (stat.isSymbolicLink() || realpathSync(path) !== path) throw new Error(`${label} is not canonical`);
  return path;
}

function validateAbsolute(value: string, label: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new TypeError(`${label} must be an absolute normalized path`);
  }
  return value;
}

interface ExecutableIdentity { readonly path: string; readonly digest: string }

function validateExecutable(value: string): ExecutableIdentity {
  try {
    const path = validateExistingPath(value, 'HARNESS_OFFLINE_OS_SANDBOX_PATH_INVALID');
    accessSync(path, constants.X_OK);
    const stat = lstatSync(path);
    const uid = process.getuid?.() ?? stat.uid;
    if (!stat.isFile() || stat.nlink !== 1 || (stat.mode & 0o022) !== 0
      || (stat.uid !== 0 && stat.uid !== uid)) throw new Error();
    return Object.freeze({
      path,
      digest: createHash('sha256').update(readFileSync(path)).digest('hex'),
    });
  } catch {
    throw new Error('HARNESS_OFFLINE_OS_SANDBOX_PATH_INVALID');
  }
}

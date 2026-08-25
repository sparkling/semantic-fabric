// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  accessSync,
  constants,
  existsSync,
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
  statSync,
} from 'node:fs';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';
import type { StructuredCommand } from './contracts.js';
import type {
  NativeResourceBoundary,
  NativeResourceLimits,
  NativeResourceScope,
} from './resource-boundary.js';
import {
  createSystemOfflineIsolator,
  type BoundaryCommand,
  type OfflineProcessIsolator,
  type ReadOnlyMount,
} from './network.js';

export interface RustOfflineProfileOptions {
  readonly writableRoot: string;
  readonly cargoExecutable: string;
  readonly toolchainRoot: string;
  readonly registryRoot: string;
  readonly registryKey: string;
  readonly cargoExtensionRoot?: string;
  readonly bwrapExecutable: string;
  readonly resourceBoundary: NativeResourceBoundary;
  readonly resourceLimits: NativeResourceLimits;
  readonly assertClosureStable?: () => void;
}

export interface RustOfflineProfile {
  readonly isolator: OfflineProcessIsolator;
  readonly cargoExecutable: '/toolchain/bin/cargo';
  readonly environment: Readonly<Record<string, string>>;
  readonly readOnlyMounts: readonly ReadOnlyMount[];
}

export function createRustOfflineProfile(options: RustOfflineProfileOptions): RustOfflineProfile {
  const toolchainRoot = canonicalDirectory(options.toolchainRoot, 'HARNESS_RUST_TOOLCHAIN_INVALID');
  const cargoExecutable = canonicalFile(options.cargoExecutable, 'HARNESS_RUST_CARGO_INVALID');
  if (!contains(toolchainRoot, cargoExecutable)
    || relative(toolchainRoot, cargoExecutable).split(sep).join('/') !== 'bin/cargo') {
    throw new Error('HARNESS_RUST_CARGO_TOOLCHAIN_MISMATCH');
  }
  if (!/^[A-Za-z0-9._-]{8,128}$/.test(options.registryKey)) {
    throw new Error('HARNESS_RUST_REGISTRY_KEY_INVALID');
  }
  const registryRoot = canonicalDirectory(options.registryRoot, 'HARNESS_RUST_REGISTRY_INVALID');
  const mounts: ReadOnlyMount[] = [
    mount('/usr/bin', '/usr/bin'),
    mount('/usr/lib', '/usr/lib'),
    mount('/usr/lib', '/lib'),
    mount('/usr/libexec', '/usr/libexec'),
    mount('/usr/include', '/usr/include'),
    mount('/usr/share', '/usr/share'),
    mount('/etc/alternatives', '/etc/alternatives'),
    mount(toolchainRoot, '/toolchain'),
  ];
  if (existsSync('/usr/lib64')) {
    mounts.push(mount('/usr/lib64', '/usr/lib64'), mount('/usr/lib64', '/lib64'));
  }
  for (const kind of ['cache', 'index', 'src'] as const) {
    mounts.push(mount(
      join(registryRoot, kind, options.registryKey),
      `/cargo-home/registry/${kind}/${options.registryKey}`,
    ));
  }
  const cargoExtension = options.cargoExtensionRoot === undefined
    ? null
    : validateCargoExtensionRoot(options.cargoExtensionRoot);
  if (cargoExtension !== null) mounts.push(mount(cargoExtension.root, '/cargo-home/bin'));
  const environment = Object.freeze({
    PATH: options.cargoExtensionRoot === undefined
      ? '/toolchain/bin:/usr/bin'
      : '/cargo-home/bin:/toolchain/bin:/usr/bin',
    HOME: '/home/harness',
    CARGO_HOME: '/cargo-home',
    CARGO_NET_OFFLINE: 'true',
    CARGO_INCREMENTAL: '0',
  });
  let isolator: OfflineProcessIsolator = createSystemOfflineIsolator({
      writableRoot: options.writableRoot,
      readOnlyMounts: mounts,
      executablePath: options.bwrapExecutable,
      resourceBoundary: options.resourceBoundary,
      resourceLimits: options.resourceLimits,
    });
  if (cargoExtension !== null) isolator = extensionStableIsolator(isolator, cargoExtension);
  if (options.assertClosureStable !== undefined) {
    if (typeof options.assertClosureStable !== 'function') {
      throw new TypeError('HARNESS_RUST_CLOSURE_ASSERTION_INVALID');
    }
    options.assertClosureStable();
    isolator = closureStableIsolator(isolator, options.assertClosureStable);
  }
  return Object.freeze({
    isolator,
    cargoExecutable: '/toolchain/bin/cargo',
    environment,
    readOnlyMounts: Object.freeze(mounts),
  });
}

function closureStableIsolator(
  isolator: OfflineProcessIsolator,
  assertClosureStable: () => void,
): OfflineProcessIsolator {
  return Object.freeze({
    assertStable() {
      assertClosureStable();
      isolator.assertStable();
      assertClosureStable();
    },
    isolate(command: BoundaryCommand) {
      assertClosureStable();
      const isolated = isolator.isolate(command);
      assertClosureStable();
      return isolated;
    },
    async terminateAndVerify(scope: NativeResourceScope) {
      await isolator.terminateAndVerify(scope);
    },
    launchEnvironment(environment: Readonly<Record<string, string>>) {
      return isolator.launchEnvironment?.(environment) ?? environment;
    },
  });
}

interface CargoExtensionIdentity {
  readonly root: string;
  readonly path: string;
  readonly dev: number;
  readonly ino: number;
  readonly digest: string;
}

function validateCargoExtensionRoot(rootValue: string): CargoExtensionIdentity {
  const root = canonicalDirectory(
    rootValue,
    'HARNESS_RUST_CARGO_EXTENSION_ROOT_INVALID',
  );
  const rootStat = lstatSync(root);
  const uid = process.getuid?.() ?? rootStat.uid;
  if (rootStat.uid !== uid || (rootStat.mode & 0o077) !== 0
    || readdirSync(root).join('\0') !== 'cargo-llvm-cov') {
    throw new Error('HARNESS_RUST_CARGO_EXTENSION_ROOT_INVALID');
  }
  const path = canonicalFile(
    join(root, 'cargo-llvm-cov'),
    'HARNESS_RUST_CARGO_EXTENSION_INVALID',
  );
  const stat = lstatSync(path);
  accessSync(path, constants.X_OK);
  if (stat.uid !== uid || stat.nlink !== 1 || (stat.mode & 0o222) !== 0) {
    throw new Error('HARNESS_RUST_CARGO_EXTENSION_INVALID');
  }
  return Object.freeze({
    root,
    path,
    dev: stat.dev,
    ino: stat.ino,
    digest: createHash('sha256').update(readFileSync(path)).digest('hex'),
  });
}

function extensionStableIsolator(
  isolator: OfflineProcessIsolator,
  expected: CargoExtensionIdentity,
): OfflineProcessIsolator {
  const assertExtension = () => {
    const current = validateCargoExtensionRoot(expected.root);
    if (current.path !== expected.path || current.dev !== expected.dev
      || current.ino !== expected.ino || current.digest !== expected.digest) {
      throw new Error('HARNESS_RUST_CARGO_EXTENSION_CHANGED');
    }
  };
  return Object.freeze({
    assertStable() {
      isolator.assertStable();
      assertExtension();
    },
    isolate(command: BoundaryCommand) {
      assertExtension();
      return isolator.isolate(command);
    },
    async terminateAndVerify(scope: NativeResourceScope) {
      await isolator.terminateAndVerify(scope);
    },
    launchEnvironment(environment: Readonly<Record<string, string>>) {
      return isolator.launchEnvironment?.(environment) ?? environment;
    },
  });
}

export function bindRustOfflineCommand(
  command: StructuredCommand,
  profile: RustOfflineProfile,
): StructuredCommand {
  if (command.tool !== 'cargo') throw new Error('HARNESS_RUST_COMMAND_TOOL_INVALID');
  if (!command.argv.includes('--offline')) throw new Error('HARNESS_RUST_COMMAND_OFFLINE_FLAG_REQUIRED');
  const subcommand = command.argv.find((argument) => !argument.startsWith('-'));
  if (subcommand !== 'fmt' && !command.argv.includes('--locked')) {
    throw new Error('HARNESS_RUST_COMMAND_LOCKED_FLAG_REQUIRED');
  }
  return Object.freeze({
    ...command,
    executable: profile.cargoExecutable,
    env: { ...command.env, ...profile.environment },
  });
}

function mount(source: string, destination: string): ReadOnlyMount {
  canonicalDirectory(source, 'HARNESS_RUST_MOUNT_INVALID');
  return Object.freeze({ source, destination });
}

function canonicalDirectory(value: string, error: string): string {
  const path = canonicalPath(value, error);
  if (!statSync(path).isDirectory()) throw new Error(error);
  return path;
}

function canonicalFile(value: string, error: string): string {
  const path = canonicalPath(value, error);
  if (!statSync(path).isFile()) throw new Error(error);
  return path;
}

function canonicalPath(value: string, error: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0') || realpathSync(value) !== value) {
    throw new Error(error);
  }
  return value;
}

function contains(root: string, child: string): boolean {
  const delta = relative(root, child);
  return delta === '' || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

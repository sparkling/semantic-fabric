// SPDX-License-Identifier: MIT

import { existsSync, realpathSync, statSync } from 'node:fs';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';
import type { StructuredCommand } from './contracts.js';
import type { NativeResourceBoundary, NativeResourceLimits } from './resource-boundary.js';
import {
  createSystemOfflineIsolator,
  type OfflineProcessIsolator,
  type ReadOnlyMount,
} from './network.js';

export interface RustOfflineProfileOptions {
  readonly writableRoot: string;
  readonly cargoExecutable: string;
  readonly toolchainRoot: string;
  readonly registryRoot: string;
  readonly registryKey: string;
  readonly bwrapExecutable: string;
  readonly resourceBoundary: NativeResourceBoundary;
  readonly resourceLimits: NativeResourceLimits;
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
  const environment = Object.freeze({
    PATH: '/toolchain/bin:/usr/bin',
    HOME: '/home/harness',
    CARGO_HOME: '/cargo-home',
    CARGO_NET_OFFLINE: 'true',
    CARGO_INCREMENTAL: '0',
  });
  return Object.freeze({
    isolator: createSystemOfflineIsolator({
      writableRoot: options.writableRoot,
      readOnlyMounts: mounts,
      executablePath: options.bwrapExecutable,
      resourceBoundary: options.resourceBoundary,
      resourceLimits: options.resourceLimits,
    }),
    cargoExecutable: '/toolchain/bin/cargo',
    environment,
    readOnlyMounts: Object.freeze(mounts),
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

// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { existsSync, lstatSync, realpathSync } from 'node:fs';

let cachedBwrap: boolean | undefined;
let cachedSystemd: boolean | undefined;

export function nativeIntegrationEnabled(): boolean {
  return process.env.HARNESS_REQUIRE_NATIVE_INTEGRATION !== '0';
}

export function trustedTestNodeExecutable(): string | null {
  const candidates = [...new Set([realpathSync(process.execPath), '/usr/bin/node'])];
  for (const candidate of candidates) {
    if (isTrustedExecutable(candidate)) return candidate;
  }
  if (nativeIntegrationEnabled()) {
    throw new Error('HARNESS_TEST_NATIVE_CAPABILITY_REQUIRED:TRUSTED_NODE');
  }
  return null;
}

export function bwrapAvailable(): boolean {
  if (!nativeIntegrationEnabled()) return false;
  if (cachedBwrap !== undefined) return requireCapability(cachedBwrap, 'BWRAP');
  if (process.platform !== 'linux' || !existsSync('/usr/bin/bwrap')) {
    cachedBwrap = false;
    return requireCapability(cachedBwrap, 'BWRAP');
  }
  const mounts = ['/usr', '/usr/lib', '/usr/lib64'].filter(existsSync);
  const args = ['--unshare-net'];
  for (const source of mounts) {
    const destination = source === '/usr/lib' ? '/lib'
      : source === '/usr/lib64' ? '/lib64' : source;
    args.push('--ro-bind', source, destination);
  }
  cachedBwrap = spawnSync('/usr/bin/bwrap', [
    ...args, '--', '/usr/bin/true',
  ], { stdio: 'ignore' }).status === 0;
  return requireCapability(cachedBwrap, 'BWRAP');
}

export function systemdUserAvailable(): boolean {
  if (!nativeIntegrationEnabled()) return false;
  if (cachedSystemd !== undefined) return requireCapability(cachedSystemd, 'SYSTEMD_USER');
  if (process.platform !== 'linux'
    || !existsSync('/usr/bin/systemd-run')
    || process.env.DBUS_SESSION_BUS_ADDRESS === undefined
    || process.env.XDG_RUNTIME_DIR === undefined) {
    cachedSystemd = false;
    return requireCapability(cachedSystemd, 'SYSTEMD_USER');
  }
  cachedSystemd = spawnSync('/usr/bin/systemd-run', [
    '--user', '--quiet', '--wait', '--collect', '--pipe', '--service-type=exec',
    '--', '/usr/bin/true',
  ], { env: process.env, stdio: 'ignore' }).status === 0;
  return requireCapability(cachedSystemd, 'SYSTEMD_USER');
}

function requireCapability(available: boolean, name: string): boolean {
  if (!available && process.env.HARNESS_REQUIRE_NATIVE_INTEGRATION === '1') {
    throw new Error(`HARNESS_TEST_NATIVE_CAPABILITY_REQUIRED:${name}`);
  }
  return available;
}

function isTrustedExecutable(path: string): boolean {
  if (!existsSync(path)) return false;
  const stat = lstatSync(path, { bigint: true });
  return stat.isFile()
    && !stat.isSymbolicLink()
    && stat.uid === 0n
    && stat.nlink === 1n
    && (stat.mode & 0o111n) !== 0n
    && (stat.mode & 0o022n) === 0n
    && realpathSync(path) === path;
}

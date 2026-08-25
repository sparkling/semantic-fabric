// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';

let cachedBwrap: boolean | undefined;
let cachedSystemd: boolean | undefined;

export function bwrapAvailable(): boolean {
  if (process.env.HARNESS_REQUIRE_NATIVE_INTEGRATION === '0') return false;
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
  if (process.env.HARNESS_REQUIRE_NATIVE_INTEGRATION === '0') return false;
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

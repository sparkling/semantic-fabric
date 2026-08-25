// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { createRustOfflineProfile, bindRustOfflineCommand } from '../src/rust-sandbox.js';
import { fakeResourceBoundary, TEST_RESOURCE_LIMITS } from './helpers.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe(
  'pinned Rust sandbox profile',
  () => {
    it('maps a trusted toolchain, locked registry inputs, and sealed coverage extension', () => {
      const rustup = spawnSync('rustup', ['which', 'cargo'], { encoding: 'utf8' });
      expect(rustup.status, rustup.stderr).toBe(0);
      const cargo = realpathSync(rustup.stdout.trim());
      const toolchain = realpathSync(dirname(dirname(cargo)));
      const registryKey = 'index.crates.io-1949cf8c6b5b557f';
      const registryRoot = mkdtempSync(join(tmpdir(), 'coding-harness-registry-'));
      const writableRoot = mkdtempSync(join(tmpdir(), 'coding-harness-rust-profile-'));
      const extensionRoot = mkdtempSync(join(tmpdir(), 'coding-harness-cargo-extension-'));
      const coverageExecutable = join(extensionRoot, 'cargo-llvm-cov');
      writeFileSync(coverageExecutable, '#!/bin/sh\nexit 0\n');
      chmodSync(coverageExecutable, 0o500);
      roots.push(registryRoot, writableRoot, extensionRoot);
      for (const kind of ['cache', 'index']) {
        mkdirSync(join(registryRoot, kind, registryKey), { recursive: true });
      }
      const profile = createRustOfflineProfile({
        bwrapExecutable: realpathSync('/usr/bin/true'),
        writableRoot,
        cargoExecutable: cargo,
        toolchainRoot: toolchain,
        registryRoot,
        registryKey,
        cargoExtensionRoot: extensionRoot,
        resourceBoundary: fakeResourceBoundary,
        resourceLimits: TEST_RESOURCE_LIMITS,
      });
      const command = bindRustOfflineCommand({
        tool: 'cargo',
        executable: 'cargo',
        argv: ['--offline', 'test', '--locked', '--workspace'],
        cwd: '.',
        env: { CARGO_NET_OFFLINE: 'true' },
        timeoutMs: 1_000,
        maxOutputBytes: 1_000,
      }, profile);

      expect(command.executable).toBe('/toolchain/bin/cargo');
      expect(command.env.CARGO_HOME).toBe('/cargo-home');
      expect(profile.readOnlyMounts.filter(({ destination }) =>
        destination.startsWith('/cargo-home/registry/'))).toHaveLength(2);
      expect(profile.readOnlyMounts.some(({ destination }) =>
        destination.includes('/registry/src/'))).toBe(false);
      expect(profile.readOnlyMounts).toContainEqual({
        source: extensionRoot,
        destination: '/cargo-home/bin',
      });
      expect(profile.environment.PATH).toBe('/cargo-home/bin:/toolchain/bin:/usr/bin');
      expect(profile.readOnlyMounts.some(({ destination }) => destination === '/')).toBe(false);
      profile.isolator.assertStable();
      chmodSync(coverageExecutable, 0o700);
      expect(() => profile.isolator.assertStable()).toThrow(/CARGO_EXTENSION/);
    });
  },
);

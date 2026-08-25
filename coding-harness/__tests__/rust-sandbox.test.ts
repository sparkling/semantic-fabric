// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { createRustOfflineProfile, bindRustOfflineCommand } from '../src/rust-sandbox.js';
import { fakeResourceBoundary, TEST_RESOURCE_LIMITS } from './helpers.js';
import { bwrapAvailable } from './native-test-prerequisites.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe.runIf(bwrapAvailable())(
  'pinned Rust sandbox profile',
  () => {
    it('maps a trusted toolchain and only the exact crates.io cache triplet', () => {
      const rustup = spawnSync('rustup', ['which', 'cargo'], { encoding: 'utf8' });
      expect(rustup.status, rustup.stderr).toBe(0);
      const cargo = realpathSync(rustup.stdout.trim());
      const toolchain = realpathSync(dirname(dirname(cargo)));
      const registryRoot = realpathSync(join(process.env.HOME ?? '', '.cargo/registry'));
      const registryKey = 'index.crates.io-1949cf8c6b5b557f';
      const writableRoot = mkdtempSync(join(tmpdir(), 'coding-harness-rust-profile-'));
      roots.push(writableRoot);
      const profile = createRustOfflineProfile({
        bwrapExecutable: '/usr/bin/bwrap',
        writableRoot,
        cargoExecutable: cargo,
        toolchainRoot: toolchain,
        registryRoot,
        registryKey,
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
        destination.startsWith('/cargo-home/registry/'))).toHaveLength(3);
      expect(profile.readOnlyMounts.some(({ destination }) => destination === '/')).toBe(false);
    });
  },
);

// SPDX-License-Identifier: MIT

import {
  chmodSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { ISSUE_8_MODEL_TIMEOUT_MS } from '../src/issue-8-native-session.js';
import { ISSUE_8_NATIVE_LIMITS } from '../src/issue-8-system.js';
import { createTrustedNativeRuntime } from '../src/native-runtime.js';
import { TEST_RESOURCE_LIMITS, createTestConfig } from './helpers.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('trusted native runtime composition', () => {
  it('nests the issue model deadline inside the systemd runtime boundary', () => {
    expect(ISSUE_8_MODEL_TIMEOUT_MS).toBe(300_000);
    expect(ISSUE_8_MODEL_TIMEOUT_MS).toBeLessThan(
      ISSUE_8_NATIVE_LIMITS.runtimeSeconds * 1_000,
    );
  });

  it('copies only credential capabilities into a private disposable runtime', () => {
    const fixture = createFixture();
    const runtime = createTrustedNativeRuntime({
      config: createTestConfig(),
      runtimeParent: fixture.runtimeParent,
      allowedWorkspaceRoots: [fixture.workspace],
      workspaceRoot: fixture.workspace,
      executables: fixture.executables,
      credentials: fixture.credentials,
      resourceLimits: TEST_RESOURCE_LIMITS,
      controllerEnvironment: controllerEnvironment(),
    });

    expect(runtime.runner.hasTrustedSystemBoundaries()).toBe(true);
    expect(runtime.evidenceRoot.startsWith(runtime.runtimeRoot)).toBe(true);
    expect(statSync(runtime.runtimeRoot).mode & 0o077).toBe(0);
    const codexCopy = join(runtime.runtimeRoot, 'auth/codex/auth.json');
    const claudeCopy = join(runtime.runtimeRoot, 'auth/claude/.credentials.json');
    expect(readFileSync(codexCopy, 'utf8')).toBe('{"auth":"codex"}\n');
    expect(readFileSync(claudeCopy, 'utf8')).toBe('{"auth":"claude"}\n');
    expect(statSync(codexCopy).mode & 0o077).toBe(0);
    expect(statSync(claudeCopy).mode & 0o077).toBe(0);

    runtime.cleanup();
    runtime.cleanup();
    expect(existsSync(runtime.runtimeRoot)).toBe(false);
    expect(existsSync(fixture.credentials.codex)).toBe(true);
    expect(existsSync(fixture.credentials['claude-code'])).toBe(true);
  });

  it('rejects a credential capability visible to other users', () => {
    const fixture = createFixture();
    chmodSync(fixture.credentials.codex, 0o644);
    expect(() => createTrustedNativeRuntime({
      config: createTestConfig(),
      runtimeParent: fixture.runtimeParent,
      allowedWorkspaceRoots: [fixture.workspace],
      workspaceRoot: fixture.workspace,
      executables: fixture.executables,
      credentials: fixture.credentials,
      resourceLimits: TEST_RESOURCE_LIMITS,
      controllerEnvironment: controllerEnvironment(),
    })).toThrow('HARNESS_NATIVE_CODEX_CREDENTIAL_INVALID');
    expect(listRuntimeChildren(fixture.runtimeParent)).toEqual([]);
  });

  it('rejects a forbidden root that overlaps the closed system-library set', () => {
    const fixture = createFixture();
    expect(() => createTrustedNativeRuntime({
      config: createTestConfig(),
      runtimeParent: fixture.runtimeParent,
      allowedWorkspaceRoots: [fixture.workspace],
      workspaceRoot: fixture.workspace,
      executables: fixture.executables,
      credentials: fixture.credentials,
      resourceLimits: TEST_RESOURCE_LIMITS,
      forbiddenMountRoots: ['/usr'],
      controllerEnvironment: controllerEnvironment(),
    })).toThrow('HARNESS_NATIVE_COMMON_MOUNT_OUTSIDE_ALLOWLIST');
    expect(listRuntimeChildren(fixture.runtimeParent)).toEqual([]);
  });

  it('reserves a bounded Unix socket path under a realistic runtime parent', () => {
    const fixture = createFixture();
    const runtimeParent = join(fixture.runtimeParent, 'p'.repeat(18));
    mkdirSync(runtimeParent, { mode: 0o700 });
    const runtime = createTrustedNativeRuntime({
      config: createTestConfig(),
      runtimeParent,
      allowedWorkspaceRoots: [fixture.workspace],
      workspaceRoot: fixture.workspace,
      executables: fixture.executables,
      credentials: fixture.credentials,
      resourceLimits: TEST_RESOURCE_LIMITS,
      controllerEnvironment: controllerEnvironment(),
    });

    const projected = join(runtime.runtimeRoot, 'b', '0'.repeat(16), 'p.sock');
    expect(Buffer.byteLength(projected)).toBeLessThanOrEqual(100);
    runtime.cleanup();
  });
});

function createFixture() {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-runtime-'));
  roots.push(root);
  const runtimeParent = join(root, 'runtime');
  const workspace = join(root, 'workspace');
  const credentialsRoot = join(root, 'host-credentials');
  mkdirSync(runtimeParent, { mode: 0o700 });
  mkdirSync(workspace, { mode: 0o700 });
  mkdirSync(credentialsRoot, { mode: 0o700 });
  const codex = join(credentialsRoot, 'codex.json');
  const claude = join(credentialsRoot, 'claude.json');
  const launcher = join(root, 'launcher.mjs');
  writeFileSync(codex, '{"auth":"codex"}\n', { mode: 0o600 });
  writeFileSync(claude, '{"auth":"claude"}\n', { mode: 0o600 });
  writeFileSync(launcher, 'export {};\n', { mode: 0o600 });
  return {
    runtimeParent,
    workspace,
    credentials: { codex, 'claude-code': claude } as const,
    executables: {
      codex: realpathSync('/bin/true'),
      claude: realpathSync('/bin/true'),
      node: realpathSync(process.execPath),
      bwrap: realpathSync('/bin/true'),
      systemdRun: realpathSync('/bin/true'),
      systemctl: realpathSync('/bin/true'),
      proxyLauncher: launcher,
    },
  };
}

function controllerEnvironment(): Readonly<Record<string, string>> {
  return {
    DBUS_SESSION_BUS_ADDRESS: 'unix:path=/test/user-bus',
    XDG_RUNTIME_DIR: '/test/user-runtime',
  };
}

function listRuntimeChildren(path: string): string[] {
  return existsSync(path) ? readdirSync(path) : [];
}

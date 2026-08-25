// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, mkdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import {
  admitNativeFirstPartyModelTraffic,
  createSystemOfflineIsolator,
  isolateDependencyResolution,
  isolateOfflineCandidateCommand,
  isolateNativeFirstPartyModelTraffic,
} from '../src/network.js';
import type {
  BoundaryCommand,
  OfflineProcessIsolator,
  RegistryOriginPinningBoundary,
} from '../src/network.js';
import { fakeResourceBoundary, TEST_RESOURCE_LIMITS, TEST_RESOURCE_SCOPE } from './helpers.js';
import { bwrapAvailable } from './native-test-prerequisites.js';

const candidateCommand: BoundaryCommand = Object.freeze({
  executable: '/usr/bin/node',
  args: Object.freeze(['--test', 'candidate.test.mjs']),
  cwd: '/workspace/candidate',
  env: Object.freeze({ PATH: '/usr/bin' }),
  writablePaths: Object.freeze([]),
});

const offlineRequest = Object.freeze({
  mode: 'offline',
  channel: 'candidate-command',
  stage: 'candidate-execution',
  deterministic: true,
  allowedOrigins: Object.freeze([]),
  command: candidateCommand,
});

const fakeOfflineIsolator: OfflineProcessIsolator = Object.freeze({
  assertStable() {},
  async terminateAndVerify() {},
  isolate(command: BoundaryCommand) {
    return {
      enforcement: 'os-network-namespace',
      mechanism: 'test-netns',
      resourceScope: TEST_RESOURCE_SCOPE,
      command: {
        ...command,
        executable: '/test/offline-sandbox',
        args: ['--deny-network', '--', command.executable, ...command.args],
      },
    };
  },
});

function nativeRequest(host: 'codex' | 'claude-code'): Record<string, unknown> {
  const codex = host === 'codex';
  return {
    mode: 'first-party-model',
    channel: 'native-subscription-client',
    host,
    authentication: codex ? 'chatgpt-subscription' : 'claude-subscription',
    allowedOrigins: codex
      ? ['https://api.openai.com', 'https://chatgpt.com']
      : ['https://api.anthropic.com', 'https://claude.ai'],
    environment: codex
      ? { PATH: '/usr/bin', CODEX_HOME: '/home/test/.codex' }
      : { PATH: '/usr/bin', CLAUDE_CONFIG_DIR: '/home/test/.claude' },
    transport: {
      client: host,
      provider: codex ? 'openai' : 'anthropic',
      fallback: 'none',
      baseUrl: null,
      proxy: null,
      gateway: null,
    },
  };
}

const dependencyCommand: BoundaryCommand = Object.freeze({
  executable: '/usr/bin/npm',
  args: Object.freeze([
    'ci',
    '--ignore-scripts',
    '--registry',
    'https://registry.npmjs.org/',
    '--audit=false',
    '--fund=false',
  ]),
  cwd: '/workspace/coding-harness',
  env: Object.freeze({ PATH: '/usr/bin' }),
  writablePaths: Object.freeze([]),
});

const temporaryRoots: string[] = [];
afterEach(() => {
  for (const root of temporaryRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function sandboxFixture(): { runRoot: string; cwd: string; output: string } {
  const runRoot = mkdtempSync(join(tmpdir(), 'coding-harness-sandbox-'));
  temporaryRoots.push(runRoot);
  const cwd = join(runRoot, 'candidate');
  const output = join(runRoot, 'outputs', 'candidate');
  mkdirSync(cwd);
  mkdirSync(output, { recursive: true });
  return { runRoot, cwd, output };
}

function dependencyRequest(command: BoundaryCommand = dependencyCommand): Record<string, unknown> {
  return {
    mode: 'dependency-resolution',
    channel: 'dependency-registry',
    stage: 'dependency-resolution',
    registry: 'https://registry.npmjs.org/',
    allowedOrigins: ['https://registry.npmjs.org'],
    command,
  };
}

describe('offline candidate process boundary', () => {
  it('wraps deterministic candidate argv in an injected OS-network isolator', () => {
    const grant = isolateOfflineCandidateCommand(offlineRequest, fakeOfflineIsolator);

    expect(grant).toMatchObject({
      mode: 'offline',
      channel: 'candidate-command',
      stage: 'candidate-execution',
      enforcement: 'os-network-namespace',
      mechanism: 'test-netns',
      allowedOrigins: [],
    });
    expect(grant.command).toEqual({
      ...candidateCommand,
      executable: '/test/offline-sandbox',
      args: ['--deny-network', '--', '/usr/bin/node', '--test', 'candidate.test.mjs'],
    });
    expect(Object.isFrozen(grant)).toBe(true);
    expect(Object.isFrozen(grant.command)).toBe(true);
  });

  it('fails closed for the wrong mode, channel, origins, or a no-op isolator', () => {
    expect(() => isolateOfflineCandidateCommand(
      { ...offlineRequest, mode: 'first-party-model' },
      fakeOfflineIsolator,
    )).toThrow('candidate execution must use offline mode');
    expect(() => isolateOfflineCandidateCommand(
      { ...offlineRequest, channel: 'native-subscription-client' },
      fakeOfflineIsolator,
    )).toThrow('candidate-command channel');
    expect(() => isolateOfflineCandidateCommand(
      { ...offlineRequest, allowedOrigins: ['https://api.openai.com'] },
      fakeOfflineIsolator,
    )).toThrow('must be an empty array');
    expect(() => isolateOfflineCandidateCommand(offlineRequest, {
      assertStable() {},
      async terminateAndVerify() {},
      isolate: (command) => ({
        enforcement: 'os-network-namespace',
        mechanism: 'claimed-netns',
        resourceScope: TEST_RESOURCE_SCOPE,
        command,
      }),
    })).toThrow('HARNESS_OFFLINE_ISOLATOR_DID_NOT_WRAP_COMMAND');
  });

  it('constructs an allowlisted bwrap filesystem without exposing host root', () => {
    const fixture = sandboxFixture();
    const bwrap = createSystemOfflineIsolator({
      platform: 'linux',
      executablePath: '/usr/bin/bwrap',
      writableRoot: fixture.runRoot,
      readOnlyMounts: [{ source: '/usr', destination: '/usr' }],
      resourceBoundary: fakeResourceBoundary,
      resourceLimits: TEST_RESOURCE_LIMITS,
    });
    const request = {
      ...offlineRequest,
      command: {
        ...candidateCommand,
        cwd: fixture.cwd,
        writablePaths: [fixture.output],
      },
    };
    const bwrapGrant = isolateOfflineCandidateCommand(request, bwrap);
    expect(bwrapGrant.command.executable).toBe('/usr/bin/env');
    expect(bwrapGrant.command.args).toEqual(expect.arrayContaining([
      '--unshare-all', '--tmpfs', '/', '--ro-bind', '/usr', '/usr',
      '--ro-bind', fixture.cwd, fixture.cwd, '--bind', fixture.output, fixture.output,
      '--chdir', fixture.cwd, '--', '/usr/bin/node', '--test', 'candidate.test.mjs',
    ]));
    expect(bwrapGrant.command.args.join(' ')).not.toContain('--ro-bind / /');
    expect(bwrapGrant.mechanism).toBe('systemd-cgroup-v2-bwrap');
  });

  it('refuses to run unsandboxed when the platform or isolator is unavailable', () => {
    const fixture = sandboxFixture();
    expect(() => createSystemOfflineIsolator({
      platform: 'linux',
      executablePath: '/does/not/exist/bwrap',
      writableRoot: fixture.runRoot,
      readOnlyMounts: [{ source: '/usr', destination: '/usr' }],
      resourceBoundary: fakeResourceBoundary,
      resourceLimits: TEST_RESOURCE_LIMITS,
    })).toThrow('HARNESS_OFFLINE_OS_SANDBOX_PATH_INVALID');
    expect(() => createSystemOfflineIsolator({
      platform: 'darwin',
      executablePath: '/usr/bin/bwrap',
      writableRoot: fixture.runRoot,
      readOnlyMounts: [{ source: '/usr', destination: '/usr' }],
      resourceBoundary: fakeResourceBoundary,
      resourceLimits: TEST_RESOURCE_LIMITS,
    })).toThrow('HARNESS_OFFLINE_OS_SANDBOX_UNAVAILABLE');
  });

  it.runIf(bwrapAvailable())(
    'runs a real Cargo build offline without exposing an unmounted host secret',
    () => {
      const fixture = sandboxFixture();
      const source = fixture.cwd;
      mkdirSync(join(source, 'src'));
      const secretRoot = mkdtempSync(join(tmpdir(), 'coding-harness-secret-'));
      temporaryRoots.push(secretRoot);
      const secret = join(secretRoot, 'credential.txt');
      writeFileSync(secret, 'must-not-be-readable');
      writeFileSync(join(source, 'Cargo.toml'), [
        '[package]',
        'name = "sandbox-smoke"',
        'version = "0.1.0"',
        'edition = "2024"',
        'build = "build.rs"',
        '',
      ].join('\n'));
      writeFileSync(join(source, 'Cargo.lock'), [
        '# This file is automatically @generated by Cargo.',
        'version = 4',
        '',
        '[[package]]',
        'name = "sandbox-smoke"',
        'version = "0.1.0"',
        '',
      ].join('\n'));
      writeFileSync(join(source, 'src/lib.rs'), 'pub fn answer() -> u8 { 42 }\n');
      writeFileSync(join(source, 'build.rs'), [
        'fn main() {',
        '  let secret = std::env::var("HOST_SECRET_PATH").unwrap();',
        '  assert!(std::fs::read_to_string(secret).is_err());',
        '}',
        '',
      ].join('\n'));

      const rustup = spawnSync('rustup', ['which', 'cargo'], { encoding: 'utf8' });
      expect(rustup.status, rustup.stderr).toBe(0);
      const cargo = realpathSync(rustup.stdout.trim());
      const toolchain = realpathSync(dirname(dirname(cargo)));
      const mounts = [
        { source: '/usr', destination: '/usr' },
        { source: '/usr/lib', destination: '/lib' },
        ...(existsSync('/usr/lib64') ? [{ source: '/usr/lib64', destination: '/lib64' }] : []),
        { source: '/etc/alternatives', destination: '/etc/alternatives' },
        { source: toolchain, destination: toolchain },
      ];
      const isolator = createSystemOfflineIsolator({
        executablePath: '/usr/bin/bwrap',
        writableRoot: fixture.runRoot,
        readOnlyMounts: mounts,
        resourceBoundary: fakeResourceBoundary,
        resourceLimits: TEST_RESOURCE_LIMITS,
      });
      const grant = isolateOfflineCandidateCommand({
        ...offlineRequest,
        command: {
          executable: cargo,
          args: ['check', '--locked', '--offline'],
          cwd: source,
          env: {
            PATH: `${join(toolchain, 'bin')}:/usr/bin`,
            HOME: '/home/harness',
            CARGO_HOME: '/home/harness/.cargo',
            CARGO_TARGET_DIR: fixture.output,
            CARGO_NET_OFFLINE: 'true',
            HOST_SECRET_PATH: secret,
          },
          writablePaths: [fixture.output],
        },
      }, isolator);
      const result = spawnSync(grant.command.executable, [...grant.command.args], {
        cwd: grant.command.cwd,
        env: { ...grant.command.env },
        encoding: 'utf8',
      });
      expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(0);
    },
    30_000,
  );
});

describe('native first-party model network boundary', () => {
  it.each(['codex', 'claude-code'] as const)(
    'admits only exact configured origins for the %s native subscription client',
    (host) => {
      const grant = admitNativeFirstPartyModelTraffic(nativeRequest(host), SECURE_HARNESS_CONFIG);
      expect(grant).toMatchObject({
        mode: 'first-party-model',
        channel: 'native-subscription-client',
        host,
        authenticationEvidence: 'native-client-first-party-auth',
        fallback: 'none',
      });
      expect(Object.isFrozen(grant)).toBe(true);
      expect(Object.isFrozen(grant.allowedOrigins)).toBe(true);
    },
  );

  it('rejects alternate channels, host/auth/provider mismatch, and partial origins', () => {
    expect(() => admitNativeFirstPartyModelTraffic(
      { ...nativeRequest('codex'), channel: 'candidate-command' },
      SECURE_HARNESS_CONFIG,
    )).toThrow('native-subscription-client channel');
    expect(() => admitNativeFirstPartyModelTraffic(
      { ...nativeRequest('codex'), authentication: 'claude-subscription' },
      SECURE_HARNESS_CONFIG,
    )).toThrow('does not match codex');
    expect(() => admitNativeFirstPartyModelTraffic(
      {
        ...nativeRequest('codex'),
        transport: { ...nativeRequest('codex').transport as object, provider: 'anthropic' },
      },
      SECURE_HARNESS_CONFIG,
    )).toThrow('HARNESS_INDIRECT_MODEL_TRANSPORT_PROHIBITED');
    expect(() => admitNativeFirstPartyModelTraffic(
      { ...nativeRequest('codex'), allowedOrigins: ['https://api.openai.com'] },
      SECURE_HARNESS_CONFIG,
    )).toThrow('exact configured host origins');
  });

  it.each([
    { allowedOrigins: ['https://openrouter.ai'] },
    { transport: { ...nativeRequest('codex').transport as object, baseUrl: 'https://openrouter.ai' } },
    { transport: { ...nativeRequest('codex').transport as object, proxy: 'https://requesty.ai' } },
    { transport: { ...nativeRequest('codex').transport as object, gateway: 'https://requesty.ai' } },
    { environment: { PATH: '/usr/bin', OPENROUTER_API_KEY: 'forbidden' } },
    { environment: { PATH: '/usr/bin', REQUESTY_BASE_URL: 'https://requesty.ai' } },
    { environment: { PATH: '/usr/bin', HTTPS_PROXY: 'https://proxy.invalid' } },
  ])('never admits an indirect gateway, base URL, or proxy route', (override) => {
    expect(() => admitNativeFirstPartyModelTraffic(
      { ...nativeRequest('codex'), ...override },
      SECURE_HARNESS_CONFIG,
    )).toThrow();
  });

  it('requires an origin-pinning process wrapper and binds its command', async () => {
    const request = { ...nativeRequest('codex'), command: candidateCommand };
    await expect(isolateNativeFirstPartyModelTraffic(
      request,
      SECURE_HARNESS_CONFIG,
    )).rejects.toThrow('HARNESS_NATIVE_ORIGIN_BOUNDARY_REQUIRED');
    await expect(isolateNativeFirstPartyModelTraffic(
      request,
      SECURE_HARNESS_CONFIG,
      {
        pin: (command, origins) => ({
          enforcement: 'origin-pinned-process-boundary',
          mechanism: 'noop-egress-claim',
          pinnedOrigins: origins,
          command,
        }),
      },
    )).rejects.toThrow('HARNESS_NATIVE_ORIGIN_COMMAND_MISMATCH');

    const grant = await isolateNativeFirstPartyModelTraffic(
      request,
      SECURE_HARNESS_CONFIG,
      {
        pin: (command, origins) => ({
          enforcement: 'origin-pinned-process-boundary',
          mechanism: 'test-egress-firewall',
          pinnedOrigins: origins,
          command: {
            ...command,
            executable: '/usr/bin/env',
            args: [command.executable, ...command.args],
          },
        }),
      },
    );
    expect(grant).toMatchObject({
      enforcement: 'origin-pinned-process-boundary',
      mechanism: 'test-egress-firewall',
    });
  });
});

describe('registry-only dependency process boundary', () => {
  it('requires an injected origin-pinning boundary for the separate npm ci stage', () => {
    expect(() => isolateDependencyResolution(
      dependencyRequest(),
      SECURE_HARNESS_CONFIG,
    )).toThrow('HARNESS_REGISTRY_ORIGIN_BOUNDARY_REQUIRED');

    const pin = vi.fn((command: BoundaryCommand, origins: readonly string[]) => ({
      enforcement: 'origin-pinned-process-boundary',
      mechanism: 'test-egress-firewall',
      pinnedOrigins: origins,
      command: {
        ...command,
        executable: '/usr/bin/env',
        args: [command.executable, ...command.args],
      },
    }));
    const boundary: RegistryOriginPinningBoundary = { pin };
    const grant = isolateDependencyResolution(
      dependencyRequest(),
      SECURE_HARNESS_CONFIG,
      boundary,
    );

    expect(pin).toHaveBeenCalledWith(dependencyCommand, ['https://registry.npmjs.org']);
    expect(grant).toMatchObject({
      mode: 'dependency-resolution',
      channel: 'dependency-registry',
      stage: 'dependency-resolution',
      registry: 'https://registry.npmjs.org/',
      allowedOrigins: ['https://registry.npmjs.org'],
      pinnedOrigins: ['https://registry.npmjs.org'],
      enforcement: 'origin-pinned-process-boundary',
    });
  });

  it('rejects mixed channels, alternate origins, redirecting env, and non-deterministic installs', () => {
    const boundary: RegistryOriginPinningBoundary = {
      pin: (command, origins) => ({
        enforcement: 'origin-pinned-process-boundary',
        mechanism: 'test-egress-firewall',
        pinnedOrigins: origins,
        command: {
          ...command,
          executable: '/usr/bin/env',
          args: [command.executable, ...command.args],
        },
      }),
    };
    expect(() => isolateDependencyResolution(
      { ...dependencyRequest(), channel: 'candidate-command' },
      SECURE_HARNESS_CONFIG,
      boundary,
    )).toThrow('explicit dependency-registry stage');
    expect(() => isolateDependencyResolution(
      { ...dependencyRequest(), allowedOrigins: ['https://requesty.ai'] },
      SECURE_HARNESS_CONFIG,
      boundary,
    )).toThrow('registry-only');
    expect(() => isolateDependencyResolution(
      dependencyRequest({ ...dependencyCommand, env: { NPM_CONFIG_REGISTRY: 'https://requesty.ai' } }),
      SECURE_HARNESS_CONFIG,
      boundary,
    )).toThrow('may redirect network traffic');
    expect(() => isolateDependencyResolution(
      dependencyRequest({ ...dependencyCommand, args: ['install'] }),
      SECURE_HARNESS_CONFIG,
      boundary,
    )).toThrow('dependency command must be');
  });

  it('rejects false origin-pinning evidence', () => {
    expect(() => isolateDependencyResolution(
      dependencyRequest(),
      SECURE_HARNESS_CONFIG,
      {
        pin: (command) => ({
          enforcement: 'origin-pinned-process-boundary',
          mechanism: 'lying-boundary',
          pinnedOrigins: ['https://openrouter.ai'],
          command: {
            ...command,
            executable: '/usr/bin/env',
            args: [command.executable, ...command.args],
          },
        }),
      },
    )).toThrow('unexpected origin');

    expect(() => isolateDependencyResolution(
      dependencyRequest(),
      SECURE_HARNESS_CONFIG,
      {
        pin: () => ({
          enforcement: 'origin-pinned-process-boundary',
          mechanism: 'mismatched-boundary',
          pinnedOrigins: ['https://registry.npmjs.org'],
          command: { ...dependencyCommand, executable: '/usr/bin/curl', args: [] },
        }),
      },
    )).toThrow('HARNESS_REGISTRY_ORIGIN_COMMAND_MISMATCH');
  });
});

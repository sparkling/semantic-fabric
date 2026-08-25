// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import { isolateNativeModelFilesystem } from '../src/native-filesystem.js';
import {
  SystemNativeFilesystemBoundary,
  systemNativeRuntimeLibraryMounts,
} from '../src/native-system-filesystem.js';
import type { BoundaryCommand } from '../src/network.js';
import { bwrapAvailable } from './native-test-prerequisites.js';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('system native filesystem mount validation', () => {
  it('exposes only the closed system-runtime mount set', () => {
    expect(systemNativeRuntimeLibraryMounts()).toEqual([
      { source: '/usr/lib', destination: '/usr/lib' },
      { source: '/usr/lib', destination: '/lib' },
      { source: '/usr/lib64', destination: '/usr/lib64' },
      { source: '/usr/lib64', destination: '/lib64' },
      {
        source: '/etc/ssl/certs/ca-certificates.crt',
        destination: '/etc/ssl/certs/ca-certificates.crt',
      },
    ]);
    expect(Object.isFrozen(systemNativeRuntimeLibraryMounts())).toBe(true);
  });

  it('rejects arbitrary directory and relocated runtime-file mounts', () => {
    const fixture = validationFixture();
    expect(() => new SystemNativeFilesystemBoundary({
      ...fixture.options,
      hosts: {
        ...fixture.options.hosts,
        codex: {
          ...fixture.options.hosts.codex,
          runtimeMounts: [{
            source: fixture.controllerRoot,
            destination: fixture.controllerRoot,
          }],
        },
      },
    })).toThrow('HARNESS_NATIVE_codex:RUNTIME_MOUNT_OUTSIDE_ALLOWLIST');
    expect(() => new SystemNativeFilesystemBoundary({
      ...fixture.options,
      hosts: {
        ...fixture.options.hosts,
        codex: {
          ...fixture.options.hosts.codex,
          runtimeMounts: [{ source: fixture.runtimeFile, destination: '/home/harness/runtime' }],
        },
      },
    })).toThrow('HARNESS_NATIVE_codex:RUNTIME_MOUNT_OUTSIDE_ALLOWLIST');
    expect(() => new SystemNativeFilesystemBoundary({
      ...fixture.options,
      forbiddenMountRoots: [fixture.controllerRoot, '/usr'],
    })).toThrow('HARNESS_NATIVE_COMMON_MOUNT_OUTSIDE_ALLOWLIST');
  });

  it('rejects a host credential mounted instead of its private copied capability', () => {
    const fixture = validationFixture();
    const hostCredential = join(fixture.controllerRoot, 'host-auth.json');
    writeFileSync(hostCredential, 'host secret\n', { mode: 0o600 });
    expect(() => new SystemNativeFilesystemBoundary({
      ...fixture.options,
      hosts: {
        ...fixture.options.hosts,
        codex: {
          ...fixture.options.hosts.codex,
          authenticationMounts: [{
            source: hostCredential,
            destination: '/home/harness/.codex/auth.json',
          }],
        },
      },
    })).toThrow('HARNESS_NATIVE_codex:AUTH_MOUNT_OUTSIDE_ALLOWLIST');
  });
});

describe.runIf(bwrapAvailable())('system native filesystem boundary', () => {
  it('mounts only the active broker session and hides host/configuration paths', async () => {
    const root = privateRoot();
    const workspace = join(root, 'workspace');
    const credentialRoot = join(root, 'host-credential');
    const brokerRoot = join(root, 'broker');
    const brokerSession = join(brokerRoot, 'session-one');
    mkdirSync(join(workspace, '.git'), { recursive: true, mode: 0o700 });
    mkdirSync(credentialRoot, { mode: 0o700 });
    mkdirSync(brokerSession, { recursive: true, mode: 0o700 });
    writeFileSync(join(workspace, 'visible.txt'), 'visible\n');
    writeFileSync(join(workspace, '.git/config'), 'host git metadata\n');
    writeFileSync(join(workspace, '.mcp.json'), '{"mcpServers":{"unsafe":{}}}\n');
    writeFileSync(join(workspace, 'sealed-evaluator.rs'), 'oracle law\n');
    const output = join(workspace, 'result.json');
    writeFileSync(output, '', { mode: 0o600 });
    const hostCredential = join(credentialRoot, 'host.json');
    const copiedCredential = join(root, 'copied.json');
    writeFileSync(hostCredential, 'host secret\n', { mode: 0o600 });
    writeFileSync(copiedCredential, 'runtime capability\n', { mode: 0o600 });
    const probe = join(root, 'probe.mjs');
    writeFileSync(probe, filesystemProbe(), { mode: 0o600 });
    const socket = join(brokerSession, 'p.sock');
    const server = createServer();
    await new Promise<void>((ready, reject) => {
      server.once('error', reject);
      server.listen(socket, ready);
    });

    try {
      const node = realpathSync(process.execPath);
      const launcher = realpathSync(join(harnessRoot, 'dist/native-proxy-launcher.js'));
      const boundary = new SystemNativeFilesystemBoundary({
        bwrapExecutable: realpathSync('/usr/bin/bwrap'),
        brokerRoot,
        allowedRuntimeFiles: [node, launcher, probe],
        authenticationSourceRoot: root,
        forbiddenMountRoots: [credentialRoot],
        hosts: {
          codex: {
            authenticationMounts: [{
              source: copiedCredential,
              destination: '/home/harness/.codex/auth.json',
            }],
            runtimeMounts: [node, launcher, probe]
              .map((path) => ({ source: path, destination: path })),
            privateEnvironment: {
              HOME: '/home/harness', CODEX_HOME: '/home/harness/.codex',
            },
          },
          'claude-code': {
            authenticationMounts: [{
              source: copiedCredential,
              destination: '/home/harness/.claude/.credentials.json',
            }],
            runtimeMounts: [node, launcher, probe]
              .map((path) => ({ source: path, destination: path })),
            privateEnvironment: {
              HOME: '/home/harness', CLAUDE_CONFIG_DIR: '/home/harness/.claude',
            },
          },
        },
      });
      const command: BoundaryCommand = {
        executable: node,
        args: [
          launcher, '--broker-socket', socket, '--',
          node, probe, output, hostCredential,
        ],
        cwd: workspace,
        env: {},
        writablePaths: [output],
      };
      const isolated = isolateNativeModelFilesystem(command, {
        host: 'codex',
        workspaceRoot: workspace,
        readOnlyRoots: [workspace],
        writablePaths: [output],
        maskedPaths: [
          join(workspace, '.git'),
          join(workspace, '.mcp.json'),
          join(workspace, 'sealed-evaluator.rs'),
        ],
        hostFileConfidentiality: true,
        emptyPrivateHome: true,
        hostRootMounted: false,
      }, boundary);
      const executed = spawnSync(isolated.command.executable, [...isolated.command.args], {
        cwd: isolated.command.cwd,
        env: {},
        encoding: 'utf8',
      });
      expect(executed.status, executed.stderr).toBe(0);
      expect(JSON.parse(readFileSync(output, 'utf8'))).toEqual({
        visible: 'visible',
        gitMasked: true,
        mcpMasked: true,
        evaluatorMasked: true,
        hostCredentialHidden: true,
        runtimeCredential: 'runtime capability',
        caBundleVisible: true,
        home: '/home/harness',
      });
      expect(isolated.mountManifestDigest).toMatch(/^[a-f0-9]{64}$/);
      expect(isolated.configurationMaskDigest).toMatch(/^[a-f0-9]{64}$/);
      expect(isolated).toMatchObject({
        hostFileConfidentiality: true,
        emptyPrivateHome: true,
        privateEphemeralHome: true,
        hostRootMounted: false,
        hostCredentialPathMounted: false,
      });
    } finally {
      await new Promise<void>((closed) => server.close(() => closed()));
    }
  });
});

function privateRoot(): string {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-filesystem-'));
  roots.push(root);
  return root;
}

function validationFixture() {
  const root = privateRoot();
  const brokerRoot = join(root, 'broker');
  const authenticationSourceRoot = join(root, 'auth');
  const controllerRoot = join(root, 'controller');
  mkdirSync(brokerRoot, { mode: 0o700 });
  mkdirSync(authenticationSourceRoot, { mode: 0o700 });
  mkdirSync(controllerRoot, { mode: 0o700 });
  const runtimeFile = join(root, 'runtime.mjs');
  const codexAuth = join(authenticationSourceRoot, 'codex.json');
  const claudeAuth = join(authenticationSourceRoot, 'claude.json');
  writeFileSync(runtimeFile, 'export {};\n', { mode: 0o600 });
  writeFileSync(codexAuth, 'codex\n', { mode: 0o600 });
  writeFileSync(claudeAuth, 'claude\n', { mode: 0o600 });
  const options = {
    bwrapExecutable: realpathSync('/bin/true'),
    brokerRoot,
    allowedRuntimeFiles: [runtimeFile],
    authenticationSourceRoot,
    forbiddenMountRoots: [controllerRoot],
    hosts: {
      codex: {
        authenticationMounts: [{
          source: codexAuth, destination: '/home/harness/.codex/auth.json',
        }],
        runtimeMounts: [{ source: runtimeFile, destination: runtimeFile }],
        privateEnvironment: { HOME: '/home/harness', CODEX_HOME: '/home/harness/.codex' },
      },
      'claude-code': {
        authenticationMounts: [{
          source: claudeAuth, destination: '/home/harness/.claude/.credentials.json',
        }],
        runtimeMounts: [{ source: runtimeFile, destination: runtimeFile }],
        privateEnvironment: {
          HOME: '/home/harness', CLAUDE_CONFIG_DIR: '/home/harness/.claude',
        },
      },
    },
  } as const;
  return { controllerRoot, options, runtimeFile };
}

function filesystemProbe(): string {
  return `import { readFileSync, writeFileSync } from 'node:fs';
const [output, hostCredential] = process.argv.slice(2);
const hidden = (path) => { try { readFileSync(path); return false; } catch { return true; } };
const masked = (path) => { try {
  return !readFileSync(path, 'utf8').includes('unsafe');
} catch { return true; } };
writeFileSync(output, JSON.stringify({
  visible: readFileSync('visible.txt', 'utf8').trim(),
  gitMasked: hidden('.git/config'),
  mcpMasked: masked('.mcp.json'),
  evaluatorMasked: hidden('sealed-evaluator.rs'),
  hostCredentialHidden: hidden(hostCredential),
  runtimeCredential: readFileSync('/home/harness/.codex/auth.json', 'utf8').trim(),
  caBundleVisible: readFileSync('/etc/ssl/certs/ca-certificates.crt', 'utf8')
    .includes('BEGIN CERTIFICATE'),
  home: process.env.HOME,
}));
`;
}

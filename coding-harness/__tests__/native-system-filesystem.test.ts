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
import { SystemNativeFilesystemBoundary } from '../src/native-system-filesystem.js';
import type { BoundaryCommand } from '../src/network.js';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('system native filesystem boundary', () => {
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
        commonRuntimeMounts: [
          { source: '/usr/lib', destination: '/usr/lib' },
          { source: '/usr/lib', destination: '/lib' },
          { source: '/usr/lib64', destination: '/usr/lib64' },
          { source: '/usr/lib64', destination: '/lib64' },
          { source: node, destination: node },
          { source: launcher, destination: launcher },
          { source: probe, destination: probe },
        ],
        hosts: {
          codex: {
            authenticationMounts: [{
              source: copiedCredential,
              destination: '/home/harness/.codex/auth.json',
            }],
            runtimeMounts: [{ source: node, destination: node }],
            privateEnvironment: {
              HOME: '/home/harness', CODEX_HOME: '/home/harness/.codex',
            },
          },
          'claude-code': {
            authenticationMounts: [{
              source: copiedCredential,
              destination: '/home/harness/.claude/.credentials.json',
            }],
            runtimeMounts: [{ source: node, destination: node }],
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
        maskedPaths: [join(workspace, '.git'), join(workspace, '.mcp.json')],
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
        hostCredentialHidden: true,
        runtimeCredential: 'runtime capability',
        home: '/home/harness',
      });
      expect(isolated.mountManifestDigest).toMatch(/^[a-f0-9]{64}$/);
      expect(isolated.configurationMaskDigest).toMatch(/^[a-f0-9]{64}$/);
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
  hostCredentialHidden: hidden(hostCredential),
  runtimeCredential: readFileSync('/home/harness/.codex/auth.json', 'utf8').trim(),
  home: process.env.HOME,
}));
`;
}

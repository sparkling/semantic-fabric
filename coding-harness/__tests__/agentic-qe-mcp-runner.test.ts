// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import type { ProviderFreeAgenticQeMcpRequest } from '../src/agentic-qe-lcov.js';
import { SystemAgenticQeMcpRunner } from '../src/agentic-qe-mcp-runner.js';
import type { ProviderFreeAgenticQeSastMcpRequest } from '../src/agentic-qe-sast.js';

type Behavior = 'success' | 'wrong-id' | 'rpc-error' | 'hang' | 'flood';

interface Fixture {
  readonly root: string;
  readonly candidateRoot: string;
  readonly lcovPath: string;
  readonly packageRoot: string;
  readonly bundlePath: string;
  readonly nodePath: string;
  readonly bwrapPath: string;
  readonly boundaryObservation: string;
  readonly mcpObservation: string;
  readonly request: ProviderFreeAgenticQeMcpRequest;
}

const roots: string[] = [];

afterEach(async () => {
  await Promise.all(roots.splice(0).map(async (root) => await rm(root, {
    recursive: true,
    force: true,
  })));
});

describe('system Agentic-QE MCP runner', () => {
  it('runs the exact MCP lifecycle inside a provider-free offline bwrap profile', async () => {
    const fixture = await createFixture('success');
    const runner = createRunner(fixture);
    const result = await runner.invoke(fixture.request) as { content: unknown[] };
    const boundary = await jsonFile(fixture.boundaryObservation);
    const mcp = await jsonFile(fixture.mcpObservation);

    expect(result.content).toHaveLength(1);
    expect(boundary.wrapperEnvironment).toEqual({});
    expect(boundary.flags).toEqual(expect.arrayContaining([
      '--die-with-parent', '--new-session', '--unshare-all', '--unshare-net', '--clearenv',
    ]));
    expect(boundary.mounts).toEqual(expect.arrayContaining([
      [fixture.candidateRoot, fixture.candidateRoot],
      [fixture.lcovPath, fixture.lcovPath],
      [fixture.packageRoot, fixture.packageRoot],
      [fixture.nodePath, fixture.nodePath],
    ]));
    expect(boundary.environment).toMatchObject({
      HOME: '/home/harness',
      TMPDIR: '/tmp',
      PATH: '/nonexistent',
      AQE_MEMORY_BACKEND: 'memory',
      AQE_LLM_ROUTER_DISABLED: '1',
      AQE_SESSION_CACHE: 'off',
      AQE_LOOP_DETECTION_ENABLED: 'false',
    });
    expect(Object.keys(mcp.environment)).not.toEqual(expect.arrayContaining([
      'OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'OPENROUTER_API_KEY', 'HTTPS_PROXY',
    ]));
    expect(mcp.messages).toMatchObject([
      { jsonrpc: '2.0', id: 1, method: 'initialize' },
      { jsonrpc: '2.0', method: 'initialized', params: {} },
      {
        jsonrpc: '2.0',
        id: 2,
        method: 'tools/call',
        params: { name: 'qe/coverage/gaps', arguments: fixture.request.arguments },
      },
      { jsonrpc: '2.0', id: 3, method: 'shutdown', params: {} },
    ]);
    expect(runner.identityEvidence()).toMatchObject({
      node: { path: fixture.nodePath, sha256: expect.stringMatching(/^[a-f0-9]{64}$/) },
      bwrap: { path: fixture.bwrapPath, sha256: expect.stringMatching(/^[a-f0-9]{64}$/) },
      package: {
        root: fixture.packageRoot,
        entryPath: fixture.bundlePath,
        name: 'agentic-qe',
        version: '3.13.10-test',
        treeSha256: expect.stringMatching(/^[a-f0-9]{64}$/),
      },
    });
  });

  it('discovers and runs only the advertised comprehensive SAST profile', async () => {
    const fixture = await createFixture('success');
    const runner = createRunner(fixture);
    const request = sastRequest(fixture.candidateRoot);
    await runner.invoke(request);
    const boundary = await jsonFile(fixture.boundaryObservation);
    const mcp = await jsonFile(fixture.mcpObservation);

    expect(boundary.mounts).toEqual(expect.arrayContaining([
      [fixture.candidateRoot, fixture.candidateRoot],
      [fixture.packageRoot, fixture.packageRoot],
      [fixture.nodePath, fixture.nodePath],
    ]));
    expect(boundary.mounts).not.toContainEqual([fixture.lcovPath, fixture.lcovPath]);
    expect(mcp.messages).toMatchObject([
      { jsonrpc: '2.0', id: 1, method: 'initialize' },
      { jsonrpc: '2.0', method: 'initialized', params: {} },
      { jsonrpc: '2.0', id: 2, method: 'tools/list', params: {} },
      {
        jsonrpc: '2.0',
        id: 3,
        method: 'tools/call',
        params: { name: 'security_scan_comprehensive', arguments: request.arguments },
      },
      { jsonrpc: '2.0', id: 4, method: 'shutdown', params: {} },
    ]);
    expect(runner.commandDigest(request)).toMatch(/^[a-f0-9]{64}$/);
  });

  it.each([
    ['package', 'HARNESS_AGENTIC_QE_PACKAGE_CHANGED'],
    ['node', 'HARNESS_AGENTIC_QE_NODE_CHANGED'],
    ['bwrap', 'HARNESS_AGENTIC_QE_BWRAP_CHANGED'],
  ] as const)('rejects %s identity tampering before launch', async (target, error) => {
    const fixture = await createFixture('success');
    const runner = createRunner(fixture);
    const path = target === 'package' ? fixture.bundlePath
      : target === 'node' ? fixture.nodePath : fixture.bwrapPath;
    const original = await readFile(path, 'utf8');
    await writeFile(path, `${original}\n// tampered\n`);
    await chmod(path, 0o700);

    await expect(runner.invoke(fixture.request)).rejects.toThrow(error);
  });

  it.each([
    ['wrong-id', 'HARNESS_AGENTIC_QE_MCP_RESPONSE_ID_INVALID'],
    ['rpc-error', 'HARNESS_AGENTIC_QE_MCP_RPC_ERROR:2:-32000:forced failure'],
  ] as const)('fails closed on %s protocol output', async (behavior, error) => {
    const fixture = await createFixture(behavior);
    await expect(createRunner(fixture).invoke(fixture.request)).rejects.toThrow(error);
  });

  it('kills the MCP process group on timeout', async () => {
    const fixture = await createFixture('hang');
    await expect(createRunner(fixture, { hardTimeoutMs: 1_000 }).invoke(fixture.request))
      .rejects.toThrow('HARNESS_AGENTIC_QE_MCP_TIMEOUT');
    const observation = await jsonFile(fixture.mcpObservation);
    expect(() => process.kill(observation.pid as number, 0)).toThrow();
  });

  it('kills the MCP process group when combined output exceeds the hard ceiling', async () => {
    const fixture = await createFixture('flood');
    await expect(createRunner(fixture, { maxOutputBytes: 1024 }).invoke(fixture.request))
      .rejects.toThrow('HARNESS_AGENTIC_QE_MCP_OUTPUT_LIMIT_EXCEEDED');
  });

  it('rejects undeclared provider variables at the runner boundary', async () => {
    const fixture = await createFixture('success');
    const poisoned = structuredClone(fixture.request) as unknown as Record<string, any>;
    poisoned.runtime.environment.variables.OPENAI_API_KEY = 'sealed-value';

    await expect(createRunner(fixture).invoke(
      poisoned as unknown as ProviderFreeAgenticQeMcpRequest,
    )).rejects.toThrow(/Agentic-QE MCP variables has invalid keys/);
  });
});

function createRunner(
  fixture: Fixture,
  limits: Readonly<{ hardTimeoutMs?: number; maxOutputBytes?: number }> = {},
): SystemAgenticQeMcpRunner {
  return new SystemAgenticQeMcpRunner({
    nodeExecutable: fixture.nodePath,
    aqeMcpExecutable: fixture.bundlePath,
    aqePackageRoot: fixture.packageRoot,
    bwrapExecutable: fixture.bwrapPath,
    hardTimeoutMs: limits.hardTimeoutMs,
    maxOutputBytes: limits.maxOutputBytes,
    terminationGraceMs: 50,
    maxPackageFiles: 20,
    maxPackageBytes: 1_000_000,
  });
}

async function createFixture(behavior: Behavior): Promise<Fixture> {
  const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-aqe-mcp-'));
  roots.push(root);
  const candidateRoot = join(root, 'candidate');
  const packageRoot = join(root, 'agentic-qe');
  const bundlePath = join(packageRoot, 'dist/mcp/bundle.js');
  const nodePath = join(root, 'node');
  const bwrapPath = join(root, 'bwrap');
  const boundaryObservation = join(root, 'boundary.json');
  const mcpObservation = join(root, 'mcp.json');
  await mkdir(join(candidateRoot, 'coverage'), { recursive: true });
  await mkdir(join(packageRoot, 'dist/mcp'), { recursive: true });
  const lcovPath = join(candidateRoot, 'coverage/lcov.info');
  const lcov = 'TN:\nSF:src/lib.rs\nDA:1,1\nDA:2,0\nend_of_record\n';
  await writeFile(lcovPath, lcov);
  await writeFile(join(packageRoot, 'package.json'), JSON.stringify({
    name: 'agentic-qe',
    version: '3.13.10-test',
    type: 'module',
    bin: { 'aqe-mcp': './dist/mcp/bundle.js' },
  }));
  await writeFile(bundlePath, fakeMcpSource(behavior, mcpObservation));
  await writeFile(nodePath, '#!/bin/sh\nexec /usr/bin/node "$@"\n');
  await writeFile(bwrapPath, fakeBwrapSource(boundaryObservation));
  await Promise.all([bundlePath, nodePath, bwrapPath].map(async (path) => await chmod(path, 0o700)));
  const request: ProviderFreeAgenticQeMcpRequest = {
    executable: 'aqe-mcp',
    transport: 'stdio-mcp',
    method: 'tools/call',
    toolName: 'qe/coverage/gaps',
    arguments: {
      target: candidateRoot,
      coverageFile: lcovPath,
      coverageFormat: 'lcov',
      language: 'rust',
      minRisk: 0,
      limit: 20,
      prioritization: 'complexity',
      includeGhost: false,
    },
    bindings: {
      taskId: 'task-aqe-0001',
      runId: 'run-aqe-0001',
      candidateTree: 'b'.repeat(40),
      lcovSha256: sha256(lcov),
      coverageCommandDigest: 'c'.repeat(64),
      generatorVersion: 'cargo-llvm-cov 0.8.7',
    },
    runtime: {
      network: 'offline',
      environment: {
        inheritance: 'none',
        variables: {
          AQE_MEMORY_BACKEND: 'memory',
          AQE_LLM_ROUTER_DISABLED: '1',
          AQE_SESSION_CACHE: 'off',
          AQE_LOOP_DETECTION_ENABLED: 'false',
        },
      },
      filesystem: {
        inputAccess: 'read-only',
        readOnlyPaths: [candidateRoot, lcovPath].sort(),
        privateHome: true,
        privateWritableTmp: true,
      },
      timeoutMs: 120_000,
      maxOutputBytes: 5_000_000,
    },
  };
  return {
    root, candidateRoot, lcovPath, packageRoot, bundlePath, nodePath, bwrapPath,
    boundaryObservation, mcpObservation, request,
  };
}

function fakeBwrapSource(observation: string): string {
  return `#!/usr/bin/node
const { spawn } = require('node:child_process');
const { writeFileSync } = require('node:fs');
const args = process.argv.slice(2), flags = [], mounts = [], environment = {};
let cwd = '/', command = '', commandArgs = [];
for (let index = 0; index < args.length;) {
  const arg = args[index];
  if (arg === '--') { command = args[index + 1]; commandArgs = args.slice(index + 2); break; }
  if (arg === '--setenv') { environment[args[index + 1]] = args[index + 2]; index += 3; continue; }
  if (arg === '--ro-bind') { mounts.push([args[index + 1], args[index + 2]]); index += 3; continue; }
  if (arg === '--chdir') { cwd = args[index + 1]; index += 2; continue; }
  if (['--tmpfs','--hostname','--cap-drop','--dir','--dev','--proc'].includes(arg)) { index += 2; continue; }
  flags.push(arg); index += 1;
}
writeFileSync(${JSON.stringify(observation)}, JSON.stringify({ flags, mounts, environment, wrapperEnvironment: process.env }));
const child = spawn(command, commandArgs, { cwd, env: environment, stdio: 'inherit' });
child.on('close', (code, signal) => process.exit(code ?? (signal ? 1 : 0)));
child.on('error', () => process.exit(127));
`;
}

function fakeMcpSource(behavior: Behavior, observation: string): string {
  return `#!/usr/bin/node
import { writeFileSync } from 'node:fs';
import { createInterface } from 'node:readline';
const behavior = ${JSON.stringify(behavior)}, messages = [];
const record = () => writeFileSync(${JSON.stringify(observation)}, JSON.stringify({ pid: process.pid, environment: process.env, messages }));
const send = value => process.stdout.write(JSON.stringify(value) + '\\n');
for await (const line of createInterface({ input: process.stdin })) {
  const message = JSON.parse(line); messages.push(message); record();
  if (message.method === 'initialize') send({ jsonrpc: '2.0', id: 1, result: {
    protocolVersion: '2025-11-25', capabilities: { tools: { listChanged: true }, logging: {} },
    serverInfo: { name: 'agentic-qe-v3', version: '3.13.10-test', protocolVersion: '2025-11-25' }
  }});
  if (message.method === 'tools/list') send({ jsonrpc: '2.0', id: message.id, result: { tools: [{
    name: 'security_scan_comprehensive', description: 'Run SAST scans', inputSchema: {
      type: 'object', properties: {
        sast: { type: 'boolean', description: 'Run SAST scan', default: true },
        dast: { type: 'boolean', description: 'Run DAST scan', default: false },
        target: { type: 'string', description: 'Target to scan' }
      }
    }
  }] } });
  if (message.method === 'tools/call') {
    if (behavior === 'hang') continue;
    if (behavior === 'flood') { process.stderr.write('x'.repeat(5000)); continue; }
    if (behavior === 'rpc-error') { send({ jsonrpc: '2.0', id: message.id, error: { code: -32000, message: 'forced failure' } }); continue; }
    send({ jsonrpc: '2.0', id: behavior === 'wrong-id' ? 99 : message.id, result: { content: [{ type: 'text', text: '{"success":true}' }] } });
  }
  if (message.method === 'shutdown') { send({ jsonrpc: '2.0', id: message.id, result: {} }); process.stdin.unref(); setTimeout(() => process.exit(0), 10); }
}
`;
}

async function jsonFile(path: string): Promise<Record<string, any>> {
  return JSON.parse(await readFile(path, 'utf8')) as Record<string, any>;
}

function sha256(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

function sastRequest(target: string): ProviderFreeAgenticQeSastMcpRequest {
  return {
    executable: 'aqe-mcp',
    transport: 'stdio-mcp',
    method: 'tools/call',
    toolName: 'security_scan_comprehensive',
    arguments: {
      target,
      sast: true,
      dast: false,
    },
    bindings: {
      taskId: 'task-aqe-sast-0001',
      runId: 'run-aqe-sast-0001',
      candidateTree: 'd'.repeat(40),
      snapshotSha256: 'e'.repeat(64),
      profile: 'sast-only-flat-v1',
    },
    runtime: {
      network: 'offline',
      environment: {
        inheritance: 'none',
        variables: {
          AQE_MEMORY_BACKEND: 'memory',
          AQE_LLM_ROUTER_DISABLED: '1',
          AQE_SESSION_CACHE: 'off',
          AQE_LOOP_DETECTION_ENABLED: 'false',
        },
      },
      filesystem: {
        inputAccess: 'read-only',
        readOnlyPaths: [target],
        privateHome: true,
        privateWritableTmp: true,
      },
      timeoutMs: 120_000,
      maxOutputBytes: 5_000_000,
    },
  };
}

// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { createSocket, type Socket as DatagramSocket } from 'node:dgram';
import { readlinkSync } from 'node:fs';
import {
  chmod, link, mkdir, mkdtemp, readdir, rm, stat, symlink, writeFile,
} from 'node:fs/promises';
import { createServer, type Server } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  PROGRAMME_V5_RUFLO_CLI_IDENTITY,
  PROGRAMME_V5_RUFLO_NODE_IDENTITY,
  collectProgrammeV5RufloEvidence,
  stableProgrammeV5RufloFileDigest,
} from '../src/programme-v5-ruflo.js';
import {
  createProgrammeV5RufloPrivateRuntime,
  type ProgrammeV5RufloPrivateRuntime,
}
  from '../src/programme-v5-ruflo-runtime.js';
import { bwrapAvailable } from './native-test-prerequisites.js';

const nativeIt = bwrapAvailable() ? it : it.skip;

describe('programme v5 local Ruflo MCP collector', () => {
  it('rejects a non-canonical configured package source', async () => {
    const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-ruflo-source-'));
    const variable = 'SF_HARNESS_RUFLO_PACKAGE_ROOT';
    const original = process.env[variable];
    try {
      process.env[variable] = 'relative/package';
      expect(() => createProgrammeV5RufloPrivateRuntime(root))
        .toThrow('HARNESS_PROGRAMME_V5_RUFLO_RUNTIME_PACKAGE_ROOT_INVALID');
    } finally {
      if (original === undefined) delete process.env[variable];
      else process.env[variable] = original;
      await rm(root, { recursive: true, force: true });
    }
  });

  nativeIt('queries exact persisted task and swarm IDs through the pinned local stdio server', async () => {
    const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-ruflo-'));
    const taskId = 'coordination-test-0001';
    const swarmId = 'swarm-test-0001';
    const now = new Date().toISOString();
    const originalOpenRouter = process.env.OPENROUTER_API_KEY;
    const originalOpenAi = process.env.OPENAI_API_KEY;
    try {
      await mkdir(join(root, '.claude-flow', 'tasks'), { recursive: true });
      await mkdir(join(root, '.claude-flow', 'swarm'), { recursive: true });
      await writeFile(join(root, '.claude-flow', 'tasks', 'store.json'), JSON.stringify({
        version: '3.0.0',
        tasks: {
          [taskId]: {
            taskId,
            type: 'feature',
            description: 'Exercise the real pinned stdio MCP collector',
            priority: 'critical',
            status: 'in_progress',
            progress: 25,
            assignedTo: ['collector-test'],
            tags: ['schema-v5'],
            createdAt: now,
            startedAt: now,
            completedAt: null,
          },
        },
      }));
      await writeFile(join(root, '.claude-flow', 'swarm', 'swarm-state.json'), JSON.stringify({
        version: '3.0.0',
        swarms: {
          [swarmId]: {
            swarmId,
            topology: 'hierarchical',
            maxAgents: 4,
            status: 'running',
            agents: [],
            tasks: [taskId],
            config: {
              topology: 'hierarchical',
              maxAgents: 4,
              strategy: 'specialized',
              communicationProtocol: 'message-bus',
              autoScaling: false,
              consensusMechanism: 'raft',
            },
            createdAt: now,
            updatedAt: now,
          },
        },
      }));
      process.env.OPENROUTER_API_KEY = 'must-not-cross-the-collector-boundary';
      process.env.OPENAI_API_KEY = 'must-not-cross-the-collector-boundary';

      const evidence = await collectProgrammeV5RufloEvidence({
        repositoryRoot: root,
        taskId: 'programme-task-0001',
        runId: 'programme-run-0001',
        routeSnapshotDigest: 'a'.repeat(64),
        captureNonce: 'b'.repeat(64),
        transactionStartedAt: now,
        swarmId,
        coordinationTaskId: taskId,
        hookIds: ['hook-route-0001'],
        traceIds: ['trace-run-0001'],
        timeoutMs: 20_000,
      });

      expect(evidence.schemaVersion).toBe(2);
      expect(evidence.captureNonce).toBe('b'.repeat(64));
      expect(evidence.transactionStartedAt).toBe(now);
      expect(evidence.captureBindingDigest).toMatch(/^[a-f0-9]{64}$/);
      expect(evidence.cli).toEqual(PROGRAMME_V5_RUFLO_CLI_IDENTITY);
      expect(evidence.cli).toMatchObject({
        nodePath: PROGRAMME_V5_RUFLO_NODE_IDENTITY.path,
        nodeDigest: PROGRAMME_V5_RUFLO_NODE_IDENTITY.digest,
      });
      expect(evidence.taskStatus).toMatchObject({ taskId, status: 'in_progress' });
      expect(evidence.swarmStatus).toMatchObject({
        swarmId,
        status: 'running',
        topology: 'hierarchical',
        config: { strategy: 'specialized', consensusMechanism: 'raft' },
      });
      expect(evidence.taskStatusRequest.params.arguments).toEqual({ taskId });
      expect(evidence.swarmStatusRequest.params.arguments).toEqual({ swarmId });
      expect(evidence.taskStatusDigest).toMatch(/^[a-f0-9]{64}$/);
      expect(evidence.swarmStatusDigest).toMatch(/^[a-f0-9]{64}$/);
      expect(evidence.providerVariablesStripped).toBe(true);
      expect(evidence.authoritative).toBe(false);
    } finally {
      if (originalOpenRouter === undefined) delete process.env.OPENROUTER_API_KEY;
      else process.env.OPENROUTER_API_KEY = originalOpenRouter;
      if (originalOpenAi === undefined) delete process.env.OPENAI_API_KEY;
      else process.env.OPENAI_API_KEY = originalOpenAi;
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  nativeIt('uses a private network namespace and snapshots only regular ledger files', async () => {
    const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-ruflo-runtime-'));
    let cleanup: (() => void) | undefined;
    try {
      await mkdir(join(root, '.claude-flow', 'tasks'), { recursive: true });
      await mkdir(join(root, '.claude-flow', 'swarm'), { recursive: true });
      await writeFile(join(root, '.claude-flow', 'tasks', 'store.json'), '{"version":"3.0.0","tasks":{}}');
      await writeFile(join(root, '.claude-flow', 'swarm', 'swarm-state.json'), '{"version":"3.0.0","swarms":{}}');

      const runtime = createProgrammeV5RufloPrivateRuntime(root);
      cleanup = runtime.cleanup;
      expect(runtime.environment).not.toHaveProperty('SF_HARNESS_RUFLO_PACKAGE_ROOT');
      expect(runtime.args).toContain('--unshare-net');
      expect(runtime.args).not.toContain('--share-net');
      expect(runtime.args.filter((value) => value === '--unshare-net')).toHaveLength(1);
      expect(runtime.args).not.toContain(join(root, '.claude-flow', 'tasks'));
      expect(runtime.args).not.toContain(join(root, '.claude-flow', 'swarm'));
      expect(runtime.args.some((value) => value.endsWith('/ledger/tasks'))).toBe(true);
      expect(runtime.args.some((value) => value.endsWith('/ledger/swarm'))).toBe(true);
      for (const destination of [
        '/workspace/.claude-flow/tasks', '/workspace/.claude-flow/swarm',
      ]) {
        const source = runtime.args[runtime.args.lastIndexOf(destination) - 1]!;
        const identity = await stat(source);
        expect(identity.mode & 0o777).toBe(0o500);
        const file = await stat(join(
          source,
          destination.endsWith('/tasks') ? 'store.json' : 'swarm-state.json',
        ));
        expect(file.mode & 0o777).toBe(0o400);
        expect(file.nlink).toBe(1);
      }
      cleanup();
      cleanup = undefined;
      await chmod(join(root, '.claude-flow', 'tasks', 'store.json'), 0o666);
      expect(() => createProgrammeV5RufloPrivateRuntime(root))
        .toThrow('HARNESS_IMMUTABLE_RUNTIME_SOURCE_UNTRUSTED');
    } finally {
      cleanup?.();
      await rm(root, { recursive: true, force: true });
    }
  });

  nativeIt('accepts cooperative group-write but rejects alternate exact ledger objects', async () => {
    const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-ruflo-ledger-'));
    const tasks = join(root, '.claude-flow', 'tasks');
    const taskStore = join(tasks, 'store.json');
    const sibling = join(tasks, 'ignored.json');
    const swarm = join(root, '.claude-flow', 'swarm', 'swarm-state.json');
    let runtime: ProgrammeV5RufloPrivateRuntime | undefined;
    let socket: Server | undefined;
    try {
      await mkdir(tasks, { recursive: true });
      await mkdir(join(root, '.claude-flow', 'swarm'), { recursive: true });
      await writeFile(taskStore, '{"version":"3.0.0","tasks":{}}');
      await writeFile(sibling, '{"ignored":true}');
      await writeFile(swarm, '{"version":"3.0.0","swarms":{}}');
      await chmod(taskStore, 0o664);
      runtime = createProgrammeV5RufloPrivateRuntime(root);
      const taskDestination = runtime.args.lastIndexOf('/workspace/.claude-flow/tasks');
      const privateTasks = runtime.args[taskDestination - 1]!;
      expect(await readdir(privateTasks)).toEqual(['store.json']);
      runtime.cleanup();
      runtime = undefined;

      await rm(taskStore);
      await link(sibling, taskStore);
      expect(() => createProgrammeV5RufloPrivateRuntime(root))
        .toThrow('HARNESS_IMMUTABLE_RUNTIME_SOURCE_UNTRUSTED');

      await rm(taskStore);
      await symlink('ignored.json', taskStore);
      expect(() => createProgrammeV5RufloPrivateRuntime(root))
        .toThrow('HARNESS_IMMUTABLE_RUNTIME_SOURCE_INVALID');

      await rm(taskStore);
      socket = createServer((connection) => connection.destroy());
      await listen(socket, taskStore);
      expect(() => createProgrammeV5RufloPrivateRuntime(root))
        .toThrow('HARNESS_IMMUTABLE_RUNTIME_SOURCE_UNTRUSTED');
      await closeServer(socket);
      socket = undefined;
      await rm(taskStore, { force: true });

      const fifo = spawnSync('mkfifo', [taskStore], { encoding: 'utf8' });
      expect(fifo.status, fifo.stderr).toBe(0);
      expect(() => createProgrammeV5RufloPrivateRuntime(root))
        .toThrow('HARNESS_IMMUTABLE_RUNTIME_SOURCE_UNTRUSTED');
    } finally {
      runtime?.cleanup();
      await closeServer(socket);
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  nativeIt('fails when a cooperative ledger source changes during the full snapshot', async () => {
    const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-ruflo-race-'));
    const taskStore = join(root, '.claude-flow', 'tasks', 'store.json');
    let runtime: ProgrammeV5RufloPrivateRuntime | undefined;
    let mutator: ReturnType<typeof spawn> | undefined;
    let completed: Promise<number | null> | undefined;
    try {
      await mkdir(join(root, '.claude-flow', 'tasks'), { recursive: true });
      await mkdir(join(root, '.claude-flow', 'swarm'), { recursive: true });
      await writeFile(taskStore, '{"version":"3.0.0","tasks":{}}');
      await writeFile(
        join(root, '.claude-flow', 'swarm', 'swarm-state.json'),
        '{"version":"3.0.0","swarms":{}}',
      );
      mutator = spawn(process.execPath, ['-e', ledgerMutator, taskStore], {
        stdio: 'ignore',
      });
      completed = new Promise<number | null>((resolveClose) => {
        mutator.once('close', (code) => resolveClose(code));
      });
      expect(() => {
        runtime = createProgrammeV5RufloPrivateRuntime(root);
      }).toThrow('HARNESS_IMMUTABLE_RUNTIME_SOURCE_CHANGED');
      expect(await completed).toBe(0);
    } finally {
      runtime?.cleanup();
      if (mutator?.exitCode === null && mutator.signalCode === null) mutator.kill('SIGTERM');
      await completed;
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  nativeIt('blocks host TCP and source-directory Unix sockets with sensitive controls', async () => {
    const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-ruflo-net-'));
    const taskDirectory = join(root, '.claude-flow', 'tasks');
    const socketPath = join(taskDirectory, 'escape.sock');
    let runtime: ProgrammeV5RufloPrivateRuntime | undefined;
    let tcp: Server | undefined;
    let unix: Server | undefined;
    let dns: DatagramSocket | undefined;
    let dnsPackets = 0;
    try {
      await mkdir(taskDirectory, { recursive: true });
      await mkdir(join(root, '.claude-flow', 'swarm'), { recursive: true });
      await writeFile(join(taskDirectory, 'store.json'), '{"version":"3.0.0","tasks":{}}');
      await writeFile(join(root, '.claude-flow', 'swarm', 'swarm-state.json'), '{"version":"3.0.0","swarms":{}}');
      tcp = createServer((socket) => socket.destroy());
      unix = createServer((socket) => socket.destroy());
      dns = createSocket('udp4');
      dns.on('message', () => { dnsPackets += 1; });
      await listen(tcp, 0, '127.0.0.1');
      await listen(unix, socketPath);
      await bindDatagram(dns);
      const address = tcp.address();
      const dnsAddress = dns.address();
      if (address === null || typeof address === 'string') throw new Error('TCP_CONTROL_INVALID');
      runtime = createProgrammeV5RufloPrivateRuntime(root);

      const isolatedNamespace = probe(runtime, runtime.args, namespaceProbe, []);
      expect(isolatedNamespace.status, isolatedNamespace.stderr).toBe(0);
      expect(isolatedNamespace.stdout.trim()).not.toBe(readlinkSync('/proc/self/ns/net'));

      const sealed = probe(runtime, runtime.args, blockedProbe, [
        String(address.port), '/workspace/.claude-flow/tasks/escape.sock',
      ]);
      expect(sealed.status, sealed.stderr).toBe(0);

      const sharedNetwork = runtime.args.map((argument) =>
        argument === '--unshare-net' ? '--share-net' : argument);
      const tcpControl = probe(runtime, sharedNetwork, tcpProbe, [String(address.port)]);
      expect(tcpControl.status, tcpControl.stderr).toBe(0);
      expect(tcpControl.stdout).toBe('connected\n');
      const sharedNamespace = probe(runtime, sharedNetwork, namespaceProbe, []);
      expect(sharedNamespace.status, sharedNamespace.stderr).toBe(0);
      expect(sharedNamespace.stdout.trim()).toBe(readlinkSync('/proc/self/ns/net'));

      const isolatedDns = probe(runtime, runtime.args, dnsProbe, [String(dnsAddress.port)]);
      expect(isolatedDns.status, isolatedDns.stderr).toBe(0);
      await settleCanary();
      expect(dnsPackets).toBe(0);
      const sharedDns = probe(runtime, sharedNetwork, dnsProbe, [String(dnsAddress.port)]);
      expect(sharedDns.status, sharedDns.stderr).toBe(0);
      await settleCanary();
      expect(dnsPackets).toBeGreaterThan(0);

      const liveDirectory = [...runtime.args];
      const taskDestination = liveDirectory.lastIndexOf('/workspace/.claude-flow/tasks');
      liveDirectory[taskDestination - 1] = taskDirectory;
      const unixControl = probe(runtime, liveDirectory, unixProbe, [
        '/workspace/.claude-flow/tasks/escape.sock',
      ]);
      expect(unixControl.status, unixControl.stderr).toBe(0);
      expect(unixControl.stdout).toBe('connected\n');
    } finally {
      runtime?.cleanup();
      await closeServer(tcp);
      await closeServer(unix);
      await closeDatagram(dns);
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it('rejects writable and multiply linked local runtime files', async () => {
    const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-ruflo-file-'));
    const file = join(root, 'entry.js');
    try {
      await writeFile(file, 'process.exit(0);\n', { mode: 0o700 });
      expect(stableProgrammeV5RufloFileDigest(file, true)).toBe(
        createHash('sha256').update('process.exit(0);\n').digest('hex'),
      );
      await chmod(file, 0o720);
      expect(() => stableProgrammeV5RufloFileDigest(file, true)).toThrow(/LOCAL_FILE_UNTRUSTED/);
      await chmod(file, 0o700);
      await link(file, join(root, 'entry-hardlink.js'));
      expect(() => stableProgrammeV5RufloFileDigest(file, true)).toThrow(/LOCAL_FILE_UNTRUSTED/);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});

const blockedProbe = [
  "const net=require('node:net');let pending=2,unexpected=false;",
  "const done=()=>{if(--pending===0)process.exit(unexpected?1:0)};",
  "const check=(socket)=>{socket.once('connect',()=>{unexpected=true;socket.destroy();done()});",
  "socket.once('error',done)};",
  "check(net.connect(Number(process.argv[1]),'127.0.0.1'));",
  "check(net.connect(process.argv[2]));setTimeout(()=>process.exit(2),3000).unref();",
].join('');
const tcpProbe = [
  "const socket=require('node:net').connect(Number(process.argv[1]),'127.0.0.1');",
  "socket.once('connect',()=>{console.log('connected');socket.destroy()});",
  "socket.once('error',()=>process.exit(1));setTimeout(()=>process.exit(2),3000).unref();",
].join('');
const unixProbe = [
  "const socket=require('node:net').connect(process.argv[1]);",
  "socket.once('connect',()=>{console.log('connected');socket.destroy()});",
  "socket.once('error',()=>process.exit(1));setTimeout(()=>process.exit(2),3000).unref();",
].join('');
const namespaceProbe = "console.log(require('node:fs').readlinkSync('/proc/self/ns/net'))";
const dnsProbe = [
  "const dns=require('node:dns');const resolver=new dns.Resolver();",
  "resolver.setServers([`127.0.0.1:${process.argv[1]}`]);",
  "resolver.resolve4('semantic-fabric-canary.invalid',()=>{});",
  "setTimeout(()=>process.exit(0),300);",
].join('');
const ledgerMutator = [
  "const fs=require('node:fs');const path=process.argv[1];let count=0;",
  "setTimeout(()=>{const timer=setInterval(()=>{count+=1;",
  "fs.writeFileSync(path,JSON.stringify({version:'3.0.0',tasks:{count}}));",
  "if(count===300)clearInterval(timer)},5)},25);",
].join('');

function probe(
  runtime: ProgrammeV5RufloPrivateRuntime,
  sandboxArgs: readonly string[],
  script: string,
  args: readonly string[],
) {
  const separator = sandboxArgs.indexOf('--');
  if (separator < 0) throw new Error('SANDBOX_SEPARATOR_MISSING');
  return spawnSync(runtime.executable, [
    ...sandboxArgs.slice(0, separator + 1), '/runtime/node', '-e', script, ...args,
  ], {
    cwd: runtime.cwd, env: runtime.environment, encoding: 'utf8', timeout: 5_000,
    maxBuffer: 16_384,
  });
}

async function listen(server: Server, ...args: [number, string] | [string]): Promise<void> {
  await new Promise<void>((resolveListen, reject) => {
    const failed = (error: Error) => reject(error);
    server.once('error', failed);
    server.listen(...args, () => {
      server.off('error', failed);
      resolveListen();
    });
  });
}

async function closeServer(server: Server | undefined): Promise<void> {
  if (server === undefined || !server.listening) return;
  await new Promise<void>((resolveClose) => server.close(() => resolveClose()));
}

async function bindDatagram(socket: DatagramSocket): Promise<void> {
  await new Promise<void>((resolveBind, reject) => {
    const failed = (error: Error) => reject(error);
    socket.once('error', failed);
    socket.bind(0, '127.0.0.1', () => {
      socket.off('error', failed);
      resolveBind();
    });
  });
}

async function closeDatagram(socket: DatagramSocket | undefined): Promise<void> {
  if (socket === undefined) return;
  await new Promise<void>((resolveClose) => socket.close(() => resolveClose()));
}

async function settleCanary(): Promise<void> {
  await new Promise<void>((resolveSettle) => setTimeout(resolveSettle, 50));
}

// SPDX-License-Identifier: MIT

import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  isolateNativeResources,
  limitsForProcessDeadline,
  SystemdResourceBoundary,
  type NativeResourceLimits,
} from '../src/resource-boundary.js';
import { systemdUserAvailable } from './native-test-prerequisites.js';

const roots: string[] = [];
const limits: NativeResourceLimits = Object.freeze({
  memoryBytes: 64 * 1024 * 1024,
  processCount: 4,
  cpuQuotaPercent: 100,
  cpuTimeSeconds: 5,
  runtimeSeconds: 5,
  fileBytes: 1024 * 1024,
  openFiles: 32,
});

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('native process deadline resource limits', () => {
  it('reserves cgroup cleanup headroom inside the declared hard ceiling', () => {
    const bounded = limitsForProcessDeadline({
      ...limits,
      cpuTimeSeconds: 7_200,
      runtimeSeconds: 1_800,
    }, 1_200_000, 500);

    expect(bounded.runtimeSeconds).toBe(1_201);
    expect(bounded.runtimeSeconds).toBeLessThan(1_800);
    expect(() => limitsForProcessDeadline(limits, 5_000, 1_000))
      .toThrow('HARNESS_NATIVE_RESOURCE_DEADLINE_NOT_NESTED');
  });
});

describe.runIf(systemdUserAvailable())('systemd cgroup v2 resource boundary', () => {
  it('applies the declared cgroup and rlimit contract to a real transient service', () => {
    const root = privateRoot();
    const boundary = systemBoundary();
    const isolated = isolateNativeResources({
      executable: '/usr/bin/sh',
      args: ['-c', 'ulimit -n; cat /proc/self/cgroup'],
      cwd: root,
      env: {},
      writablePaths: [],
    }, limits, boundary);
    expect(isolated.command.args).toEqual(expect.arrayContaining([
      expect.stringMatching(/^--unit=semantic-fabric-harness-[0-9a-f-]{36}\.service$/),
      `--property=MemoryMax=${limits.memoryBytes}`,
      '--property=MemorySwapMax=0',
      `--property=TasksMax=${limits.processCount}`,
      `--property=CPUQuota=${limits.cpuQuotaPercent}%`,
      `--property=LimitFSIZE=${limits.fileBytes}`,
      `--property=LimitNOFILE=${limits.openFiles}`,
      '--property=KillMode=control-group',
      '--property=ExitType=cgroup',
      '--property=KillSignal=SIGTERM',
      '--property=FinalKillSignal=SIGKILL',
      '--property=SendSIGKILL=yes',
      '--property=TimeoutStopSec=100ms',
    ]));
    const result = spawnSync(isolated.command.executable, [...isolated.command.args], {
      cwd: root,
      env: { ...boundary.launchEnvironment(isolated.command.env) },
      encoding: 'utf8',
    });
    expect(result.status, result.stderr).toBe(0);
    const [openFiles, cgroup] = result.stdout.trim().split('\n');
    expect(openFiles).toBe(String(limits.openFiles));
    expect(cgroup).toMatch(/^0::\/user\.slice\//);
    expect(cgroup).not.toBe('0::/');
  });

  it('fails a process that breaches the file-size ceiling', () => {
    const root = privateRoot();
    const output = join(root, 'oversized.bin');
    const boundary = systemBoundary();
    const isolated = isolateNativeResources({
      executable: '/usr/bin/dd',
      args: ['if=/dev/zero', `of=${output}`, 'bs=1048576', 'count=2', 'status=none'],
      cwd: root,
      env: {},
      writablePaths: [output],
    }, limits, boundary);
    const result = spawnSync(isolated.command.executable, [...isolated.command.args], {
      cwd: root,
      env: { ...boundary.launchEnvironment(isolated.command.env) },
      encoding: 'utf8',
    });
    expect(result.status).not.toBe(0);
    expect(statSync(output).size).toBeLessThanOrEqual(limits.fileBytes);
  });

  it('waits for a TERM-ignoring descendant to leave the exact cgroup', async () => {
    const root = privateRoot();
    const mainPidFile = join(root, 'main.pid');
    const childPidFile = join(root, 'child.pid');
    const boundary = systemBoundary();
    const isolated = isolateNativeResources({
      executable: '/usr/bin/sh',
      args: ['-c', [
        'trap "" TERM',
        `printf '%s' "$$" > '${mainPidFile}'`,
        `/usr/bin/setsid /usr/bin/sh -c 'trap "" TERM; printf "%s" "$$" > "${childPidFile}"; while :; do /usr/bin/sleep 1; done' &`,
        `while test ! -s '${childPidFile}'; do /usr/bin/sleep 0.01; done`,
        'wait',
      ].join('\n')],
      cwd: root,
      env: {},
      writablePaths: [mainPidFile, childPidFile],
    }, { ...limits, processCount: 16, runtimeSeconds: 30 }, boundary);
    const child = spawn(isolated.command.executable, [...isolated.command.args], {
      cwd: root,
      env: { ...boundary.launchEnvironment(isolated.command.env) },
      stdio: 'ignore',
    });
    try {
      await waitForActiveUnit(isolated.scope.unit);
      await waitForFile(mainPidFile);
      await waitForFile(childPidFile);
      const pids = [mainPidFile, childPidFile].map((path) => readFileSync(path, 'utf8').trim());
      const started = Date.now();
      await boundary.terminateAndVerify(isolated.scope);
      expect(Date.now() - started).toBeGreaterThanOrEqual(50);
      await waitForClose(child);
      expect(pids.every((pid) => !existsSync(`/proc/${pid}`))).toBe(true);
      await boundary.terminateAndVerify(isolated.scope);
    } finally {
      if (child.exitCode === null) {
        try { await boundary.terminateAndVerify(isolated.scope); } catch { /* test cleanup */ }
        child.kill('SIGKILL');
      }
    }
  });
});

function systemBoundary(): SystemdResourceBoundary {
  return new SystemdResourceBoundary({
    executablePath: '/usr/bin/systemd-run',
    systemctlPath: '/usr/bin/systemctl',
    terminationGraceMs: 100,
    sourceEnvironment: process.env,
  });
}

function privateRoot(): string {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-resources-'));
  roots.push(root);
  return root;
}

async function waitForActiveUnit(unit: string): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const status = spawnSync('/usr/bin/systemctl', [
      '--user', 'show', '--property=ActiveState', '--value', '--', unit,
    ], { encoding: 'utf8' });
    if (status.status === 0 && status.stdout.trim() === 'active') return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(`transient unit did not become active: ${unit}`);
}

async function waitForFile(path: string): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (existsSync(path) && statSync(path).size > 0) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(`process evidence did not appear: ${path}`);
}

async function waitForClose(child: ReturnType<typeof spawn>): Promise<number | null> {
  if (child.exitCode !== null || child.signalCode !== null) return child.exitCode;
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('transient service did not stop')), 5_000);
    timeout.unref();
    child.once('close', (code) => {
      clearTimeout(timeout);
      resolve(code);
    });
  });
}

// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  isolateNativeResources,
  SystemdResourceBoundary,
  type NativeResourceLimits,
} from '../src/resource-boundary.js';

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

describe('systemd cgroup v2 resource boundary', () => {
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
      `--property=MemoryMax=${limits.memoryBytes}`,
      '--property=MemorySwapMax=0',
      `--property=TasksMax=${limits.processCount}`,
      `--property=CPUQuota=${limits.cpuQuotaPercent}%`,
      `--property=LimitFSIZE=${limits.fileBytes}`,
      `--property=LimitNOFILE=${limits.openFiles}`,
      '--property=KillMode=control-group',
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
});

function systemBoundary(): SystemdResourceBoundary {
  return new SystemdResourceBoundary({
    executablePath: '/usr/bin/systemd-run',
    sourceEnvironment: process.env,
  });
}

function privateRoot(): string {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-resources-'));
  roots.push(root);
  return root;
}

// SPDX-License-Identifier: MIT

import { once } from 'node:events';
import {
  mkdtempSync,
  readdirSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { createConnection } from 'node:net';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  digestOriginBrokerEvents,
  originBrokerTimeoutIsPolicyDenial,
  pinnedTcpConnectionOptions,
  resolvePublicDnsHost,
  selectPublicDnsAddress,
  summarizeOriginBrokerEvents,
  UnixSocketOriginPinningBoundary,
  type DnsAddress,
} from '../src/native-egress.js';
import type { BoundaryCommand } from '../src/network.js';

const { lookup } = vi.hoisted(() => ({
  lookup: vi.fn(async () => [{ address: '1.1.1.1', family: 4 }]),
}));
vi.mock('node:dns/promises', () => ({ lookup }));

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('native exact-origin egress resolution', () => {
  it('separates policy denials from allowed-origin transport failures', () => {
    const admitted = { attemptId: 1, target: 'api.openai.com|4|1.1.1.1' } as const;
    expect(summarizeOriginBrokerEvents([
      { ...admitted, outcome: 'origin-admitted' },
      { ...admitted, outcome: 'transport-connected' },
      { ...admitted, outcome: 'transport-error' },
      { attemptId: 2, target: 'chatgpt.com|4|1.0.0.1', outcome: 'transport-connected' },
    ])).toEqual({
      allowedConnections: 2,
      deniedConnections: 0,
      transportErrors: 1,
      boundaryErrors: 0,
    });
    expect(summarizeOriginBrokerEvents([
      { attemptId: 1, target: 'forbidden.example', outcome: 'policy-denied' },
      { ...admitted, outcome: 'transport-error' },
      { attemptId: 2, target: 'broker', outcome: 'boundary-error' },
    ])).toEqual({
      allowedConnections: 0,
      deniedConnections: 1,
      transportErrors: 1,
      boundaryErrors: 1,
    });
    expect(originBrokerTimeoutIsPolicyDenial(0)).toBe(false);
    expect(originBrokerTimeoutIsPolicyDenial(1)).toBe(true);

    const connectedThenFailed = [
      { ...admitted, outcome: 'transport-connected' as const },
      { ...admitted, outcome: 'transport-error' as const },
      { attemptId: 2, target: admitted.target, outcome: 'origin-admitted' as const },
    ];
    const separateFailure = [
      { ...admitted, outcome: 'transport-connected' as const },
      { attemptId: 2, target: admitted.target, outcome: 'origin-admitted' as const },
      { attemptId: 2, target: admitted.target, outcome: 'transport-error' as const },
    ];
    expect(digestOriginBrokerEvents(connectedThenFailed))
      .not.toBe(digestOriginBrokerEvents(separateFailure));
  });

  it('records a partial CONNECT closed before timeout as a policy denial', async () => {
    const brokerRoot = privateRoot('coding-harness-egress-partial-');
    const launcher = join(brokerRoot, 'launcher.mjs');
    writeFileSync(launcher, 'export {};\n', { mode: 0o600 });
    const boundary = new UnixSocketOriginPinningBoundary({
      brokerRoot,
      nodeExecutable: realpathSync(process.execPath),
      launcherPath: launcher,
      connectionTimeoutMs: 1_000,
    });
    const grant = await boundary.pin(command(brokerRoot), ['https://api.openai.com']);
    const socketPath = grant.command.args[2];
    expect(socketPath).toBeTypeOf('string');
    const client = createConnection(socketPath as string);
    await once(client, 'connect');
    client.end('CONN');
    await once(client, 'close');
    await new Promise((resolveTurn) => setImmediate(resolveTurn));

    const completion = await boundary.complete(grant.command);
    expect(completion.allowedConnections).toBe(0);
    expect(completion.deniedConnections).toBe(1);
  });

  it('selects a stable public address and produces a numeric TCP target', () => {
    const selected = selectPublicDnsAddress('api.openai.com', [
      { address: '2606:4700:4700::1111', family: 6 },
      { address: '93.184.216.35', family: 4 },
      { address: '93.184.216.34', family: 4 },
      { address: '93.184.216.34', family: 4 },
    ]);

    expect(selected).toEqual({ address: '93.184.216.34', family: 4 });
    expect(pinnedTcpConnectionOptions(selected)).toEqual({
      host: '93.184.216.34',
      port: 443,
      family: 4,
    });
    expect(Object.isFrozen(selected)).toBe(true);
  });

  it.each([
    ['0.1.2.3', 4],
    ['10.1.2.3', 4],
    ['100.64.0.1', 4],
    ['127.0.0.1', 4],
    ['169.254.1.1', 4],
    ['172.16.0.1', 4],
    ['192.0.2.1', 4],
    ['192.168.1.1', 4],
    ['198.18.0.1', 4],
    ['198.51.100.1', 4],
    ['203.0.113.1', 4],
    ['224.0.0.1', 4],
    ['255.255.255.255', 4],
    ['::', 6],
    ['::1', 6],
    ['::ffff:127.0.0.1', 6],
    ['64:ff9b::808:808', 6],
    ['100::1', 6],
    ['2001::1', 6],
    ['2001:db8::1', 6],
    ['2002::1', 6],
    ['3fff::1', 6],
    ['5f00::1', 6],
    ['fc00::1', 6],
    ['fe80::1', 6],
    ['fec0::1', 6],
    ['ff02::1', 6],
  ] as const)('rejects private or reserved DNS answer %s', (address, family) => {
    expect(() => selectPublicDnsAddress('api.openai.com', [{ address, family }]))
      .toThrow('HARNESS_NATIVE_EGRESS_DNS_NON_PUBLIC');
  });

  it('rejects the entire DNS result when any answer is non-public', () => {
    expect(() => selectPublicDnsAddress('chatgpt.com', [
      { address: '93.184.216.34', family: 4 },
      { address: '127.0.0.1', family: 4 },
    ])).toThrow('HARNESS_NATIVE_EGRESS_DNS_NON_PUBLIC');
  });

  it('rejects empty, malformed, and mismatched DNS answers', () => {
    expect(() => selectPublicDnsAddress('claude.ai', []))
      .toThrow('HARNESS_NATIVE_EGRESS_DNS_EMPTY');
    expect(() => selectPublicDnsAddress('claude.ai', [
      { address: 'not-an-address', family: 4 },
    ])).toThrow('HARNESS_NATIVE_EGRESS_DNS_ADDRESS_INVALID');
    expect(() => selectPublicDnsAddress('claude.ai', [
      { address: '2606:4700:4700::1111', family: 4 },
    ])).toThrow('HARNESS_NATIVE_EGRESS_DNS_ADDRESS_INVALID');
  });

  it('normalizes DNS hostnames, wraps resolver failure, and rejects IP literals first', async () => {
    const resolver = vi.fn(async (hostname: string): Promise<readonly DnsAddress[]> => {
      expect(hostname).toBe('api.anthropic.com');
      return [{ address: '2001:4860:4860::8888', family: 6 }];
    });
    await expect(resolvePublicDnsHost('API.ANTHROPIC.COM', resolver)).resolves.toEqual({
      address: '2001:4860:4860::8888',
      family: 6,
    });
    expect(resolver).toHaveBeenCalledOnce();

    await expect(resolvePublicDnsHost('claude.ai', async () => {
      throw new Error('resolver unavailable');
    })).rejects.toThrow('HARNESS_NATIVE_EGRESS_DNS_FAILED:claude.ai');

    const literalResolver = vi.fn(async (): Promise<readonly DnsAddress[]> => []);
    await expect(resolvePublicDnsHost('8.8.8.8', literalResolver))
      .rejects.toThrow('HARNESS_NATIVE_EGRESS_LITERAL_ORIGIN_PROHIBITED');
    expect(literalResolver).not.toHaveBeenCalled();
  });

  it('rejects a public IP origin before creating a broker session', async () => {
    const brokerRoot = privateRoot('coding-harness-egress-');
    const launcher = join(brokerRoot, 'launcher.mjs');
    writeFileSync(launcher, 'export {};\n', { mode: 0o600 });
    const boundary = new UnixSocketOriginPinningBoundary({
      brokerRoot,
      nodeExecutable: realpathSync(process.execPath),
      launcherPath: launcher,
    });

    await expect(boundary.pin(command(brokerRoot), ['https://8.8.8.8']))
      .rejects.toThrow('HARNESS_NATIVE_EGRESS_LITERAL_ORIGIN_PROHIBITED');
    expect(readdirSync(brokerRoot)).toEqual(['launcher.mjs']);
  });

  it('refuses to construct numeric connection options from a non-public address', () => {
    expect(() => pinnedTcpConnectionOptions({ address: '127.0.0.1', family: 4 }))
      .toThrow('HARNESS_NATIVE_EGRESS_PINNED_ADDRESS_INVALID');
  });
});

function privateRoot(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  roots.push(root);
  return root;
}

function command(cwd: string): BoundaryCommand {
  return {
    executable: realpathSync(process.execPath),
    args: ['--version'],
    cwd,
    env: { PATH: '/usr/bin' },
    writablePaths: [],
  };
}

// SPDX-License-Identifier: MIT

import { createHash, randomBytes } from 'node:crypto';
import { lookup as dnsLookup } from 'node:dns/promises';
import {
  chmodSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
} from 'node:fs';
import {
  BlockList,
  createConnection,
  createServer,
  isIP,
  type Server,
  type Socket,
} from 'node:net';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';
import { deepFreeze, normalizePublicHttpsOrigin } from './contracts.js';
import type {
  BoundaryCommand,
  NativeModelOriginPinningBoundary,
  RegistryPinResult,
} from './network.js';

export interface UnixOriginBoundaryOptions {
  readonly brokerRoot: string;
  readonly nodeExecutable: string;
  readonly launcherPath: string;
  readonly connectionTimeoutMs?: number;
  readonly maxHeaderBytes?: number;
}

export interface OriginBoundaryCompletion {
  readonly allowedConnections: number;
  readonly deniedConnections: number;
  readonly connectDigest: string;
}

interface Session {
  readonly directory: string;
  readonly server: Server;
  readonly sockets: Set<Socket>;
  readonly requested: string[];
  allowed: number;
  denied: number;
}

export interface DnsAddress {
  readonly address: string;
  readonly family: 4 | 6;
}

export type DnsResolver = (hostname: string) => Promise<readonly DnsAddress[]>;
export const NATIVE_EGRESS_SOCKET_PATH_LIMIT = 100;

const IPV4_NON_PUBLIC = addressBlockList([
  ['0.0.0.0', 8],
  ['10.0.0.0', 8],
  ['100.64.0.0', 10],
  ['127.0.0.0', 8],
  ['169.254.0.0', 16],
  ['172.16.0.0', 12],
  ['192.0.0.0', 24],
  ['192.0.2.0', 24],
  ['192.88.99.0', 24],
  ['192.168.0.0', 16],
  ['198.18.0.0', 15],
  ['198.51.100.0', 24],
  ['203.0.113.0', 24],
  ['224.0.0.0', 4],
  ['240.0.0.0', 4],
], 'ipv4');

const IPV6_NON_PUBLIC = addressBlockList([
  ['::', 128],
  ['::1', 128],
  ['::ffff:0:0', 96],
  ['64:ff9b::', 96],
  ['64:ff9b:1::', 48],
  ['100::', 64],
  ['2001::', 23],
  ['2001:db8::', 32],
  ['2002::', 16],
  ['3fff::', 20],
  ['5f00::', 16],
  ['fc00::', 7],
  ['fe80::', 10],
  ['fec0::', 10],
  ['ff00::', 8],
], 'ipv6');

export class UnixSocketOriginPinningBoundary implements NativeModelOriginPinningBoundary {
  readonly #brokerRoot: string;
  readonly #node: FileIdentity;
  readonly #launcher: FileIdentity;
  readonly #connectionTimeoutMs: number;
  readonly #maxHeaderBytes: number;
  readonly #sessions = new Map<string, Session>();

  constructor(options: UnixOriginBoundaryOptions) {
    this.#brokerRoot = validatePrivateDirectory(options.brokerRoot);
    this.#node = validateExecutable(options.nodeExecutable, 'NODE');
    this.#launcher = validateFile(options.launcherPath, 'LAUNCHER');
    this.#connectionTimeoutMs = integer(
      options.connectionTimeoutMs ?? 30_000,
      1_000,
      300_000,
      'CONNECTION_TIMEOUT',
    );
    this.#maxHeaderBytes = integer(
      options.maxHeaderBytes ?? 16_384,
      1_024,
      65_536,
      'MAX_HEADER',
    );
  }

  async pin(command: BoundaryCommand, rawOrigins: readonly string[]): Promise<RegistryPinResult> {
    this.assertStable();
    const origins = rawOrigins.map((origin, index) =>
      normalizePublicHttpsOrigin(origin, `origins[${index}]`));
    if (origins.length === 0 || new Set(origins).size !== origins.length) {
      throw new Error('HARNESS_NATIVE_ORIGIN_SET_INVALID');
    }
    const allowedHosts = new Map(await Promise.all(origins.map(async (origin) => {
      const hostname = normalizedHostname(new URL(origin).hostname);
      return [hostname, await resolvePublicDnsHost(hostname)] as const;
    })));
    const sessionId = randomBytes(8).toString('hex');
    const directory = join(this.#brokerRoot, sessionId);
    mkdirSync(directory, { mode: 0o700 });
    const socketPath = join(directory, 'p.sock');
    if (Buffer.byteLength(socketPath) > NATIVE_EGRESS_SOCKET_PATH_LIMIT) {
      rmSync(directory, { recursive: true, force: true });
      throw new Error('HARNESS_NATIVE_EGRESS_SOCKET_PATH_TOO_LONG');
    }
    const sockets = new Set<Socket>();
    const session: Session = {
      directory,
      sockets,
      requested: [],
      allowed: 0,
      denied: 0,
      server: createServer((socket) => {
        sockets.add(socket);
        socket.once('close', () => sockets.delete(socket));
        this.#handleClient(session, socket, allowedHosts);
      }),
    };
    session.server.on('error', () => {
      session.denied += 1;
    });
    await listenUnix(session.server, socketPath);
    chmodSync(socketPath, 0o600);
    const bounded = deepFreeze({
      ...command,
      executable: this.#node.path,
      args: [
        this.#launcher.path,
        '--broker-socket', socketPath,
        '--', command.executable, ...command.args,
      ],
    });
    const key = commandKey(bounded);
    if (this.#sessions.has(key)) throw new Error('HARNESS_NATIVE_ORIGIN_SESSION_COLLISION');
    this.#sessions.set(key, session);
    return deepFreeze({
      enforcement: 'origin-pinned-process-boundary',
      mechanism: 'unix-connect-broker',
      pinnedOrigins: [...origins],
      command: bounded,
    });
  }

  async complete(command: BoundaryCommand): Promise<OriginBoundaryCompletion> {
    const key = commandKey(command);
    const session = this.#sessions.get(key);
    if (session === undefined) throw new Error('HARNESS_NATIVE_ORIGIN_SESSION_UNKNOWN');
    this.#sessions.delete(key);
    await closeSession(session);
    this.assertStable();
    return deepFreeze({
      allowedConnections: session.allowed,
      deniedConnections: session.denied,
      connectDigest: digest([...session.requested].sort()),
    });
  }

  assertStable(): void {
    assertSameFile(this.#node, true, 'HARNESS_NATIVE_EGRESS_NODE_CHANGED');
    assertSameFile(this.#launcher, false, 'HARNESS_NATIVE_EGRESS_LAUNCHER_CHANGED');
    if (validatePrivateDirectory(this.#brokerRoot) !== this.#brokerRoot) {
      throw new Error('HARNESS_NATIVE_EGRESS_BROKER_ROOT_CHANGED');
    }
  }

  #handleClient(
    session: Session,
    client: Socket,
    allowedHosts: ReadonlyMap<string, DnsAddress>,
  ): void {
    client.setTimeout(this.#connectionTimeoutMs, () => client.destroy());
    let buffered = Buffer.alloc(0);
    const onData = (chunk: Buffer) => {
      buffered = Buffer.concat([buffered, chunk]);
      if (buffered.length > this.#maxHeaderBytes) {
        session.denied += 1;
        client.destroy();
        return;
      }
      const end = buffered.indexOf('\r\n\r\n');
      if (end < 0) return;
      client.off('data', onData);
      const header = buffered.subarray(0, end + 4).toString('ascii');
      const tail = buffered.subarray(end + 4);
      const target = parseConnectTarget(header);
      const pinned = target === null ? undefined : allowedHosts.get(target.host);
      session.requested.push(target === null
        ? 'invalid'
        : pinned === undefined ? target.host : `${target.host}|${pinned.family}|${pinned.address}`);
      if (target === null || pinned === undefined) {
        session.denied += 1;
        client.end('HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n');
        return;
      }
      // The broker opens a numeric TCP target, so DNS cannot be rebound after
      // admission. TLS remains end-to-end: the client still sends the original
      // hostname in its CONNECT request and subsequent SNI-bearing handshake.
      const remote = createConnection(pinnedTcpConnectionOptions(pinned));
      session.sockets.add(remote);
      remote.setTimeout(this.#connectionTimeoutMs, () => remote.destroy());
      remote.once('close', () => session.sockets.delete(remote));
      remote.once('error', () => {
        session.denied += 1;
        client.destroy();
      });
      remote.once('connect', () => {
        session.allowed += 1;
        client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
        if (tail.length > 0) remote.write(tail);
        client.pipe(remote);
        remote.pipe(client);
      });
    };
    client.on('data', onData);
  }
}

export async function resolvePublicDnsHost(
  rawHostname: string,
  resolver: DnsResolver = systemDnsResolver,
): Promise<DnsAddress> {
  const hostname = normalizedHostname(rawHostname);
  let answers: readonly DnsAddress[];
  try {
    answers = await resolver(hostname);
  } catch (cause) {
    throw new Error(`HARNESS_NATIVE_EGRESS_DNS_FAILED:${hostname}`, { cause });
  }
  return selectPublicDnsAddress(hostname, answers);
}

export function selectPublicDnsAddress(
  hostname: string,
  answers: readonly DnsAddress[],
): DnsAddress {
  if (!Array.isArray(answers) || answers.length === 0) {
    throw new Error(`HARNESS_NATIVE_EGRESS_DNS_EMPTY:${hostname}`);
  }
  const unique = new Map<string, DnsAddress>();
  for (const answer of answers) {
    const detectedFamily = isIP(answer.address);
    if ((answer.family !== 4 && answer.family !== 6)
      || detectedFamily !== answer.family || answer.address.includes('%')) {
      throw new Error(`HARNESS_NATIVE_EGRESS_DNS_ADDRESS_INVALID:${hostname}`);
    }
    if (!isPublicAddress(answer.address, answer.family)) {
      throw new Error(`HARNESS_NATIVE_EGRESS_DNS_NON_PUBLIC:${hostname}`);
    }
    unique.set(`${answer.family}:${answer.address}`, Object.freeze({ ...answer }));
  }
  return [...unique.values()].sort((left, right) =>
    left.family - right.family || left.address.localeCompare(right.address))[0];
}

export function pinnedTcpConnectionOptions(address: DnsAddress): Readonly<{
  host: string;
  port: 443;
  family: 4 | 6;
}> {
  if (!isPublicAddress(address.address, address.family)) {
    throw new Error('HARNESS_NATIVE_EGRESS_PINNED_ADDRESS_INVALID');
  }
  return Object.freeze({ host: address.address, port: 443, family: address.family });
}

function normalizedHostname(value: string): string {
  const hostname = value.toLowerCase().replace(/^\[|\]$/g, '');
  if (isIP(hostname) !== 0) throw new Error('HARNESS_NATIVE_EGRESS_LITERAL_ORIGIN_PROHIBITED');
  const labels = hostname.split('.');
  if (hostname.length > 253 || labels.some((label) =>
    !/^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label))) {
    throw new Error('HARNESS_NATIVE_EGRESS_HOSTNAME_INVALID');
  }
  return hostname;
}

async function systemDnsResolver(hostname: string): Promise<readonly DnsAddress[]> {
  const answers = await dnsLookup(hostname, { all: true, verbatim: true });
  return answers.map(({ address, family }) => {
    if (family !== 4 && family !== 6) {
      throw new Error(`HARNESS_NATIVE_EGRESS_DNS_ADDRESS_INVALID:${hostname}`);
    }
    return { address, family };
  });
}

function isPublicAddress(address: string, family: 4 | 6): boolean {
  const list = family === 4 ? IPV4_NON_PUBLIC : IPV6_NON_PUBLIC;
  return !list.check(address, family === 4 ? 'ipv4' : 'ipv6');
}

function addressBlockList(
  ranges: ReadonlyArray<readonly [address: string, prefix: number]>,
  family: 'ipv4' | 'ipv6',
): BlockList {
  const list = new BlockList();
  for (const [address, prefix] of ranges) list.addSubnet(address, prefix, family);
  return list;
}

function parseConnectTarget(header: string): { host: string } | null {
  if (header.includes('\0') || header.includes('\r\n ') || header.includes('\r\n\t')) return null;
  const lines = header.slice(0, -4).split('\r\n');
  const match = /^CONNECT ([A-Za-z0-9.-]+):443 HTTP\/1\.[01]$/.exec(lines[0] ?? '');
  if (match === null || !lines.slice(1).every((line) => /^[!-~]+(?: [\x20-\x7e]*)?$/.test(line))) {
    return null;
  }
  const host = match[1].toLowerCase();
  if (host.startsWith('.') || host.endsWith('.') || host.includes('..')) return null;
  return { host };
}

async function listenUnix(server: Server, socketPath: string): Promise<void> {
  await new Promise<void>((resolveReady, reject) => {
    const error = (cause: Error) => reject(cause);
    server.once('error', error);
    server.listen(socketPath, () => {
      server.off('error', error);
      resolveReady();
    });
  });
}

async function closeSession(session: Session): Promise<void> {
  for (const socket of session.sockets) socket.destroy();
  await new Promise<void>((resolveClosed) => session.server.close(() => resolveClosed()));
  rmSync(session.directory, { recursive: true, force: true });
}

interface FileIdentity { readonly path: string; readonly digest: string }

function validateExecutable(value: string, label: string): FileIdentity {
  const identity = validateFile(value, label);
  if ((lstatSync(identity.path).mode & 0o111) === 0) {
    throw new Error(`HARNESS_NATIVE_EGRESS_${label}_INVALID`);
  }
  return identity;
}

function validateFile(value: string, label: string): FileIdentity {
  try {
    if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) throw new Error();
    const stat = lstatSync(value);
    const uid = process.getuid?.() ?? stat.uid;
    if (!stat.isFile() || stat.isSymbolicLink() || realpathSync(value) !== value
      || stat.nlink !== 1 || (stat.mode & 0o022) !== 0
      || (stat.uid !== 0 && stat.uid !== uid)) throw new Error();
    return Object.freeze({ path: value, digest: digest(readFileSync(value)) });
  } catch {
    throw new Error(`HARNESS_NATIVE_EGRESS_${label}_INVALID`);
  }
}

function validatePrivateDirectory(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error('HARNESS_NATIVE_EGRESS_BROKER_ROOT_INVALID');
  }
  const stat = lstatSync(value);
  const uid = process.getuid?.() ?? stat.uid;
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value
    || stat.uid !== uid || (stat.mode & 0o077) !== 0) {
    throw new Error('HARNESS_NATIVE_EGRESS_BROKER_ROOT_INVALID');
  }
  return value;
}

function assertSameFile(identity: FileIdentity, executable: boolean, error: string): void {
  const current = executable
    ? validateExecutable(identity.path, 'STABILITY')
    : validateFile(identity.path, 'STABILITY');
  if (current.digest !== identity.digest) throw new Error(error);
}

function commandKey(command: BoundaryCommand): string {
  return digest(command);
}

function digest(value: unknown): string {
  const bytes = Buffer.isBuffer(value) ? value : Buffer.from(JSON.stringify(value));
  return createHash('sha256').update(bytes).digest('hex');
}

function integer(value: number, minimum: number, maximum: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new Error(`HARNESS_NATIVE_EGRESS_${label}_INVALID`);
  }
  return value;
}

export function assertBrokerRootContains(root: string, path: string): void {
  const delta = relative(root, path);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error('HARNESS_NATIVE_EGRESS_SOCKET_OUTSIDE_ROOT');
  }
}

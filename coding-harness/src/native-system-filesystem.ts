// SPDX-License-Identifier: MIT
import { createHash } from 'node:crypto';
import {
  existsSync,
  lstatSync,
  readFileSync,
  realpathSync,
  statSync,
} from 'node:fs';
import { basename, dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { deepFreeze } from './contracts.js';
import type {
  NativeFilesystemIsolationResult,
  NativeFilesystemPolicy,
  NativeModelFilesystemBoundary,
} from './native-filesystem.js';
import type { NativeHost } from './models/types.js';
import type { BoundaryCommand } from './network.js';

export interface NativeRuntimeMount {
  readonly source: string;
  readonly destination: string;
}
export interface NativeHostFilesystemProfile {
  readonly authenticationMounts: readonly NativeRuntimeMount[];
  readonly runtimeMounts: readonly NativeRuntimeMount[];
  readonly privateEnvironment: Readonly<Record<string, string>>;
}

export interface SystemNativeFilesystemBoundaryOptions {
  readonly bwrapExecutable: string;
  readonly brokerRoot: string;
  readonly allowedRuntimeFiles: readonly string[];
  readonly authenticationSourceRoot: string;
  readonly forbiddenMountRoots: readonly string[];
  readonly hosts: Readonly<Record<NativeHost, NativeHostFilesystemProfile>>;
}

interface StableMount extends NativeRuntimeMount {
  readonly identity: string;
  readonly mutable: boolean;
}
export class SystemNativeFilesystemBoundary implements NativeModelFilesystemBoundary {
  readonly #bwrap: FileIdentity;
  readonly #brokerRoot: string;
  readonly #authenticationSourceRoot: string;
  readonly #forbiddenMountRoots: readonly string[];
  readonly #common: readonly StableMount[];
  readonly #hosts: Readonly<Record<NativeHost, {
    readonly authentication: readonly StableMount[];
    readonly runtime: readonly StableMount[];
    readonly environment: Readonly<Record<string, string>>;
  }>>;

  constructor(options: SystemNativeFilesystemBoundaryOptions) {
    if (process.platform !== 'linux') throw new Error('HARNESS_NATIVE_FILESYSTEM_OS_UNAVAILABLE');
    this.#bwrap = validateExecutable(options.bwrapExecutable, 'BWRAP');
    this.#brokerRoot = privateDirectory(options.brokerRoot, 'BROKER_ROOT');
    this.#authenticationSourceRoot = privateDirectory(
      options.authenticationSourceRoot,
      'AUTHENTICATION_SOURCE_ROOT',
    );
    this.#forbiddenMountRoots = validateForbiddenRoots(options.forbiddenMountRoots);
    const allowedRuntimeFiles = validateRuntimeFiles(options.allowedRuntimeFiles);
    this.#common = validateMounts(systemNativeRuntimeLibraryMounts(), false, 'COMMON', {
      allowedRuntimeFiles,
      authenticationSourceRoot: this.#authenticationSourceRoot,
      forbiddenMountRoots: this.#forbiddenMountRoots,
      kind: 'common',
    });
    this.#hosts = Object.freeze({
      codex: profile(options.hosts.codex, 'codex', {
        allowedRuntimeFiles,
        authenticationSourceRoot: this.#authenticationSourceRoot,
        forbiddenMountRoots: this.#forbiddenMountRoots,
      }),
      'claude-code': profile(options.hosts['claude-code'], 'claude-code', {
        allowedRuntimeFiles,
        authenticationSourceRoot: this.#authenticationSourceRoot,
        forbiddenMountRoots: this.#forbiddenMountRoots,
      }),
    });
    assertMountTopology([
      ...this.#common,
      ...Object.values(this.#hosts).flatMap(({ runtime, authentication }) => [
        ...runtime,
        ...authentication,
      ]),
    ]);
  }

  isolate(
    command: BoundaryCommand,
    policy: NativeFilesystemPolicy,
  ): NativeFilesystemIsolationResult {
    this.assertStable();
    const selected = this.#hosts[policy.host];
    const workspace = canonicalDirectory(policy.workspaceRoot, 'WORKSPACE');
    if (workspace === '/') throw new Error('HARNESS_NATIVE_WORKSPACE_ROOT_BROAD');
    const brokerSession = validateBrokerSession(command, this.#brokerRoot);
    const requestedReadOnly = policy.readOnlyRoots.map((path) => existingPath(path, 'READ_ONLY'));
    if (requestedReadOnly.includes('/')) throw new Error('HARNESS_NATIVE_READ_ONLY_ROOT_BROAD');
    const writable = policy.writablePaths.map((path) => writableFile(path));
    const masked = policy.maskedPaths.map((path) => mask(path, workspace));
    const runtime = [...this.#common, ...selected.runtime, ...selected.authentication];
    const visible = dedupeMounts([
      ...runtime,
      { source: brokerSession, destination: brokerSession },
      { source: workspace, destination: workspace },
      ...requestedReadOnly.map((path) => ({ source: path, destination: path })),
    ]);
    assertVisible(command.executable, visible, 'HARNESS_NATIVE_COMMAND_NOT_MOUNTED');
    const directories = parentDirectories([
      '/dev', '/proc', '/tmp', '/run', '/home/harness',
      ...visible.filter(({ source }) => statSync(source).isDirectory())
        .map(({ destination }) => destination),
      ...masked.filter(({ kind }) => kind === 'directory').map(({ path }) => path),
    ], [
      ...visible.filter(({ source }) => statSync(source).isFile())
        .map(({ destination }) => destination),
      ...writable,
      ...masked.filter(({ kind }) => kind === 'file').map(({ path }) => path),
    ]);
    const mountManifest = {
      runtime: runtime.map(({ source, destination, identity }) => ({ source, destination, identity })),
      brokerSession,
      workspace,
      requestedReadOnly,
      writable,
      masked: masked.map(({ path, kind }) => ({ path, kind })),
    };
    const prefix = [
      '--die-with-parent', '--new-session', '--unshare-all', '--clearenv', '--tmpfs', '/',
      '--hostname', 'semantic-fabric-harness', '--cap-drop', 'ALL',
      ...directories.flatMap((path) => ['--dir', path]),
      '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/run',
      ...visible.flatMap(({ source, destination }) => ['--ro-bind', source, destination]),
      ...writable.flatMap((path) => ['--bind', path, path]),
      ...masked.flatMap(({ path, kind }) => kind === 'directory'
        ? ['--tmpfs', path]
        : ['--ro-bind', '/dev/null', path]),
      ...Object.entries({ ...command.env, ...selected.environment })
        .flatMap(([name, value]) => ['--setenv', name, value]),
      '--chdir', workspace, '--',
    ];
    return deepFreeze({
      enforcement: 'os-filesystem-namespace',
      mechanism: 'bwrap-private-root-unshared-net',
      mountManifestDigest: digest(mountManifest),
      configurationMaskDigest: digest(mountManifest.masked),
      host: policy.host,
      workspaceRoot: policy.workspaceRoot,
      readOnlyRoots: policy.readOnlyRoots,
      writablePaths: policy.writablePaths,
      maskedPaths: policy.maskedPaths,
      hostFileConfidentiality: true,
      emptyPrivateHome: true,
      privateEphemeralHome: true,
      hostRootMounted: false,
      hostCredentialPathMounted: false,
      command: {
        ...command,
        executable: this.#bwrap.path,
        args: [...prefix, command.executable, ...command.args],
      },
    });
  }

  assertStable(): void {
    if (validateExecutable(this.#bwrap.path, 'BWRAP').digest !== this.#bwrap.digest) {
      throw new Error('HARNESS_NATIVE_BWRAP_CHANGED');
    }
    privateDirectory(this.#brokerRoot, 'BROKER_ROOT');
    for (const mount of [
      ...this.#common,
      ...Object.values(this.#hosts).flatMap(({ runtime, authentication }) => [
        ...runtime,
        ...authentication,
      ]),
    ]) {
      if (mountIdentity(mount.source, mount.mutable) !== mount.identity) {
        throw new Error('HARNESS_NATIVE_RUNTIME_MOUNT_CHANGED');
      }
    }
  }
}
interface MountValidationContext {
  readonly allowedRuntimeFiles: ReadonlySet<string>;
  readonly authenticationSourceRoot: string;
  readonly forbiddenMountRoots: readonly string[];
}
function profile(
  value: NativeHostFilesystemProfile,
  host: NativeHost,
  context: MountValidationContext,
) {
  if (value === undefined) throw new Error(`HARNESS_NATIVE_FILESYSTEM_PROFILE_REQUIRED:${host}`);
  const authentication = validateMounts(value.authenticationMounts, true, `${host}:AUTH`, {
    ...context,
    kind: 'authentication',
  });
  const expectedCredential = host === 'codex'
    ? '/home/harness/.codex/auth.json'
    : '/home/harness/.claude/.credentials.json';
  if (authentication.length !== 1 || authentication[0]?.destination !== expectedCredential) {
    throw new Error(`HARNESS_NATIVE_AUTH_MOUNT_INVALID:${host}`);
  }
  const runtime = validateMounts(value.runtimeMounts, false, `${host}:RUNTIME`, {
    ...context,
    kind: 'runtime-file',
  });
  const environment = Object.freeze({ ...value.privateEnvironment });
  const expectedConfig = host === 'codex' ? 'CODEX_HOME' : 'CLAUDE_CONFIG_DIR';
  if (environment.HOME !== '/home/harness'
    || environment[expectedConfig] !== `/home/harness/${host === 'codex' ? '.codex' : '.claude'}`
    || Object.values(environment).some((entry) => entry.includes('\0'))) {
    throw new Error(`HARNESS_NATIVE_PRIVATE_ENVIRONMENT_INVALID:${host}`);
  }
  return Object.freeze({ authentication, runtime, environment });
}
function validateBrokerSession(command: BoundaryCommand, brokerRoot: string): string {
  if (command.args[1] !== '--broker-socket' || command.args[3] !== '--') {
    throw new Error('HARNESS_NATIVE_BROKER_COMMAND_INVALID');
  }
  const socketPath = command.args[2];
  if (socketPath === undefined || basename(socketPath) !== 'p.sock') {
    throw new Error('HARNESS_NATIVE_BROKER_COMMAND_INVALID');
  }
  const session = privateDirectory(dirname(socketPath), 'BROKER_SESSION');
  if (!contains(brokerRoot, session) || session === brokerRoot) {
    throw new Error('HARNESS_NATIVE_BROKER_SESSION_OUTSIDE_ROOT');
  }
  const socket = lstatSync(socketPath);
  if (!socket.isSocket() || socket.isSymbolicLink() || realpathSync(socketPath) !== socketPath) {
    throw new Error('HARNESS_NATIVE_BROKER_SOCKET_INVALID');
  }
  return session;
}

function validateMounts(
  values: readonly NativeRuntimeMount[],
  mutable: boolean,
  label: string,
  context: MountValidationContext & Readonly<{
    kind: 'common' | 'runtime-file' | 'authentication';
  }>,
): StableMount[] {
  const destinations = new Set<string>();
  return values.map((entry, index) => {
    if (entry === null || typeof entry !== 'object') throw new Error(`HARNESS_NATIVE_${label}_MOUNT_INVALID`);
    const source = existingPath(entry.source, `${label}[${index}].source`);
    const destination = absolute(entry.destination, `${label}[${index}].destination`);
    if (destination === '/' || destinations.has(destination)) {
      throw new Error(`HARNESS_NATIVE_${label}_MOUNT_INVALID`);
    }
    assertMountScope(source, destination, label, context);
    destinations.add(destination);
    return Object.freeze({
      source,
      destination,
      mutable,
      identity: mountIdentity(source, mutable),
    });
  });
}

const SYSTEM_LIBRARY_MOUNTS = Object.freeze([
  Object.freeze({ source: '/usr/lib', destination: '/usr/lib' }),
  Object.freeze({ source: '/usr/lib', destination: '/lib' }),
  Object.freeze({ source: '/usr/lib64', destination: '/usr/lib64' }),
  Object.freeze({ source: '/usr/lib64', destination: '/lib64' }),
] as const);

export function systemNativeRuntimeLibraryMounts(): readonly NativeRuntimeMount[] {
  return Object.freeze(SYSTEM_LIBRARY_MOUNTS
    .filter(({ source }) => existsSync(source))
    .map(({ source, destination }) => Object.freeze({ source, destination })));
}

function assertMountScope(
  source: string,
  destination: string,
  label: string,
  context: MountValidationContext & Readonly<{
    kind: 'common' | 'runtime-file' | 'authentication';
  }>,
): void {
  const stat = lstatSync(source);
  if (context.kind === 'common') {
    const allowed = SYSTEM_LIBRARY_MOUNTS.some((entry) =>
      entry.source === source && entry.destination === destination);
    if (!allowed || !stat.isDirectory()
      || overlapsAny(source, context.forbiddenMountRoots)
      || overlapsAny(destination, context.forbiddenMountRoots)) {
      throw new Error(`HARNESS_NATIVE_${label}_MOUNT_OUTSIDE_ALLOWLIST`);
    }
    return;
  }
  if (context.kind === 'runtime-file') {
    if (!stat.isFile() || source !== destination || !context.allowedRuntimeFiles.has(source)) {
      throw new Error(`HARNESS_NATIVE_${label}_MOUNT_OUTSIDE_ALLOWLIST`);
    }
    return;
  }
  if (!stat.isFile() || source === context.authenticationSourceRoot
    || !contains(context.authenticationSourceRoot, source)) {
    throw new Error(`HARNESS_NATIVE_${label}_MOUNT_OUTSIDE_ALLOWLIST`);
  }
}

function validateRuntimeFiles(values: readonly string[]): ReadonlySet<string> {
  if (values.length === 0) throw new Error('HARNESS_NATIVE_RUNTIME_FILES_REQUIRED');
  const output = new Set(values.map((path, index) => {
    const value = existingPath(path, `RUNTIME_FILES[${index}]`);
    if (!lstatSync(value).isFile()) throw new Error('HARNESS_NATIVE_RUNTIME_FILE_INVALID');
    mountIdentity(value, false);
    return value;
  }));
  if (output.size !== values.length) throw new Error('HARNESS_NATIVE_RUNTIME_FILE_DUPLICATE');
  return output;
}

function validateForbiddenRoots(values: readonly string[]): readonly string[] {
  if (values.length === 0) throw new Error('HARNESS_NATIVE_FORBIDDEN_MOUNT_ROOTS_REQUIRED');
  const roots = values.map((path, index) => {
    const value = existingPath(path, `FORBIDDEN_MOUNT_ROOTS[${index}]`);
    if (!lstatSync(value).isDirectory() || value === '/') {
      throw new Error('HARNESS_NATIVE_FORBIDDEN_MOUNT_ROOT_INVALID');
    }
    return value;
  });
  return Object.freeze([...new Set(roots)]);
}

function overlapsAny(path: string, roots: readonly string[]): boolean {
  return roots.some((root) => contains(root, path) || contains(path, root));
}

function assertMountTopology(mounts: readonly StableMount[]): void {
  const unique = new Map<string, StableMount>();
  for (const mount of mounts) {
    const previous = unique.get(mount.destination);
    if (previous !== undefined && previous.source !== mount.source) {
      throw new Error('HARNESS_NATIVE_MOUNT_DESTINATION_COLLISION');
    }
    unique.set(mount.destination, mount);
  }
  const entries = [...unique.values()];
  for (let left = 0; left < entries.length; left += 1) {
    for (let right = left + 1; right < entries.length; right += 1) {
      const first = entries[left] as StableMount;
      const second = entries[right] as StableMount;
      if (contains(first.destination, second.destination)
        || contains(second.destination, first.destination)) {
        throw new Error('HARNESS_NATIVE_MOUNT_DESTINATION_OVERLAP');
      }
    }
  }
}

function mountIdentity(path: string, mutable: boolean): string {
  const stat = lstatSync(path);
  const uid = process.getuid?.() ?? stat.uid;
  if (stat.isSymbolicLink() || realpathSync(path) !== path
    || (!stat.isFile() && !stat.isDirectory())
    || (stat.mode & 0o022) !== 0
    || (!mutable && stat.uid !== 0 && stat.uid !== uid)
    || (mutable && (stat.uid !== uid || (stat.mode & 0o077) !== 0))) {
    throw new Error('HARNESS_NATIVE_RUNTIME_MOUNT_UNTRUSTED');
  }
  const content = stat.isFile() ? readFileSync(path) : Buffer.alloc(0);
  return digest({
    path,
    kind: stat.isFile() ? 'file' : 'directory',
    device: stat.dev,
    inode: stat.ino,
    mode: stat.mode,
    uid: stat.uid,
    size: stat.size,
    modifiedMs: stat.mtimeMs,
    content: createHash('sha256').update(content).digest('hex'),
  });
}

interface Mask { readonly path: string; readonly kind: 'file' | 'directory' }

function mask(path: string, workspace: string): Mask {
  absolute(path, 'MASK');
  if (!contains(workspace, path) || path === workspace || !existsSync(path)) {
    throw new Error('HARNESS_NATIVE_MASK_PATH_INVALID');
  }
  const stat = lstatSync(path);
  if (stat.isSymbolicLink() || realpathSync(path) !== path) {
    throw new Error('HARNESS_NATIVE_MASK_PATH_UNTRUSTED');
  }
  if (stat.isFile()) return { path, kind: 'file' };
  if (stat.isDirectory()) return { path, kind: 'directory' };
  throw new Error('HARNESS_NATIVE_MASK_PATH_INVALID');
}

function writableFile(path: string): string {
  const value = existingPath(path, 'WRITABLE');
  const stat = lstatSync(value);
  if (!stat.isFile() || stat.nlink !== 1) throw new Error('HARNESS_NATIVE_WRITABLE_FILE_INVALID');
  return value;
}

function assertVisible(path: string, mounts: readonly NativeRuntimeMount[], error: string): void {
  if (!mounts.some(({ destination }) => contains(destination, path))) throw new Error(error);
}

function dedupeMounts(mounts: readonly NativeRuntimeMount[]): NativeRuntimeMount[] {
  const output = new Map<string, NativeRuntimeMount>();
  for (const mount of mounts) {
    const previous = output.get(mount.destination);
    if (previous !== undefined && previous.source !== mount.source) {
      throw new Error('HARNESS_NATIVE_MOUNT_DESTINATION_COLLISION');
    }
    output.set(mount.destination, mount);
  }
  return [...output.values()];
}

function parentDirectories(
  directoryPaths: readonly string[],
  filePaths: readonly string[],
): string[] {
  const output = new Set<string>();
  for (const path of directoryPaths) {
    let current = path;
    while (current !== '/') {
      output.add(current);
      current = dirname(current);
    }
  }
  for (const path of filePaths) {
    let current = dirname(path);
    while (current !== '/') {
      output.add(current);
      current = dirname(current);
    }
  }
  return [...output].sort((left, right) => left.split(sep).length - right.split(sep).length);
}

interface FileIdentity { readonly path: string; readonly digest: string }

function validateExecutable(path: string, label: string): FileIdentity {
  const value = existingPath(path, label);
  const stat = lstatSync(value);
  const uid = process.getuid?.() ?? stat.uid;
  if (!stat.isFile() || stat.nlink !== 1 || (stat.mode & 0o111) === 0
    || (stat.mode & 0o022) !== 0 || (stat.uid !== 0 && stat.uid !== uid)) {
    throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  }
  return Object.freeze({ path: value, digest: digest(readFileSync(value)) });
}

function privateDirectory(path: string, label: string): string {
  const value = canonicalDirectory(path, label);
  const stat = lstatSync(value);
  const uid = process.getuid?.() ?? stat.uid;
  if (stat.uid !== uid || (stat.mode & 0o077) !== 0) {
    throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  }
  return value;
}

function canonicalDirectory(path: string, label: string): string {
  const value = existingPath(path, label);
  if (!statSync(value).isDirectory()) throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  return value;
}

function existingPath(path: string, label: string): string {
  const value = absolute(path, label);
  const stat = lstatSync(value);
  if (stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  }
  return value;
}

function absolute(path: string, label: string): string {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  }
  return path;
}

function contains(root: string, path: string): boolean {
  const delta = relative(root, path);
  return delta === '' || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

function digest(value: unknown): string {
  const bytes = Buffer.isBuffer(value) ? value : Buffer.from(JSON.stringify(value));
  return createHash('sha256').update(bytes).digest('hex');
}

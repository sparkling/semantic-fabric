// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { lstatSync, mkdirSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { createImmutablePrivateRuntime } from './immutable-private-runtime.js';
import {
  PROGRAMME_V5_RUFLO_BWRAP_IDENTITY,
  PROGRAMME_V5_RUFLO_CLI_IDENTITY,
  PROGRAMME_V5_RUFLO_NODE_IDENTITY,
} from './programme-v5-ruflo-contract.js';
import { systemNativeRuntimeLibraryMounts } from './native-system-filesystem.js';

const PACKAGE_ROOT = dirname(dirname(PROGRAMME_V5_RUFLO_CLI_IDENTITY.entryPath));
const PACKAGE_MANIFEST = join(PACKAGE_ROOT, 'package.json');
const PACKAGE_BIN = join(PACKAGE_ROOT, 'bin');
const PACKAGE_DIST = join(PACKAGE_ROOT, 'dist');
const CLI_CORE_ROOT = join(PACKAGE_ROOT, 'node_modules', '@claude-flow', 'cli-core');
const SECURITY_ROOT = join(PACKAGE_ROOT, 'node_modules', '@claude-flow', 'security');
const BCRYPT_ROOT = join(PACKAGE_ROOT, 'node_modules', 'bcryptjs');
const ZOD_ROOT = join(PACKAGE_ROOT, 'node_modules', 'zod');
const MANIFEST_DIGEST = '633b4446e2574f0863ba53ab5918fb663d55a8d1c5195e7811cd5a10e67320b8';
const BIN_DIGEST = '17479c2c2ee3143942738bff57fbddc959ec97decf62425b0c98045004d2a771';
const DIST_DIGEST = 'a3d4b4ea863454b38b2ef34dba8243e1395654c325979ec696d636151546139e';
const CLI_CORE_DIGEST = '834adb683e8c3f7b5a305d8b9000ca39754b1d1769118c1a847345190930e72e';
const SECURITY_DIGEST = 'a7246b4cfb669c985afec7b6146821f923c76e137cabd0246a72ec3ba1a194aa';
const BCRYPT_DIGEST = '5a3298560aabac5f100308256dc74316a511e8ece4b3de039ef0f5d206fd568c';
const ZOD_DIGEST = '9605ce9ccc2d0fe5f8d87cde90fb4ce9d9c8e8c8515ce9857825a54eab568e2d';
const PASSWD_DIGEST = '1673047aafa580b5f9b444ab097e43fcca2df7418d71e579f059abd9b4c2a738';
const GROUP_DIGEST = 'ef41ce1d713e984f66052cc15402503708ecfeadd7762068ef5b90d18e7361bf';
const RUNTIME_PARENT = '/home/claude/.cache/semantic-fabric-harness';
const MAX_LEDGER_FILE_BYTES = 32 * 1024 * 1024;

export interface ProgrammeV5RufloPrivateRuntime {
  readonly executable: string;
  readonly args: readonly string[];
  readonly cwd: string;
  readonly environment: Readonly<Record<string, string>>;
  cleanup(): void;
}

export function createProgrammeV5RufloPrivateRuntime(
  repositoryRoot: string,
): ProgrammeV5RufloPrivateRuntime {
  const repository = canonicalDirectory(repositoryRoot, 'REPOSITORY');
  const taskStore = join(repository, '.claude-flow', 'tasks', 'store.json');
  const swarmState = join(repository, '.claude-flow', 'swarm', 'swarm-state.json');
  assertRootOnlyBwrapParent();
  mkdirSync(RUNTIME_PARENT, { recursive: true, mode: 0o700 });
  const runtime = createImmutablePrivateRuntime({
    parent: canonicalDirectory(RUNTIME_PARENT, 'PARENT'),
    prefix: 'ruflo-',
    files: [
      {
        key: 'node', sourcePath: PROGRAMME_V5_RUFLO_NODE_IDENTITY.path,
        relativePath: 'node', executable: true,
        expectedDigest: PROGRAMME_V5_RUFLO_NODE_IDENTITY.digest,
      },
      {
        key: 'bwrap', sourcePath: PROGRAMME_V5_RUFLO_BWRAP_IDENTITY.path,
        relativePath: 'bwrap-attestation', executable: true,
        expectedDigest: PROGRAMME_V5_RUFLO_BWRAP_IDENTITY.digest,
      },
      {
        key: 'manifest', sourcePath: PACKAGE_MANIFEST,
        relativePath: 'package/package.json', executable: false,
        expectedDigest: MANIFEST_DIGEST, maxBytes: 1_048_576,
      },
      {
        key: 'passwd', sourcePath: '/etc/passwd', relativePath: 'etc/passwd',
        executable: false, expectedDigest: PASSWD_DIGEST, maxBytes: 65_536,
      },
      {
        key: 'group', sourcePath: '/etc/group', relativePath: 'etc/group',
        executable: false, expectedDigest: GROUP_DIGEST, maxBytes: 65_536,
      },
      {
        key: 'taskStore', sourcePath: taskStore,
        relativePath: 'ledger/tasks/store.json', executable: false,
        maxBytes: MAX_LEDGER_FILE_BYTES,
        sourcePolicy: 'same-principal-cooperative-snapshot',
      },
      {
        key: 'swarmState', sourcePath: swarmState,
        relativePath: 'ledger/swarm/swarm-state.json', executable: false,
        maxBytes: MAX_LEDGER_FILE_BYTES,
        sourcePolicy: 'same-principal-cooperative-snapshot',
      },
    ],
    trees: [
      {
        key: 'bin', sourceRoot: PACKAGE_BIN, relativePath: 'package/bin',
        maxFiles: 16, maxBytes: 1_048_576,
      },
      {
        key: 'dist', sourceRoot: PACKAGE_DIST, relativePath: 'package/dist',
        maxFiles: 1_024, maxBytes: 20_000_000,
      },
      {
        key: 'cliCore', sourceRoot: CLI_CORE_ROOT,
        relativePath: 'package/node_modules/@claude-flow/cli-core',
        maxFiles: 128, maxBytes: 1_000_000,
      },
      {
        key: 'security', sourceRoot: SECURITY_ROOT,
        relativePath: 'package/node_modules/@claude-flow/security',
        maxFiles: 256, maxBytes: 2_000_000,
      },
      {
        key: 'bcrypt', sourceRoot: BCRYPT_ROOT,
        relativePath: 'package/node_modules/bcryptjs',
        maxFiles: 128, maxBytes: 1_000_000,
      },
      {
        key: 'zod', sourceRoot: ZOD_ROOT,
        relativePath: 'package/node_modules/zod',
        maxFiles: 1_024, maxBytes: 10_000_000,
      },
    ],
  });
  try {
    assertPackageSource(runtime);
    const libraries = systemNativeRuntimeLibraryMounts();
    const directories = [
      '/runtime', '/runtime/package', '/workspace', '/workspace/.claude-flow',
      '/workspace/.claude-flow/tasks', '/workspace/.claude-flow/swarm',
      '/usr', '/usr/lib', '/usr/lib64', '/lib', '/lib64', '/etc', '/etc/ssl',
      '/etc/ssl/certs', '/home', '/home/harness',
    ];
    const args = [
      '--die-with-parent', '--new-session', '--unshare-all', '--unshare-net',
      '--clearenv', '--tmpfs', '/',
      '--hostname', 'semantic-fabric-ruflo', '--cap-drop', 'ALL',
      ...directories.flatMap((path) => ['--dir', path]),
      '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/run',
      ...libraries.flatMap(({ source, destination }) => ['--ro-bind', source, destination]),
      '--ro-bind', runtime.files.node.path, '/runtime/node',
      '--ro-bind', join(runtime.root, 'package'), '/runtime/package',
      '--ro-bind', runtime.files.passwd.path, '/etc/passwd',
      '--ro-bind', runtime.files.group.path, '/etc/group',
      '--ro-bind', join(runtime.root, 'ledger/tasks'), '/workspace/.claude-flow/tasks',
      '--ro-bind', join(runtime.root, 'ledger/swarm'), '/workspace/.claude-flow/swarm',
      '--tmpfs', '/workspace/.claude-flow/policy',
      '--setenv', 'CLAUDE_FLOW_CWD', '/workspace',
      '--setenv', 'HOME', '/home/harness',
      '--setenv', 'LANG', 'C.UTF-8', '--setenv', 'LC_ALL', 'C.UTF-8',
      '--setenv', 'TZ', 'UTC', '--chdir', '/workspace', '--',
      '/runtime/node', '/runtime/package/bin/mcp-server.js',
    ];
    return Object.freeze({
      executable: PROGRAMME_V5_RUFLO_BWRAP_IDENTITY.path,
      args: Object.freeze(args),
      cwd: repository,
      environment: Object.freeze({ LANG: 'C.UTF-8', LC_ALL: 'C.UTF-8', TZ: 'UTC' }),
      cleanup: runtime.cleanup,
    });
  } catch (error) {
    runtime.cleanup();
    throw error;
  }
}

function assertPackageSource(runtime: ReturnType<typeof createImmutablePrivateRuntime>): void {
  const bin = runtime.trees.bin;
  const dist = runtime.trees.dist;
  const cliCore = runtime.trees.cliCore;
  const security = runtime.trees.security;
  const bcrypt = runtime.trees.bcrypt;
  const zod = runtime.trees.zod;
  const manifest = runtime.files.manifest;
  if (bin?.digest !== BIN_DIGEST || bin.fileCount !== 3 || bin.totalBytes !== 18_734
    || dist?.digest !== DIST_DIGEST || dist.fileCount !== 773 || dist.totalBytes !== 9_565_663
    || cliCore?.digest !== CLI_CORE_DIGEST || cliCore.fileCount !== 46
    || cliCore.totalBytes !== 156_182
    || security?.digest !== SECURITY_DIGEST || security.fileCount !== 122
    || security.totalBytes !== 584_774
    || bcrypt?.digest !== BCRYPT_DIGEST || bcrypt.fileCount !== 11
    || bcrypt.totalBytes !== 112_292
    || zod?.digest !== ZOD_DIGEST || zod.fileCount !== 596
    || zod.totalBytes !== 3_594_196
    || manifest?.digest !== MANIFEST_DIGEST) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_PACKAGE_SOURCE_MISMATCH');
  }
  const body = {
    manifestDigest: manifest.digest,
    binDigest: bin.digest,
    distDigest: dist.digest,
    cliCoreDigest: cliCore.digest,
    securityDigest: security.digest,
    bcryptDigest: bcrypt.digest,
    zodDigest: zod.digest,
    fileCount: 1 + bin.fileCount + dist.fileCount + cliCore.fileCount
      + security.fileCount + bcrypt.fileCount + zod.fileCount,
    totalBytes: 5_063 + bin.totalBytes + dist.totalBytes + cliCore.totalBytes
      + security.totalBytes + bcrypt.totalBytes + zod.totalBytes,
  };
  const digest = createHash('sha256').update(JSON.stringify(body)).digest('hex');
  if (digest !== PROGRAMME_V5_RUFLO_CLI_IDENTITY.packageSourceDigest
    || body.fileCount !== PROGRAMME_V5_RUFLO_CLI_IDENTITY.packageSourceFileCount
    || body.totalBytes !== PROGRAMME_V5_RUFLO_CLI_IDENTITY.packageSourceBytes) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_PACKAGE_SOURCE_IDENTITY_MISMATCH');
  }
}

function canonicalDirectory(value: string, label: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error(`HARNESS_PROGRAMME_V5_RUFLO_RUNTIME_${label}_INVALID`);
  }
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error(`HARNESS_PROGRAMME_V5_RUFLO_RUNTIME_${label}_INVALID`);
  }
  return value;
}

function assertRootOnlyBwrapParent(): void {
  const parent = canonicalDirectory(dirname(PROGRAMME_V5_RUFLO_BWRAP_IDENTITY.path), 'BWRAP_PARENT');
  const stat = lstatSync(parent);
  if (stat.uid !== 0 || (stat.mode & 0o022) !== 0) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_RUNTIME_BWRAP_PARENT_INVALID');
  }
}

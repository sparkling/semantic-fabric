// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync,
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  mkdirSync,
  openSync,
  readSync,
  readdirSync,
  realpathSync,
  writeSync,
  type BigIntStats,
} from 'node:fs';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';

const TOOLCHAIN_CONTENT_DIGEST =
  '81cc515ef94bae07d2451ff3701ce6e6eee7878327dc8088ebac773f1570f7c4';
const REGISTRY_CONTENT_DIGEST =
  '13c044505bc77ccb98ad61887bf118d850df7484faa2901f7288698399098ff0';
const REGISTRY_KEY = 'index.crates.io-1949cf8c6b5b557f';
const MAX_FILE_BYTES = 1_000_000_000;
const MAX_TOOLCHAIN_ENTRIES = 70_000;
const MAX_REGISTRY_ENTRIES = 100_000;
const MAX_TOOLCHAIN_BYTES = 2_000_000_000;
const MAX_REGISTRY_BYTES = 3_000_000_000;

export interface Issue8RustClosure {
  readonly toolchainRoot: string;
  readonly registryRoot: string;
  readonly cargoExecutable: string;
  readonly evidence: Readonly<Record<string, string>>;
  assertStable(): void;
}

interface TreeSource {
  readonly source: string;
  readonly prefix: string;
}

interface CopiedTree {
  readonly contentDigest: string;
  readonly entries: number;
  readonly bytes: number;
}

export function prepareIssue8RustClosure(input: Readonly<{
  scratchRoot: string;
  toolchainSource: string;
  registrySource: string;
}>): Issue8RustClosure {
  const scratchRoot = canonicalDirectory(input.scratchRoot, 'HARNESS_RUST_CLOSURE_SCRATCH_INVALID');
  const sourceToolchain = canonicalDirectory(
    input.toolchainSource,
    'HARNESS_RUST_CLOSURE_TOOLCHAIN_SOURCE_INVALID',
  );
  const sourceRegistry = canonicalDirectory(
    input.registrySource,
    'HARNESS_RUST_CLOSURE_REGISTRY_SOURCE_INVALID',
  );
  const closureRoot = join(scratchRoot, 'rust-closure');
  const toolchainRoot = join(closureRoot, 'toolchain');
  const registryRoot = join(closureRoot, 'registry');
  mkdirSync(toolchainRoot, { recursive: true, mode: 0o700 });
  mkdirSync(registryRoot, { recursive: true, mode: 0o700 });

  const toolchain = copyTrees(
    [{ source: sourceToolchain, prefix: '' }],
    toolchainRoot,
    MAX_TOOLCHAIN_ENTRIES,
    MAX_TOOLCHAIN_BYTES,
  );
  const registry = copyTrees([
    { source: registryDirectory(sourceRegistry, 'cache'), prefix: `cache/${REGISTRY_KEY}` },
    { source: registryDirectory(sourceRegistry, 'index'), prefix: `index/${REGISTRY_KEY}` },
    { source: registryDirectory(sourceRegistry, 'src'), prefix: `src/${REGISTRY_KEY}` },
  ], registryRoot, MAX_REGISTRY_ENTRIES, MAX_REGISTRY_BYTES);
  if (toolchain.contentDigest !== TOOLCHAIN_CONTENT_DIGEST
    || registry.contentDigest !== REGISTRY_CONTENT_DIGEST) {
    throw new Error('HARNESS_RUST_CLOSURE_CONTENT_MISMATCH');
  }
  hardenTree(toolchainRoot);
  hardenTree(registryRoot);
  const metadataDigest = metadataTreeDigest([
    { source: toolchainRoot, prefix: 'toolchain' },
    { source: registryRoot, prefix: 'registry' },
  ]);
  const assertStable = () => {
    if (metadataTreeDigest([
      { source: toolchainRoot, prefix: 'toolchain' },
      { source: registryRoot, prefix: 'registry' },
    ]) !== metadataDigest) throw new Error('HARNESS_RUST_CLOSURE_CHANGED');
  };
  assertStable();
  return Object.freeze({
    toolchainRoot,
    registryRoot,
    cargoExecutable: join(toolchainRoot, 'bin', 'cargo'),
    evidence: Object.freeze({
      rustToolchainClosure: `${TOOLCHAIN_CONTENT_DIGEST}:${String(toolchain.entries)}:${String(toolchain.bytes)}`,
      rustRegistryClosure: `${REGISTRY_CONTENT_DIGEST}:${String(registry.entries)}:${String(registry.bytes)}`,
      rustClosureMetadata: metadataDigest,
    }),
    assertStable,
  });
}

function copyTrees(
  sources: readonly TreeSource[],
  destinationRoot: string,
  maxEntries: number,
  maxBytes: number,
): CopiedTree {
  const content = createHash('sha256');
  const emittedDirectories = new Set<string>();
  let entries = 0;
  let bytes = 0;
  const emit = (kind: 'd' | 'f', path: string, extra = '') => {
    content.update(`${kind}\0${path}\0${extra}\0`, 'utf8');
    entries += 1;
    if (entries > maxEntries) throw new Error('HARNESS_RUST_CLOSURE_ENTRY_LIMIT');
  };
  const createVirtualDirectory = (path: string) => {
    if (path === '' || emittedDirectories.has(path)) return;
    const parent = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
    createVirtualDirectory(parent);
    mkdirSync(join(destinationRoot, path), { recursive: true, mode: 0o700 });
    emittedDirectories.add(path);
    emit('d', path);
  };
  for (const { source, prefix } of sources) {
    createVirtualDirectory(prefix);
    const visit = (directory: string) => {
      for (const name of readdirSync(directory).sort()) {
        const sourcePath = join(directory, name);
        const stat = lstatSync(sourcePath, { bigint: true });
        const child = relative(source, sourcePath).split(sep).join('/');
        const virtual = prefix === '' ? child : `${prefix}/${child}`;
        if (stat.isDirectory() && !stat.isSymbolicLink()
          && realpathSync(sourcePath) === sourcePath) {
          createVirtualDirectory(virtual);
          visit(sourcePath);
          continue;
        }
        if (!stat.isFile() || stat.isSymbolicLink() || realpathSync(sourcePath) !== sourcePath
          || stat.size > BigInt(MAX_FILE_BYTES)) {
          throw new Error('HARNESS_RUST_CLOSURE_SOURCE_INVALID');
        }
        const target = join(destinationRoot, virtual);
        const copied = copyFile(sourcePath, target, (stat.mode & 0o111n) !== 0n);
        bytes += copied.bytes;
        if (bytes > maxBytes) throw new Error('HARNESS_RUST_CLOSURE_BYTE_LIMIT');
        emit('f', virtual, `${copied.executable ? 'x' : 'r'}\0${String(copied.bytes)}\0${copied.digest}`);
      }
    };
    visit(source);
  }
  return Object.freeze({ contentDigest: content.digest('hex'), entries, bytes });
}

function copyFile(source: string, target: string, executable: boolean) {
  const input = openSync(source, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  const output = openSync(target, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL, 0o600);
  try {
    const before = fstatSync(input, { bigint: true });
    const hash = createHash('sha256');
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < before.size) {
      const count = readSync(
        input,
        buffer,
        0,
        Math.min(buffer.length, Number(before.size - offset)),
        Number(offset),
      );
      if (count === 0) break;
      writeAll(output, buffer, count, Number(offset));
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    const after = fstatSync(input, { bigint: true });
    const written = fstatSync(output, { bigint: true });
    if (offset !== before.size || written.size !== before.size || !sameIdentity(before, after)) {
      throw new Error('HARNESS_RUST_CLOSURE_SOURCE_CHANGED');
    }
    chmodSync(target, executable ? 0o500 : 0o400);
    return Object.freeze({
      digest: hash.digest('hex'),
      bytes: Number(before.size),
      executable,
    });
  } finally {
    closeSync(input);
    closeSync(output);
  }
}

function hardenTree(root: string): void {
  const visit = (directory: string) => {
    for (const name of readdirSync(directory).sort()) {
      const path = join(directory, name);
      const stat = lstatSync(path);
      if (stat.isDirectory() && !stat.isSymbolicLink()) visit(path);
      else if (!stat.isFile() || stat.isSymbolicLink()) {
        throw new Error('HARNESS_RUST_CLOSURE_COPY_INVALID');
      }
    }
    chmodSync(directory, 0o700);
  };
  visit(root);
}

function metadataTreeDigest(sources: readonly TreeSource[]): string {
  const hash = createHash('sha256');
  for (const { source, prefix } of sources) {
    const visit = (directory: string) => {
      for (const name of readdirSync(directory).sort()) {
        const path = join(directory, name);
        const stat = lstatSync(path, { bigint: true });
        const child = relative(source, path).split(sep).join('/');
        const virtual = prefix === '' ? child : `${prefix}/${child}`;
        if (stat.isDirectory() && !stat.isSymbolicLink()
          && realpathSync(path) === path) {
          metadataEntry(hash, 'd', virtual, stat);
          visit(path);
        } else if (stat.isFile() && !stat.isSymbolicLink()
          && realpathSync(path) === path) {
          metadataEntry(hash, 'f', virtual, stat);
        } else throw new Error('HARNESS_RUST_CLOSURE_COPY_INVALID');
      }
    };
    visit(source);
  }
  return hash.digest('hex');
}

function metadataEntry(
  hash: ReturnType<typeof createHash>,
  kind: 'd' | 'f',
  path: string,
  stat: BigIntStats,
): void {
  hash.update([
    kind, path, String(stat.dev), String(stat.ino), String(stat.mode), String(stat.size),
    String(stat.mtimeNs), String(stat.ctimeNs), '',
  ].join('\0'), 'utf8');
}

function registryDirectory(root: string, kind: 'cache' | 'index' | 'src'): string {
  return canonicalDirectory(
    join(root, kind, REGISTRY_KEY),
    `HARNESS_RUST_CLOSURE_REGISTRY_${kind.toUpperCase()}_INVALID`,
  );
}

function canonicalDirectory(value: string, error: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) throw new Error(error);
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error(error);
  }
  return value;
}

function sameIdentity(left: BigIntStats, right: BigIntStats) {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.size === right.size && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function writeAll(descriptor: number, buffer: Buffer, length: number, position: number): void {
  let written = 0;
  while (written < length) {
    written += writeSync(descriptor, buffer, written, length - written, position + written);
  }
}

// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync,
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  mkdtempSync,
  openSync,
  readSync,
  realpathSync,
  type BigIntStats,
  writeFileSync,
} from 'node:fs';
import { basename, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { normalizeWorkspacePath } from './contracts.js';
import { resolveWorkspacePath } from './workspace.js';

export interface WritableFileOverlay {
  readonly source: string;
  readonly destination: string;
}

export interface WorkspaceFileOverlayLease {
  readonly mounts: readonly WritableFileOverlay[];
  assertOriginalsStable(): void;
  seal(): Readonly<Record<string, string>>;
}

interface FileIdentity {
  readonly dev: bigint;
  readonly ino: bigint;
  readonly mode: bigint;
  readonly size: bigint;
  readonly mtimeNs: bigint;
  readonly ctimeNs: bigint;
  readonly digest: string;
}

interface FileCapture {
  readonly identity: FileIdentity;
  readonly bytes: Buffer;
}

const MAX_EARL_BYTES = 10_000_000;

export function prepareWorkspaceFileOverlays(input: Readonly<{
  controlledRoot: string;
  workspaceRoot: string;
  outputRoot: string;
  workspacePaths: readonly string[];
}>): WorkspaceFileOverlayLease {
  const controlledRoot = canonicalDirectory(input.controlledRoot, 'HARNESS_OVERLAY_CONTROLLED_ROOT_INVALID');
  const workspaceRoot = canonicalDirectory(input.workspaceRoot, 'HARNESS_OVERLAY_WORKSPACE_ROOT_INVALID');
  const outputRoot = canonicalDirectory(input.outputRoot, 'HARNESS_OVERLAY_OUTPUT_ROOT_INVALID');
  assertStrictChild(controlledRoot, workspaceRoot, 'HARNESS_OVERLAY_WORKSPACE_UNCONTROLLED');
  assertStrictChild(controlledRoot, outputRoot, 'HARNESS_OVERLAY_OUTPUT_UNCONTROLLED');
  if (input.workspacePaths.length === 0) return Object.freeze({
    mounts: [], assertOriginalsStable() {}, seal: () => ({}),
  });

  const paths = input.workspacePaths.map((path, index) =>
    normalizeWorkspacePath(path, `workspacePaths[${index}]`));
  if (new Set(paths).size !== paths.length) throw new Error('HARNESS_OVERLAY_PATH_DUPLICATE');
  const overlayRoot = mkdtempSync(join(outputRoot, 'writable-file-overlays-'));
  chmodSync(overlayRoot, 0o700);
  const originals = new Map<string, FileIdentity>();
  const overlayIdentities = new Map<string, FileIdentity>();
  const mounts = paths.map((path) => {
    const destination = resolveWorkspacePath(workspaceRoot, path, {
      requireRegularFile: true,
      rejectHardlinks: true,
    });
    const identity = captureFile(destination, 'HARNESS_OVERLAY_DESTINATION_INVALID').identity;
    originals.set(path, identity);
    const name = `${digest(path).slice(0, 24)}-${basename(path)}`;
    const source = join(overlayRoot, name);
    writeFileSync(source, `HARNESS_GENERATION_REQUIRED:${path}\n`, {
      encoding: 'utf8', flag: 'wx', mode: 0o600,
    });
    chmodSync(source, 0o600);
    overlayIdentities.set(
      path,
      captureFile(source, 'HARNESS_OVERLAY_SOURCE_INVALID', true).identity,
    );
    return Object.freeze({ source, destination });
  });

  const assertOriginalsStable = () => {
    for (let index = 0; index < paths.length; index += 1) {
      const path = paths[index];
      const original = captureFile(
        mounts[index].destination,
        'HARNESS_OVERLAY_DESTINATION_CHANGED',
      ).identity;
      if (!sameFile(original, originals.get(path)!)) {
        throw new Error(`HARNESS_OVERLAY_DESTINATION_CHANGED:${path}`);
      }
    }
  };
  return Object.freeze({
    mounts: Object.freeze(mounts),
    assertOriginalsStable,
    seal(): Readonly<Record<string, string>> {
      assertOriginalsStable();
      const results: Record<string, string> = {};
      for (let index = 0; index < paths.length; index += 1) {
        const path = paths[index];
        const mount = mounts[index];
        const generated = captureFile(
          mount.source,
          'HARNESS_OVERLAY_SOURCE_CHANGED',
          true,
        );
        const initial = overlayIdentities.get(path)!;
        if (generated.identity.dev !== initial.dev || generated.identity.ino !== initial.ino
          || generated.identity.digest === initial.digest) {
          throw new Error(`HARNESS_OVERLAY_NOT_GENERATED:${path}`);
        }
        assertEarlTurtle(generated.bytes, path);
        results[path] = generated.identity.digest;
      }
      return Object.freeze(results);
    },
  });
}

function assertEarlTurtle(bytes: Buffer, path: string): void {
  let text: string;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`HARNESS_OVERLAY_EARL_INVALID:${path}`);
  }
  if (bytes.length > MAX_EARL_BYTES
    || !text.startsWith('@prefix earl: <http://www.w3.org/ns/earl#> .\n')
    || !text.includes('a earl:Assertion')
    || !text.includes('earl:subject <https://example.org/tools/semantic-fabric>')
    || !/earl:outcome earl:(?:passed|failed|untested)/.test(text)) {
    throw new Error(`HARNESS_OVERLAY_EARL_INVALID:${path}`);
  }
}

export function parseWritableFileOverlays(
  value: unknown,
  label: string,
): readonly WritableFileOverlay[] {
  if (!Array.isArray(value)) throw new TypeError(`${label} must be an array`);
  const sources = new Set<string>();
  const destinations = new Set<string>();
  return Object.freeze(value.map((entry, index) => {
    if (entry === null || typeof entry !== 'object' || Array.isArray(entry)) {
      throw new TypeError(`${label}[${index}] must be an object`);
    }
    const keys = Object.keys(entry).sort();
    if (JSON.stringify(keys) !== JSON.stringify(['destination', 'source'])) {
      throw new TypeError(`${label}[${index}] has unexpected keys`);
    }
    const { source, destination } = entry as Record<string, unknown>;
    const parsedSource = absolutePath(source, `${label}[${index}].source`);
    const parsedDestination = absolutePath(destination, `${label}[${index}].destination`);
    if (parsedSource === parsedDestination || sources.has(parsedSource)
      || destinations.has(parsedDestination)) {
      throw new TypeError(`${label} contains duplicate or self overlays`);
    }
    sources.add(parsedSource);
    destinations.add(parsedDestination);
    return Object.freeze({ source: parsedSource, destination: parsedDestination });
  }));
}

function canonicalDirectory(value: string, error: string): string {
  try {
    const path = absolutePath(value, error);
    const stat = lstatSync(path);
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(path) !== path) throw new Error();
    return path;
  } catch {
    throw new Error(error);
  }
}

function captureFile(path: string, error: string, requirePrivate = false): FileCapture {
  try {
    const pathStat = lstatSync(path, { bigint: true });
    const uid = BigInt(process.getuid?.() ?? Number(pathStat.uid));
    if (!pathStat.isFile() || pathStat.isSymbolicLink() || pathStat.nlink !== 1n
      || realpathSync(path) !== path || pathStat.size < 1n
      || pathStat.size > BigInt(MAX_EARL_BYTES)
      || (requirePrivate && pathStat.uid !== uid)
      || (requirePrivate && (pathStat.mode & 0o077n) !== 0n)) throw new Error();
    const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    try {
      const before = fstatSync(descriptor, { bigint: true });
      if (!sameStat(pathStat, before)) throw new Error();
      const bytes = Buffer.alloc(Number(before.size));
      let offset = 0;
      while (offset < bytes.length) {
        const read = readSync(descriptor, bytes, offset, bytes.length - offset, offset);
        if (read === 0) break;
        offset += read;
      }
      const after = fstatSync(descriptor, { bigint: true });
      if (offset !== bytes.length || !sameStat(before, after)) throw new Error();
      return Object.freeze({
        identity: Object.freeze({
          dev: before.dev,
          ino: before.ino,
          mode: before.mode,
          size: before.size,
          mtimeNs: before.mtimeNs,
          ctimeNs: before.ctimeNs,
          digest: digest(bytes),
        }),
        bytes,
      });
    } finally {
      closeSync(descriptor);
    }
  } catch {
    throw new Error(error);
  }
}

function assertStrictChild(parent: string, child: string, error: string): void {
  const delta = relative(parent, child);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error(error);
  }
}

function absolutePath(value: unknown, label: string): string {
  if (typeof value !== 'string' || !isAbsolute(value) || resolve(value) !== value
    || value.includes('\0')) throw new TypeError(`${label} must be an absolute normalized path`);
  return value;
}

function digest(value: string | Buffer): string {
  return createHash('sha256').update(value).digest('hex');
}

function sameFile(left: FileIdentity, right: FileIdentity): boolean {
  return left.dev === right.dev && left.ino === right.ino
    && left.mode === right.mode && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs
    && left.digest === right.digest;
}

function sameStat(left: BigIntStats, right: BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.size === right.size && left.mtimeNs === right.mtimeNs
    && left.ctimeNs === right.ctimeNs;
}

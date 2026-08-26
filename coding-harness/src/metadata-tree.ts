// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  lstatSync,
  readdirSync,
  realpathSync,
  watch,
  type BigIntStats,
  type FSWatcher,
} from 'node:fs';
import { join, relative, sep } from 'node:path';

export interface MetadataTreeSource {
  readonly source: string;
  readonly prefix: string;
}

export interface MetadataTreeOptions {
  readonly maxEntries: number;
  readonly invalidCode: string;
  readonly yieldEvery?: number;
}

export function metadataTreeDigest(
  sources: readonly MetadataTreeSource[],
  options: MetadataTreeOptions,
): string {
  const hash = createHash('sha256');
  for (const entry of metadataEntries(sources, options)) updateDigest(hash, entry);
  return hash.digest('hex');
}

export async function cooperativeMetadataTreeDigest(
  sources: readonly MetadataTreeSource[],
  options: MetadataTreeOptions,
): Promise<string> {
  const yieldEvery = positiveInteger(options.yieldEvery ?? 128, 'yieldEvery');
  const hash = createHash('sha256');
  let visited = 0;
  for (const entry of metadataEntries(sources, options)) {
    updateDigest(hash, entry);
    visited += 1;
    if (visited % yieldEvery === 0) await new Promise<void>((resolve) => setImmediate(resolve));
  }
  return hash.digest('hex');
}

export function cooperativeMetadataAssertion(
  sources: readonly MetadataTreeSource[],
  expectedDigest: string,
  options: MetadataTreeOptions,
  changedCode: string,
): () => Promise<void> {
  let active: Promise<void> | null = null;
  return () => {
    if (active === null) {
      active = observedMetadataTreeDigest(sources, options, changedCode)
        .then((digest) => {
          if (digest !== expectedDigest) throw new Error(changedCode);
        })
        .catch((cause: unknown) => {
          if (cause instanceof Error && cause.message === changedCode) throw cause;
          throw new Error(changedCode, { cause });
        })
        .finally(() => { active = null; });
    }
    return active;
  };
}

async function observedMetadataTreeDigest(
  sources: readonly MetadataTreeSource[],
  options: MetadataTreeOptions,
  changedCode: string,
): Promise<string> {
  const watchers: FSWatcher[] = [];
  let changed = false;
  let watcherFailure: unknown;
  try {
    for (const source of new Set(sources.map(({ source }) => source))) {
      const watcher = watch(source, { recursive: true }, () => { changed = true; });
      watcher.on('error', (error) => { watcherFailure ??= error; });
      watchers.push(watcher);
    }
    const digest = await cooperativeMetadataTreeDigest(sources, options);
    await new Promise<void>((resolve) => setImmediate(resolve));
    await new Promise<void>((resolve) => setImmediate(resolve));
    // Filesystem watches are advisory and can coalesce or drop a short-lived
    // event under CI load. Recompute after the drain so a mutation that raced
    // the cooperative scan is still a hard failure, not a timing-dependent
    // clean result.
    const settledDigest = metadataTreeDigest(sources, options);
    if (changed || watcherFailure !== undefined || digest !== settledDigest) throw new Error(changedCode, {
      cause: watcherFailure,
    });
    return digest;
  } finally {
    for (const watcher of watchers) watcher.close();
  }
}

interface MetadataEntry {
  readonly kind: 'd' | 'f';
  readonly path: string;
  readonly stat: BigIntStats;
}

function* metadataEntries(
  sources: readonly MetadataTreeSource[],
  options: MetadataTreeOptions,
): Generator<MetadataEntry> {
  const maxEntries = positiveInteger(options.maxEntries, 'maxEntries');
  let entries = 0;
  for (const { source, prefix } of sources) {
    function* visit(directory: string): Generator<MetadataEntry> {
      for (const name of readdirSync(directory).sort()) {
        const path = join(directory, name);
        const stat = lstatSync(path, { bigint: true });
        const child = relative(source, path).split(sep).join('/');
        const virtual = prefix === '' ? child : `${prefix}/${child}`;
        entries += 1;
        if (entries > maxEntries) throw new Error(options.invalidCode);
        if (stat.isDirectory() && !stat.isSymbolicLink() && realpathSync(path) === path) {
          yield { kind: 'd', path: virtual, stat };
          yield* visit(path);
        } else if (stat.isFile() && !stat.isSymbolicLink() && realpathSync(path) === path) {
          yield { kind: 'f', path: virtual, stat };
        } else {
          throw new Error(options.invalidCode);
        }
      }
    }
    yield* visit(source);
  }
}

function updateDigest(
  hash: ReturnType<typeof createHash>,
  entry: MetadataEntry,
): void {
  const { kind, path, stat } = entry;
  hash.update([
    kind, path, String(stat.dev), String(stat.ino), String(stat.mode), String(stat.size),
    String(stat.mtimeNs), String(stat.ctimeNs), '',
  ].join('\0'), 'utf8');
}

function positiveInteger(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 1) throw new TypeError(`${label} must be positive`);
  return value;
}

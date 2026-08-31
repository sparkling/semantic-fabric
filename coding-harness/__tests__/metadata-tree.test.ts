// SPDX-License-Identifier: MIT

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  cooperativeMetadataAssertion,
  cooperativeMetadataTreeDigest,
  metadataTreeDigest,
} from '../src/metadata-tree.js';

const watchProbe = vi.hoisted(() => ({
  calls: [] as Array<{ path: string; recursive: unknown }>,
  closed: 0,
  errorListeners: [] as Array<(error: Error) => void>,
  fake: false,
}));

vi.mock('node:fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs')>();
  const delegate = actual.watch as unknown as (...args: unknown[]) => import('node:fs').FSWatcher;
  return {
    ...actual,
    watch: ((...args: unknown[]) => {
      const [path, options] = args;
      const recursive = options !== null && typeof options === 'object'
        ? (options as { recursive?: unknown }).recursive : undefined;
      watchProbe.calls.push({ path: String(path), recursive });
      if (!watchProbe.fake) return delegate(...args);
      const watcher = {
        on: (event: string, listener: (...values: unknown[]) => void) => {
          if (event === 'error') {
            watchProbe.errorListeners.push(listener as (error: Error) => void);
          }
          return watcher;
        },
        close: () => { watchProbe.closed += 1; },
      };
      return watcher as unknown as import('node:fs').FSWatcher;
    }) as typeof actual.watch,
  };
});

const roots: string[] = [];
const METADATA_WATCHER_CAP = 8_192;

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  watchProbe.calls.length = 0;
  watchProbe.closed = 0;
  watchProbe.errorListeners.length = 0;
  watchProbe.fake = false;
});

describe('cooperative metadata tree integrity', () => {
  it('matches the synchronous digest while yielding the controller event loop', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-metadata-'));
    roots.push(root);
    mkdirSync(join(root, 'nested'));
    for (let index = 0; index < 256; index += 1) {
      writeFileSync(join(root, 'nested', `${String(index).padStart(3, '0')}.txt`), 'sealed\n');
    }
    const sources = [{ source: root, prefix: 'closure' }];
    const options = { maxEntries: 300, invalidCode: 'HARNESS_TEST_TREE_INVALID' };
    const expected = metadataTreeDigest(sources, options);
    let eventLoopAdvanced = false;
    setImmediate(() => { eventLoopAdvanced = true; });

    await expect(cooperativeMetadataTreeDigest(sources, options)).resolves.toBe(expected);
    expect(eventLoopAdvanced).toBe(true);
  });

  it('coalesces concurrent checks and rejects a changed closure', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-metadata-change-'));
    roots.push(root);
    const path = join(root, 'sealed.txt');
    writeFileSync(path, 'before\n');
    const sources = [{ source: root, prefix: '' }];
    const options = { maxEntries: 2, invalidCode: 'HARNESS_TEST_TREE_INVALID', yieldEvery: 1 };
    const expected = metadataTreeDigest(sources, options);
    const assertStable = cooperativeMetadataAssertion(
      sources, expected, options, 'HARNESS_TEST_TREE_CHANGED',
    );

    expect(assertStable()).toBe(assertStable());
    await assertStable();
    writeFileSync(path, 'after\n');
    await expect(assertStable()).rejects.toThrow('HARNESS_TEST_TREE_CHANGED');
    rmSync(root, { recursive: true, force: true });
    await expect(assertStable()).rejects.toThrow('HARNESS_TEST_TREE_CHANGED');
  });

  it('rejects a mutation behind the cooperative scan cursor', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-metadata-race-'));
    roots.push(root);
    const nested = join(root, 'nested');
    mkdirSync(nested);
    for (let index = 0; index < 256; index += 1) {
      writeFileSync(join(nested, `${String(index).padStart(3, '0')}.txt`), 'sealed\n');
    }
    const first = join(nested, '000.txt');
    const sources = [{ source: root, prefix: '' }];
    const options = {
      maxEntries: 300,
      invalidCode: 'HARNESS_TEST_TREE_INVALID',
      yieldEvery: 128,
    };
    const expected = metadataTreeDigest(sources, options);
    const assertStable = cooperativeMetadataAssertion(
      sources, expected, options, 'HARNESS_TEST_TREE_CHANGED',
    );
    setImmediate(() => writeFileSync(first, 'changed\n'));

    await expect(assertStable()).rejects.toThrow('HARNESS_TEST_TREE_CHANGED');
    expect(metadataTreeDigest(sources, options)).not.toBe(expected);
  });

  it('drains a same-check-phase mutation on a one-file tree', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-metadata-drain-'));
    roots.push(root);
    const path = join(root, 'sealed.txt');
    writeFileSync(path, 'before\n');
    const sources = [{ source: root, prefix: '' }];
    const options = { maxEntries: 2, invalidCode: 'HARNESS_TEST_TREE_INVALID' };
    const expected = metadataTreeDigest(sources, options);
    const assertStable = cooperativeMetadataAssertion(
      sources, expected, options, 'HARNESS_TEST_TREE_CHANGED',
    );
    setImmediate(() => writeFileSync(path, 'after\n'));

    await expect(assertStable()).rejects.toThrow('HARNESS_TEST_TREE_CHANGED');
    expect(metadataTreeDigest(sources, options)).not.toBe(expected);
  });

  it('watches every root and directory explicitly without recursive Node watchers', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-metadata-watch-tree-'));
    roots.push(root);
    const nested = join(root, 'nested'); const deep = join(nested, 'deep');
    mkdirSync(deep, { recursive: true });
    writeFileSync(join(deep, 'sealed.txt'), 'sealed\n');
    const sources = [{ source: root, prefix: '' }];
    const options = { maxEntries: 3, invalidCode: 'HARNESS_TEST_TREE_INVALID' };
    watchProbe.calls.length = 0; watchProbe.closed = 0; watchProbe.fake = true;
    try {
      const assertStable = cooperativeMetadataAssertion(
        sources, metadataTreeDigest(sources, options), options, 'HARNESS_TEST_TREE_CHANGED',
      );
      await assertStable();
      expect(watchProbe.calls).toEqual([root, nested, deep]
        .map((path) => ({ path, recursive: false })));
      expect(watchProbe.closed).toBe(3);
    } finally {
      watchProbe.fake = false;
    }
  });

  it('fails closed on a watcher error and closes the complete watch set', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-metadata-watch-error-'));
    roots.push(root);
    const nested = join(root, 'nested');
    mkdirSync(nested);
    writeFileSync(join(nested, 'sealed.txt'), 'sealed\n');
    const sources = [{ source: root, prefix: '' }];
    const options = { maxEntries: 2, invalidCode: 'HARNESS_TEST_TREE_INVALID' };
    watchProbe.fake = true;
    const assertStable = cooperativeMetadataAssertion(
      sources, metadataTreeDigest(sources, options), options, 'HARNESS_TEST_TREE_CHANGED',
    );

    const pending = assertStable();
    expect(watchProbe.errorListeners).toHaveLength(2);
    watchProbe.errorListeners[1]!(new Error('simulated watcher failure'));
    await expect(pending).rejects.toThrow('HARNESS_TEST_TREE_CHANGED');
    expect(watchProbe.closed).toBe(2);
  });

  it('fails closed at the explicit watcher cap and closes the partial watch tree', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-metadata-watch-cap-'));
    roots.push(root);
    for (let index = 0; index < METADATA_WATCHER_CAP; index += 1) {
      mkdirSync(join(root, String(index).padStart(5, '0')));
    }
    const sources = [{ source: root, prefix: '' }];
    const options = {
      maxEntries: METADATA_WATCHER_CAP,
      invalidCode: 'HARNESS_TEST_TREE_INVALID',
      yieldEvery: METADATA_WATCHER_CAP,
    };
    const expected = metadataTreeDigest(sources, options);
    watchProbe.calls.length = 0; watchProbe.closed = 0; watchProbe.fake = true;
    try {
      const assertStable = cooperativeMetadataAssertion(
        sources, expected, options, 'HARNESS_TEST_TREE_CHANGED',
      );
      await expect(assertStable()).rejects.toThrow('HARNESS_TEST_TREE_CHANGED');
      expect(watchProbe.calls).toHaveLength(METADATA_WATCHER_CAP);
      expect(watchProbe.closed).toBe(METADATA_WATCHER_CAP);
    } finally {
      watchProbe.fake = false;
    }
  });
});

// SPDX-License-Identifier: MIT

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  cooperativeMetadataAssertion,
  cooperativeMetadataTreeDigest,
  metadataTreeDigest,
} from '../src/metadata-tree.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
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
    for (let index = 0; index < 256; index += 1) {
      writeFileSync(join(root, `${String(index).padStart(3, '0')}.txt`), 'sealed\n');
    }
    const first = join(root, '000.txt');
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
});

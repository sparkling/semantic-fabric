// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync, cpSync, mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  assertCaptureControllerStoreStableV1,
  openCaptureControllerStoreV1,
  parseCaptureTreeListingV1,
  parseVerifiedGitBatchV1,
  type GitObjectRequestV1,
} from '../src/programme-capture-git-v1.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme capture V1 Git authority boundary', () => {
  it('detects replacement of a pinned primary Git directory', async () => {
    const root = repository();
    const store = await openCaptureControllerStoreV1(root);
    const original = join(root, '.git-original');
    renameSync(join(root, '.git'), original);
    cpSync(original, join(root, '.git'), { recursive: true });
    chmodSync(join(root, '.git'), 0o755);
    chmodSync(join(root, '.git', 'objects'), 0o755);
    await expect(assertCaptureControllerStoreStableV1(store)).rejects.toThrow(
      /CONTROLLER_STORE_CHANGED/,
    );
  });

  it('rejects a store whose parent grants rename authority without a sticky bit', async () => {
    const root = repository(0o770);
    await expect(openCaptureControllerStoreV1(root)).rejects.toThrow(/STORE_INVALID/);
  });

  it('caps tree entries before allocating the entry map', () => {
    const record = `100644 blob ${'a'.repeat(40)}\tsame\0`;
    const listing = Buffer.from(record.repeat(100_001), 'utf8');
    expect(() => parseCaptureTreeListingV1(listing, 40)).toThrow(/COMMIT_TREE_INVALID/);
  });

  it('accepts only ordered, canonical batches whose bytes match native Git IDs', () => {
    const first = Buffer.from('trusted\n');
    const second = Buffer.from('second\n');
    const firstRequest = request(first);
    const secondRequest = request(second);
    const firstBatch = batch(firstRequest, first);
    const secondBatch = batch(secondRequest, second);
    expect(parseVerifiedGitBatchV1(
      firstBatch, [firstRequest], 100, 100, 'sha1',
    ).get(firstRequest.id)).toEqual(first);

    const hostile = Buffer.from('hostile\n');
    const mutants = [
      batch(firstRequest, hostile),
      Buffer.from(`${firstRequest.id} blob 0008\ntrusted\n\n`),
      Buffer.from(`${'f'.repeat(40)} blob 8\ntrusted\n\n`),
      Buffer.from(`${firstRequest.id} tree 8\ntrusted\n\n`),
      firstBatch.subarray(0, firstBatch.length - 1),
      Buffer.concat([firstBatch, Buffer.from('extra')]),
      Buffer.concat([Buffer.from([0xff]), firstBatch.subarray(1)]),
      Buffer.concat([secondBatch, firstBatch]),
      Buffer.alloc(0),
    ];
    for (const mutant of mutants) {
      expect(() => parseVerifiedGitBatchV1(
        mutant, [firstRequest, ...(mutant === mutants[7] ? [secondRequest] : [])],
        100, 200, 'sha1',
      )).toThrow();
    }
    expect(() => parseVerifiedGitBatchV1(
      Buffer.concat([firstBatch, firstBatch]),
      [firstRequest, firstRequest],
      100,
      200,
      'sha1',
    )).toThrow(/REQUEST_INVALID/);
  });
});

function repository(parentMode = 0o700): string {
  const parent = mkdtempSync(join(tmpdir(), 'programme-capture-git-'));
  roots.push(parent);
  chmodSync(parent, parentMode);
  const root = join(parent, 'repository');
  mkdirSync(root, { mode: 0o700 });
  writeFileSync(join(root, 'source.txt'), 'trusted\n');
  git(root, ['init', '--quiet']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  git(root, ['add', '--all']);
  git(root, ['commit', '--quiet', '-m', 'trusted']);
  chmodSync(join(root, '.git'), 0o755);
  chmodSync(join(root, '.git', 'objects'), 0o755);
  return root;
}

function request(body: Buffer): GitObjectRequestV1 {
  return { id: gitObjectId('blob', body), type: 'blob' };
}

function batch(requestValue: GitObjectRequestV1, body: Buffer): Buffer {
  return Buffer.concat([
    Buffer.from(`${requestValue.id} ${requestValue.type} ${body.length}\n`, 'ascii'),
    body,
    Buffer.from('\n'),
  ]);
}

function gitObjectId(type: string, body: Buffer): string {
  return createHash('sha1').update(`${type} ${body.length}\0`).update(body).digest('hex');
}

function git(root: string, args: readonly string[]): void {
  const result = spawnSync('/usr/bin/git', args, {
    cwd: root,
    env: {
      PATH: '/usr/bin:/bin', HOME: '/nonexistent', LANG: 'C', LC_ALL: 'C',
      GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
    },
    encoding: 'utf8',
  });
  if (result.status !== 0) throw new Error(result.stderr);
}

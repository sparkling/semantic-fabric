// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import {
  chmod, link, lstat, mkdir, mkdtemp, readFile, rename, rm, symlink, writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { createImmutablePrivateRuntime } from '../src/immutable-private-runtime.js';
import {
  assertImmutablePrivateTreeOverridesStable,
  captureImmutablePrivateTreeOverrides,
  immutablePrivateTreeOverridePaths,
} from '../src/immutable-private-tree-overlay.js';

interface OverlayEntry {
  targetPath: string;
  blobPath: string;
  compression: 'gzip';
  compressedSha256: string;
  compressedBytes: number;
  decodedSha256: string;
  decodedBytes: number;
  executable: false;
}

interface Fixture {
  readonly parent: string;
  readonly source: string;
  readonly target: string;
  readonly blob: string;
  readonly decoded: Buffer;
  manifest: { schemaVersion: 1; files: OverlayEntry[] };
  manifestDigest: string;
  manifestBytes: number;
}

const roots: string[] = [];

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe('immutable private tree exact overlays', () => {
  it('fails closed on a group-writable ambient file without an override', async () => {
    const fixture = await makeFixture();
    expect(() => materialize(fixture, false))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_SOURCE_UNTRUSTED');
  });

  it.each([0o000, 0o644, 0o664, 0o777])(
    'materializes exact decoded bytes without opening an ambient target at mode %s',
    async (mode) => {
      const fixture = await makeFixture(mode);
      const runtime = materialize(fixture);
      try {
        const target = join(runtime.root, 'tree/src/memory/entry.js');
        expect(await readFile(target)).toEqual(fixture.decoded);
        expect((await lstat(target)).mode & 0o777).toBe(0o400);
        expect(runtime.trees.fixture).toMatchObject({
          digest: treeDigest(fixture.decoded),
          fileCount: 1,
          totalBytes: fixture.decoded.byteLength,
        });
      } finally {
        runtime.cleanup();
      }
    },
  );

  it('rejects a manifest changed after its digest was pinned', async () => {
    const fixture = await makeFixture();
    const bytes = await readFile(join(fixture.parent, 'overlay.json'));
    bytes[bytes.indexOf(Buffer.from('entry.js'))] = 'E'.charCodeAt(0);
    await writeFile(join(fixture.parent, 'overlay.json'), bytes);
    await chmod(join(fixture.parent, 'overlay.json'), 0o644);
    expect(() => materialize(fixture))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_DIGEST_MISMATCH');
  });

  it('rejects duplicate JSON keys even when the manifest digest is repinned', async () => {
    const fixture = await makeFixture();
    const path = join(fixture.parent, 'overlay.json');
    const canonical = (await readFile(path, 'utf8'));
    const bytes = Buffer.from(canonical.replace(
      '  "schemaVersion": 1,',
      '  "schemaVersion": 1,\n  "schemaVersion": 1,',
    ));
    await writeFile(path, bytes);
    await chmod(path, 0o644);
    fixture.manifestDigest = sha256(bytes);
    fixture.manifestBytes = bytes.byteLength;
    expect(() => materialize(fixture))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_INVALID');
  });

  it.each([
    ['extra root key', (fixture: Fixture) => {
      Object.assign(fixture.manifest, { extra: false });
    }],
    ['extra file descriptor key', (fixture: Fixture) => {
      Object.assign(fixture.manifest.files[0]!, { extra: false });
    }],
    ['renamed file descriptor key', (fixture: Fixture) => {
      const entry = fixture.manifest.files[0]! as unknown as Record<string, unknown>;
      entry.target = entry.targetPath;
      delete entry.targetPath;
    }],
  ] as const)('rejects a canonical manifest with an %s', async (_label, mutate) => {
    const fixture = await makeFixture();
    mutate(fixture);
    await repinManifest(fixture);
    expect(() => materialize(fixture))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_INVALID');
  });

  it('rejects repinned noncanonical whitespace', async () => {
    const fixture = await makeFixture();
    const bytes = Buffer.from(JSON.stringify(fixture.manifest));
    await repinRawManifest(fixture, bytes);
    expect(() => materialize(fixture))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_INVALID');
  });

  it('uses UTF-8 byte order and rejects a repinned two-entry reorder', async () => {
    const ordered = await makeFixture();
    await addManifestEntry(ordered, 'src/memory/z.js', 'z.js.gz');
    await addManifestEntry(ordered, 'src/memory/ä.js', 'umlaut.js.gz');
    ordered.manifest.files.shift();
    await repinManifest(ordered);
    expect(immutablePrivateTreeOverridePaths(captureImmutablePrivateTreeOverrides(
      manifestSpec(ordered), fixtureBounds(),
    ))).toEqual(['src/memory/z.js', 'src/memory/ä.js']);

    const reordered = await makeFixture();
    await addManifestEntry(reordered, 'src/memory/ä.js', 'umlaut.js.gz');
    await addManifestEntry(reordered, 'src/memory/z.js', 'z.js.gz');
    reordered.manifest.files.shift();
    await repinManifest(reordered);
    expect(() => captureImmutablePrivateTreeOverrides(
      manifestSpec(reordered), fixtureBounds(),
    )).toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_ORDER_INVALID');
  });

  it('rejects traversal, duplicate, and missing override targets', async () => {
    const traversal = await makeFixture();
    traversal.manifest.files[0]!.targetPath = '../entry.js';
    await repinManifest(traversal);
    expect(() => materialize(traversal))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_TARGET_INVALID');

    const blobTraversal = await makeFixture();
    blobTraversal.manifest.files[0]!.blobPath = '../escape.gz';
    await repinManifest(blobTraversal);
    expect(() => materialize(blobTraversal))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_BLOB_PATH_INVALID');

    const duplicate = await makeFixture();
    duplicate.manifest.files.push({ ...duplicate.manifest.files[0]! });
    await repinManifest(duplicate);
    expect(() => materialize(duplicate))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_TARGET_DUPLICATE');

    const duplicateBlob = await makeFixture();
    duplicateBlob.manifest.files.push({
      ...duplicateBlob.manifest.files[0]!, targetPath: 'src/memory/z.js',
    });
    await repinManifest(duplicateBlob);
    expect(() => materialize(duplicateBlob))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_BLOB_DUPLICATE');

    const missing = await makeFixture();
    missing.manifest.files[0]!.targetPath = 'src/memory/missing.js';
    await repinManifest(missing);
    await chmod(missing.target, 0o644);
    expect(() => materialize(missing))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_TARGET_MISSING');
  });

  it('rejects symlink targets and missing or multiply-linked blobs', async () => {
    const targetLink = await makeFixture();
    await rm(targetLink.target);
    await writeFile(join(targetLink.source, 'elsewhere.js'), 'not historical\n');
    await chmod(join(targetLink.source, 'elsewhere.js'), 0o644);
    await symlink('elsewhere.js', targetLink.target);
    expect(() => materialize(targetLink))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_TARGET_NOT_REGULAR');

    const missingBlob = await makeFixture();
    await rm(missingBlob.blob);
    expect(() => materialize(missingBlob))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_INVALID');

    const linkedBlob = await makeFixture();
    await link(linkedBlob.blob, `${linkedBlob.blob}.link`);
    expect(() => materialize(linkedBlob))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_UNTRUSTED');
  });

  it('rejects protected group/world-write modes and tampered payload bytes', async () => {
    const exact = await makeFixture();
    const runtime = materialize(exact);
    runtime.cleanup();

    const groupWritable = await makeFixture();
    await chmod(groupWritable.blob, 0o664);
    expect(() => materialize(groupWritable))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_UNTRUSTED');

    const groupWritableManifest = await makeFixture();
    await chmod(join(groupWritableManifest.parent, 'overlay.json'), 0o664);
    expect(() => materialize(groupWritableManifest))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_UNTRUSTED');

    const writable = await makeFixture();
    await chmod(writable.blob, 0o666);
    expect(() => materialize(writable))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_UNTRUSTED');

    const executable = await makeFixture();
    await chmod(executable.blob, 0o744);
    expect(() => materialize(executable))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_UNTRUSTED');

    const tampered = await makeFixture();
    const blob = await readFile(tampered.blob);
    blob[Math.floor(blob.byteLength / 2)]! ^= 1;
    await writeFile(tampered.blob, blob);
    await chmod(tampered.blob, 0o644);
    expect(() => materialize(tampered))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_BLOB_DIGEST_MISMATCH');
  });

  it.each([0o444, 0o600])(
    'accepts stable non-writable protected source mode %s', async (mode) => {
      const fixture = await makeFixture();
      await chmod(fixture.blob, mode);
      await chmod(join(fixture.parent, 'overlay.json'), mode);
      const runtime = materialize(fixture);
      runtime.cleanup();
    },
  );

  it('binds the exact protected source mode across captures', async () => {
    const fixture = await makeFixture();
    const before = captureImmutablePrivateTreeOverrides(manifestSpec(fixture), fixtureBounds());
    await chmod(fixture.blob, 0o444);
    const after = captureImmutablePrivateTreeOverrides(manifestSpec(fixture), fixtureBounds());
    expect(() => assertImmutablePrivateTreeOverridesStable(before, after))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_CHANGED');
  });

  it.each(['blob', 'manifest'] as const)(
    'detects an exact-byte %s swap between pre-copy and post-copy captures', async (kind) => {
    const fixture = await makeFixture();
    const spec = manifestSpec(fixture);
    const before = captureImmutablePrivateTreeOverrides(spec, fixtureBounds());
    const path = kind === 'blob' ? fixture.blob : join(fixture.parent, 'overlay.json');
    const bytes = await readFile(path);
    await rename(path, `${path}.replaced`);
    await writeFile(path, bytes);
    await chmod(path, 0o644);
    const after = captureImmutablePrivateTreeOverrides(spec, fixtureBounds());
    expect(() => assertImmutablePrivateTreeOverridesStable(before, after))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_CHANGED');
  });

  it('enforces tree bounds before retaining decoded payloads', async () => {
    const fixture = await makeFixture();
    expect(() => captureImmutablePrivateTreeOverrides(manifestSpec(fixture), {
      maxFiles: 1,
      maxBytes: fixture.decoded.byteLength - 1,
    })).toThrow('HARNESS_IMMUTABLE_RUNTIME_TREE_LIMIT_EXCEEDED');
  });

  it('keeps mutable maps and decoded buffers opaque outside the materializer', async () => {
    const fixture = await makeFixture();
    const snapshot = captureImmutablePrivateTreeOverrides(
      manifestSpec(fixture), fixtureBounds(),
    );
    const paths = immutablePrivateTreeOverridePaths(snapshot);
    expect(Object.keys(snapshot)).toEqual([]);
    expect(Object.isFrozen(snapshot)).toBe(true);
    expect((snapshot as unknown as { files?: unknown }).files).toBeUndefined();
    expect(paths).toEqual(['src/memory/entry.js']);
    expect(Object.isFrozen(paths)).toBe(true);
    expect(Reflect.set(snapshot, 'files', new Map())).toBe(false);
    const forged = Object.freeze({}) as typeof snapshot;
    expect(() => assertImmutablePrivateTreeOverridesStable(snapshot, forged))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SNAPSHOT_INVALID');
    let trapped = false;
    const proxy = new Proxy(forged, { get() { trapped = true; throw new Error('trap'); } });
    expect(() => immutablePrivateTreeOverridePaths(proxy))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SNAPSHOT_INVALID');
    expect(trapped).toBe(false);
  });

  it.each([
    ['compressed size', (entry: OverlayEntry) => { entry.compressedBytes += 1; },
      'HARNESS_IMMUTABLE_RUNTIME_OVERLAY_BLOB_SIZE_MISMATCH'],
    ['compressed hash', (entry: OverlayEntry) => { entry.compressedSha256 = '0'.repeat(64); },
      'HARNESS_IMMUTABLE_RUNTIME_OVERLAY_BLOB_DIGEST_MISMATCH'],
    ['decoded size', (entry: OverlayEntry) => { entry.decodedBytes += 1; },
      'HARNESS_IMMUTABLE_RUNTIME_OVERLAY_DECODED_SIZE_MISMATCH'],
    ['decoded hash', (entry: OverlayEntry) => { entry.decodedSha256 = '0'.repeat(64); },
      'HARNESS_IMMUTABLE_RUNTIME_OVERLAY_DECODED_DIGEST_MISMATCH'],
  ] as const)('rejects a false %s declaration', async (_label, mutate, error) => {
    const fixture = await makeFixture();
    mutate(fixture.manifest.files[0]!);
    await repinManifest(fixture);
    expect(() => materialize(fixture)).toThrow(error);
  });

  it.each([
    ['corrupt', (_bytes: Buffer) => Buffer.from('not-a-gzip-stream')],
    ['truncated', (bytes: Buffer) => bytes.subarray(0, bytes.byteLength - 8)],
  ] as const)('rejects a repinned %s gzip stream', async (_label, mutate) => {
    const fixture = await makeFixture();
    const compressed = mutate(await readFile(fixture.blob));
    await writeFile(fixture.blob, compressed);
    await chmod(fixture.blob, 0o644);
    const entry = fixture.manifest.files[0]!;
    entry.compressedBytes = compressed.byteLength;
    entry.compressedSha256 = sha256(compressed);
    await repinManifest(fixture);
    expect(() => materialize(fixture))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_DECOMPRESSION_FAILED');
  });

  it('bounds a repinned compression bomb by its declared decoded size', async () => {
    const fixture = await makeFixture();
    const compressed = gzipSync(Buffer.alloc(2_048, 'A'), { level: 9 });
    await writeFile(fixture.blob, compressed);
    await chmod(fixture.blob, 0o644);
    Object.assign(fixture.manifest.files[0]!, {
      compressedBytes: compressed.byteLength,
      compressedSha256: sha256(compressed),
      decodedBytes: 32,
      decodedSha256: sha256(Buffer.alloc(32, 'A')),
    });
    await repinManifest(fixture);
    expect(() => materialize(fixture))
      .toThrow('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_DECOMPRESSION_FAILED');
  });
});

async function makeFixture(targetMode = 0o664): Promise<Fixture> {
  const parent = await mkdtemp(join(tmpdir(), 'semantic-fabric-overlay-'));
  roots.push(parent);
  const source = join(parent, 'source');
  const target = join(source, 'src/memory/entry.js');
  const blob = join(parent, 'entry.js.gz');
  const decoded = Buffer.from('historical schema-v2 payload\n');
  const compressed = gzipSync(decoded, { level: 9 });
  await mkdir(join(source, 'src/memory'), { recursive: true });
  await Promise.all([
    chmod(source, 0o755),
    chmod(join(source, 'src'), 0o755),
    chmod(join(source, 'src/memory'), 0o755),
  ]);
  await writeFile(target, 'ambient patched payload\n');
  await chmod(target, targetMode);
  await writeFile(blob, compressed);
  await chmod(blob, 0o644);
  const fixture: Fixture = {
    parent,
    source,
    target,
    blob,
    decoded,
    manifest: {
      schemaVersion: 1,
      files: [{
        targetPath: 'src/memory/entry.js',
        blobPath: 'entry.js.gz',
        compression: 'gzip',
        compressedSha256: sha256(compressed),
        compressedBytes: compressed.byteLength,
        decodedSha256: sha256(decoded),
        decodedBytes: decoded.byteLength,
        executable: false,
      }],
    },
    manifestDigest: '',
    manifestBytes: 0,
  };
  await repinManifest(fixture);
  return fixture;
}

async function repinManifest(fixture: Fixture): Promise<void> {
  const bytes = Buffer.from(`${JSON.stringify(fixture.manifest, null, 2)}\n`);
  await repinRawManifest(fixture, bytes);
}

async function repinRawManifest(fixture: Fixture, bytes: Buffer): Promise<void> {
  await writeFile(join(fixture.parent, 'overlay.json'), bytes);
  await chmod(join(fixture.parent, 'overlay.json'), 0o644);
  fixture.manifestDigest = sha256(bytes);
  fixture.manifestBytes = bytes.byteLength;
}

async function addManifestEntry(
  fixture: Fixture,
  targetPath: string,
  blobPath: string,
): Promise<void> {
  const decoded = Buffer.from(`historical payload for ${targetPath}\n`);
  const compressed = gzipSync(decoded, { level: 9 });
  const blob = join(fixture.parent, blobPath);
  await writeFile(blob, compressed);
  await chmod(blob, 0o644);
  fixture.manifest.files.push({
    targetPath,
    blobPath,
    compression: 'gzip',
    compressedSha256: sha256(compressed),
    compressedBytes: compressed.byteLength,
    decodedSha256: sha256(decoded),
    decodedBytes: decoded.byteLength,
    executable: false,
  });
}

function materialize(fixture: Fixture, withOverride = true) {
  return createImmutablePrivateRuntime({
    parent: fixture.parent,
    prefix: 'overlay-',
    trees: [{
      key: 'fixture',
      sourceRoot: fixture.source,
      relativePath: 'tree',
      maxFiles: 4,
      maxBytes: 1_024,
      ...(withOverride ? {
        overrideManifest: manifestSpec(fixture),
      } : {}),
    }],
  });
}

function manifestSpec(fixture: Fixture) {
  return {
    sourcePath: join(fixture.parent, 'overlay.json'),
    expectedDigest: fixture.manifestDigest,
    expectedBytes: fixture.manifestBytes,
  };
}

function fixtureBounds() {
  return { maxFiles: 4, maxBytes: 1_024 };
}

function treeDigest(decoded: Buffer): string {
  const body = {
    directories: ['src', 'src/memory'],
    files: [{
      relativePath: 'src/memory/entry.js',
      digest: sha256(decoded),
      executable: false,
      size: decoded.byteLength,
    }],
  };
  return sha256(Buffer.from(JSON.stringify(body)));
}

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

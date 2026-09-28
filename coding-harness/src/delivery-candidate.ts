// SPDX-License-Identifier: MIT
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, relative } from 'node:path';
import { hash } from '@metaharness/harness';
import { createImmutablePrivateRuntime } from './immutable-private-runtime.js';
import { type DeliveryContext } from './delivery-context.js';
import { DeliveryHarness } from './delivery-runtime.js';
import { git, mainRoot, outsideDigest, type SourceSnapshot } from './delivery-workspace.js';
import { normalizeWorkspacePath } from './contracts.js';

export interface DeliveryCandidate {
  harness: DeliveryHarness;
  sourceBefore: SourceSnapshot;
  acceptedSource?: { taskId: string; commit: string; inputs: Record<string, string | null> };
  cleanup(): void;
}

/** Exact accepted files, not a successful but unintegrated candidate, release children. */
export function assertAcceptedDeliverySource(harness: DeliveryHarness, parentId: string, paths?: string[]): NonNullable<DeliveryCandidate['acceptedSource']> {
  if (harness.context.kind !== 'main') throw new Error('DELIVERY_ACCEPTED_MAIN_SOURCE_REQUIRED');
  const parent = harness.read(parentId), source = harness.snapshot();
  const review = parent.workflow?.results.at(-1);
  if (parent.status !== 'complete' || !parent.verdict?.pass || !parent.commit
    || !review?.accepted || review.request.stage !== 'review') {
    throw new Error('DELIVERY_ACCEPTED_MAIN_SOURCE_REQUIRED');
  }
  git(harness.root, 'merge-base', '--is-ancestor', parent.commit, harness.context.head());
  const selected = paths ?? parent.task.scope;
  if (!selected.length || selected.some(path => !parent.task.scope.includes(path))) throw new Error('DELIVERY_ACCEPTED_INPUT_SCOPE');
  if (selected.some(path => existsSync(join(harness.root, path)) && lstatSync(join(harness.root, path)).isDirectory())) throw new Error('DELIVERY_EXACT_FILE_SCOPE_REQUIRED');
  for (const path of selected) {
    const entry = git(harness.root, 'ls-tree', '--format=%(objectmode) %(objectname)', parent.commit, '--', path);
    const [mode, blob] = entry.split(' ');
    if ((!entry && source.files[path] !== undefined) || (entry && (source.files[path]?.split(':')[0] !== mode
      || git(harness.root, 'hash-object', '--', path) !== blob))) throw new Error('DELIVERY_ACCEPTED_INPUT_CHANGED');
  }
  return { taskId: parentId, commit: parent.commit, inputs: Object.fromEntries(selected.map(path => [path, source.files[path] ?? null])) };
}

export function createDeliveryCandidate(canonical: DeliveryHarness, input: {
  parentDirectory: string; scope: string[]; acceptedParent?: string; acceptedInputs?: string[];
}): DeliveryCandidate {
  if (canonical.context.kind !== 'main') throw new Error('DELIVERY_CANONICAL_SOURCE_REQUIRED');
  const parentDirectory = realpathSync(input.parentDirectory);
  const relativeParent = relative(realpathSync(canonical.root), parentDirectory);
  if (!relativeParent || (!isAbsolute(relativeParent) && relativeParent !== '..' && !relativeParent.startsWith('../'))) {
    throw new Error('DELIVERY_CANDIDATE_PARENT_NOT_ISOLATED');
  }
  if (input.acceptedInputs && !input.acceptedParent) throw new Error('DELIVERY_ACCEPTED_PARENT_REQUIRED');
  const acceptedSource = input.acceptedParent ? assertAcceptedDeliverySource(canonical, input.acceptedParent, input.acceptedInputs) : undefined;
  const sourceBefore = canonical.snapshot(), baseCommit = canonical.context.head();
  if (acceptedSource) {
    git(canonical.root, 'merge-base', '--is-ancestor', acceptedSource.commit, baseCommit);
    if (Object.entries(acceptedSource.inputs).some(([path, value]) => (sourceBefore.files[path] ?? null) !== value)) throw new Error('DELIVERY_ACCEPTED_INPUT_CHANGED');
  }
  const scope = input.scope.map(path => normalizeWorkspacePath(path, 'candidate scope'));
  const artifactRoots = new Set(['.metaharness', 'coding-harness/dist', 'coding-harness/node_modules', 'target']);
  if (scope.some(path => [...artifactRoots].some(root => path === root || path.startsWith(`${root}/`)))) throw new Error('DELIVERY_ARTIFACT_SCOPE_REFUSED');
  if (Object.keys(sourceBefore.files).some(path => [...artifactRoots].some(root => path === root || path.startsWith(`${root}/`)))) throw new Error('DELIVERY_TRACKED_ARTIFACT_SOURCE_REFUSED');
  if (scope.some(path => existsSync(join(canonical.root, path)) && lstatSync(join(canonical.root, path)).isDirectory())) throw new Error('DELIVERY_EXACT_FILE_SCOPE_REQUIRED');
  const emptyHash = createHash('sha256').update('').digest('hex');
  const runtime = createImmutablePrivateRuntime({ parent: parentDirectory, prefix: 'delivery-candidate-',
    files: Object.entries(sourceBefore.files).filter(([, value]) => value.split(':')[1] !== emptyHash).map(([path, value], index) => ({ key: `source-${index}`,
      sourcePath: join(canonical.root, path), relativePath: path, executable: value.startsWith('100755:'),
      expectedDigest: value.split(':')[1], sourcePolicy: 'same-principal-cooperative-snapshot' })),
    directories: ['.metaharness/delivery'],
  });
  try {
    const unseal = (directory: string): void => {
      chmodSync(directory, 0o700);
      for (const entry of readdirSync(directory, { withFileTypes: true })) if (entry.isDirectory()) unseal(join(directory, entry.name));
    };
    unseal(runtime.root);
    for (const [path, value] of Object.entries(sourceBefore.files)) if (value.split(':')[1] === emptyHash) {
      mkdirSync(dirname(join(runtime.root, path)), { recursive: true, mode: 0o700 });
      writeFileSync(join(runtime.root, path), '', { flag: 'wx', mode: value.startsWith('100755:') ? 0o500 : 0o400 });
    }
    for (const [path, value] of Object.entries(sourceBefore.files)) if (scope.some(root => path === root || path.startsWith(`${root}/`))) {
      chmodSync(join(runtime.root, path), value.startsWith('100755:') ? 0o700 : 0o600);
    }
    const snapshot = (): SourceSnapshot => {
      const files: Record<string, string> = {};
      const walk = (path: string): void => {
        if (artifactRoots.has(path)) return;
        const absolute = join(runtime.root, path), stat = lstatSync(absolute);
        if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile()) || (stat.isFile() && stat.nlink !== 1)) throw new Error('DELIVERY_NONREGULAR_CANDIDATE');
        if (stat.isDirectory()) for (const child of readdirSync(absolute).sort()) walk(path ? `${path}/${child}` : child);
        else files[path] = `${stat.mode & 0o111 ? '100755' : '100644'}:${createHash('sha256').update(readFileSync(absolute)).digest('hex')}`;
      };
      walk('');
      const ordered = Object.fromEntries(Object.entries(files).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0));
      return { files: ordered, digest: hash(ordered) };
    };
    const assert = (): void => {
      mainRoot(canonical.root);
      if (canonical.context.head() !== baseCommit || canonical.snapshot().digest !== sourceBefore.digest
        || existsSync(join(runtime.root, '.git'))) throw new Error('DELIVERY_CANDIDATE_SOURCE_DRIFT');
      if (outsideDigest(snapshot(), scope) !== outsideDigest(sourceBefore, scope)) throw new Error('DELIVERY_OUT_OF_SCOPE_CHANGE');
    };
    const context: DeliveryContext = { kind: 'candidate', root: runtime.root, directory: join(runtime.root, '.metaharness/delivery'),
      assert, head: () => baseCommit, snapshot,
      dirty: paths => { const now = snapshot(); return paths.filter(path => now.files[path] !== sourceBefore.files[path]); },
    };
    if (snapshot().digest !== sourceBefore.digest) throw new Error('DELIVERY_CANDIDATE_COPY_MISMATCH');
    return { harness: new DeliveryHarness(runtime.root, context), sourceBefore, ...(acceptedSource ? { acceptedSource } : {}), cleanup: runtime.cleanup };
  } catch (error) { runtime.cleanup(); throw error; }
}

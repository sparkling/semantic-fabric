// SPDX-License-Identifier: MIT
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { chmodSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, relative } from 'node:path';
import { hash } from '@metaharness/harness';
import { createImmutablePrivateRuntime } from './immutable-private-runtime.js';
import { type DeliveryContext } from './delivery-context.js';
import { DeliveryHarness } from './delivery-runtime.js';
import { atomicJson, git, isSnapshotSourcePath, outsideDigest, readJson, type SourceSnapshot } from './delivery-workspace.js';
import { normalizeWorkspacePath } from './contracts.js';
import { liveDeliveryRuntimeInputs } from './delivery-lineage.js';

export interface DeliveryCandidate {
  harness: DeliveryHarness;
  sourceBefore: SourceSnapshot;
  acceptedSource?: { taskId: string; commit: string; inputs: Record<string, string | null> };
  cleanup(): void;
}

/** Explicit read inputs are context, never additional mutation authority. */
export function deliveryReadPaths(harness: DeliveryHarness, scope: string[], declared: string[] = []): string[] {
  const accepted = harness.context.kind === 'candidate' ? (readJson(join(harness.context.canonicalRoot!,
    '.metaharness/delivery', `candidate-${hash(harness.root)}.json`)) as { acceptedSource?: { inputs: Record<string, string> } }).acceptedSource : undefined;
  return [...new Set([...scope, ...declared, ...Object.keys(accepted?.inputs ?? {})])];
}

const artifactRoots = new Set(['.metaharness', 'coding-harness/dist', 'coding-harness/node_modules', 'target']);

/** Read raw Git blobs, never the index, filters or another author's working bytes. */
function committedSource(root: string, commit: string) {
  const files: Record<string, string> = {}, blobs = new Map<string, Buffer>();
  const entries = git(root, 'ls-tree', '-r', '-z', '--full-tree', commit).split('\0').filter(Boolean);
  for (const entry of entries) {
    const separator = entry.indexOf('\t'), path = entry.slice(separator + 1);
    if (!isSnapshotSourcePath(path)) continue;
    normalizeWorkspacePath(path, 'committed source');
    const metadata = /^(100644|100755) blob ([a-f0-9]{40}|[a-f0-9]{64})$/.exec(entry.slice(0, separator));
    if (!metadata) throw new Error(`DELIVERY_NONREGULAR_SOURCE:${path}`);
    if ([...artifactRoots].some(root => path === root || path.startsWith(`${root}/`))) {
      throw new Error('DELIVERY_TRACKED_ARTIFACT_SOURCE_REFUSED');
    }
    const bytes = execFileSync('git', ['-C', root, 'cat-file', 'blob', metadata[2]], { maxBuffer: 32 * 1024 * 1024 });
    files[path] = `${metadata[1]}:${createHash('sha256').update(bytes).digest('hex')}`;
    blobs.set(path, bytes);
  }
  const ordered = Object.fromEntries(Object.entries(files).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0));
  return { snapshot: { files: ordered, digest: hash(ordered) }, blobs };
}

/** Integration owns working bytes; independent candidates use its last accepted commit. */
export function assertCandidateAdmission(canonical: DeliveryHarness, tasks: readonly { id: string; mutationPaths: string[]; resources: string[]; readPaths?: string[] }[]): void {
  const active = canonical.inspect().active as { id: string } | null;
  if (!active) return;
  const run = canonical.read(active.id);
  if (!run.integration || run.integration.phase !== 'prepared' || canonical.context.head() !== run.baseCommit) {
    throw new Error('DELIVERY_WRITER_ALREADY_CLAIMED');
  }
  const overlaps = (a: string, b: string) => a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
  if (tasks.some(task => task.readPaths === undefined
    || task.readPaths.some(path => run.task.scope.some(other => overlaps(path, other))))) {
    throw new Error('DELIVERY_INTEGRATION_ACTIVE_DEPENDENCY');
  }
  if (liveDeliveryRuntimeInputs(run.task.scope).length || tasks.some(task => task.id === run.task.id
    || task.mutationPaths.some(path => run.task.scope.some(other => overlaps(path, other)))
    || task.resources.some(resource => run.integration!.resources.includes(resource)))) {
    throw new Error('DELIVERY_POOL_RESOURCE_CONFLICT');
  }
}

export function candidateContext(root: string, canonical: Pick<DeliveryHarness, 'root' | 'directory'>, sourceBefore: SourceSnapshot, baseCommit: string, scope: string[], _integrationInspection = false): DeliveryContext {
  const snapshot = (): SourceSnapshot => {
    const files: Record<string, string> = {};
    const walk = (path: string): void => {
      if (artifactRoots.has(path)) return;
      const absolute = join(root, path), stat = lstatSync(absolute);
      if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile()) || (stat.isFile() && stat.nlink !== 1)) throw new Error('DELIVERY_NONREGULAR_CANDIDATE');
      if (stat.isDirectory()) for (const child of readdirSync(absolute).sort()) walk(path ? `${path}/${child}` : child);
      else files[path] = `${stat.mode & 0o111 ? '100755' : '100644'}:${createHash('sha256').update(readFileSync(absolute)).digest('hex')}`;
    };
    walk('');
    const ordered = Object.fromEntries(Object.entries(files).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0));
    return { files: ordered, digest: hash(ordered) };
  };
  const assert = (): void => {
    if (existsSync(join(root, '.git'))) throw new Error('DELIVERY_CANDIDATE_SOURCE_DRIFT');
    // Private work stays bound to its immutable snapshot; canonical drift is checked at integration.
    if (outsideDigest(snapshot(), scope) !== outsideDigest(sourceBefore, scope)) throw new Error('DELIVERY_OUT_OF_SCOPE_CHANGE');
  };
  return { kind: 'candidate', root, directory: join(root, '.metaharness/delivery'), apiDirectory: join(canonical.directory, 'api'), canonicalRoot: canonical.root, assert,
    head: () => baseCommit, snapshot,
    dirty: paths => { const now = snapshot(); return paths.filter(path => now.files[path] !== sourceBefore.files[path]); } };
}

/** Candidate pointer is not authority: scope and baseline come from canonical custody. */
export function reopenDeliveryCandidate(root: string): { harness: DeliveryHarness } {
  root = realpathSync(root);
  const pointer = readJson(join(root, '.metaharness/delivery/candidate.json')) as { canonicalRoot: string };
  const canonicalRoot = realpathSync(pointer.canonicalRoot);
  const canonical = { root: canonicalRoot, directory: join(canonicalRoot, '.metaharness/delivery') };
  const record = readJson(join(canonical.directory, `candidate-${hash(root)}.json`)) as {
    root: string; sourceBefore: SourceSnapshot; baseCommit: string; scope: string[];
  };
  if (record.root !== root || hash(record.sourceBefore.files) !== record.sourceBefore.digest) throw new Error('DELIVERY_CANDIDATE_IDENTITY');
  const context = candidateContext(root, canonical, record.sourceBefore, record.baseCommit, record.scope);
  context.assert();
  return { harness: new DeliveryHarness(root, context) };
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
  parentDirectory: string; scope: string[]; acceptedParent?: string; acceptedInputs?: string[]; readPaths?: string[];
  resources?: string[]; sourceCommit?: string;
}): DeliveryCandidate {
  if (canonical.context.kind !== 'main') throw new Error('DELIVERY_CANONICAL_SOURCE_REQUIRED');
  canonical.context.assert();
  const baseCommit = canonical.context.head();
  if (input.sourceCommit !== undefined && (typeof input.sourceCommit !== 'string'
    || !/^[a-f0-9]{40}$/.test(input.sourceCommit) || input.sourceCommit !== baseCommit)) {
    throw new Error('DELIVERY_CANDIDATE_SOURCE_COMMIT_CHANGED');
  }
  const parentDirectory = realpathSync(input.parentDirectory);
  const relativeParent = relative(realpathSync(canonical.root), parentDirectory);
  if (!relativeParent || (!isAbsolute(relativeParent) && relativeParent !== '..' && !relativeParent.startsWith('../'))) {
    throw new Error('DELIVERY_CANDIDATE_PARENT_NOT_ISOLATED');
  }
  if (input.acceptedInputs && !input.acceptedParent) throw new Error('DELIVERY_ACCEPTED_PARENT_REQUIRED');
  const acceptedSource = input.acceptedParent ? assertAcceptedDeliverySource(canonical, input.acceptedParent, input.acceptedInputs) : undefined;
  assertCandidateAdmission(canonical, [{ id: '', mutationPaths: input.scope, resources: input.resources ?? [], readPaths: input.readPaths }]);
  const active = canonical.inspect().active as { id: string } | null;
  const integration = active ? canonical.read(active.id).integration : undefined;
  const committed = input.sourceCommit === undefined ? undefined : committedSource(canonical.root, baseCommit);
  const sourceBefore = committed?.snapshot ?? integration?.sourceBefore ?? canonical.snapshot();
  if (acceptedSource) {
    git(canonical.root, 'merge-base', '--is-ancestor', acceptedSource.commit, baseCommit);
    if (Object.entries(acceptedSource.inputs).some(([path, value]) => (sourceBefore.files[path] ?? null) !== value)) throw new Error('DELIVERY_ACCEPTED_INPUT_CHANGED');
  }
  const scope = input.scope.map(path => normalizeWorkspacePath(path, 'candidate scope'));
  if (scope.some(path => /(^|\/)(\.git|\.env(?:\..*)?|\.metaharness)(\/|$)/.test(path))) throw new Error('DELIVERY_INVALID_SCOPE');
  if (scope.some(path => [...artifactRoots].some(root => path === root || path.startsWith(`${root}/`)))) throw new Error('DELIVERY_ARTIFACT_SCOPE_REFUSED');
  if (Object.keys(sourceBefore.files).some(path => [...artifactRoots].some(root => path === root || path.startsWith(`${root}/`)))) throw new Error('DELIVERY_TRACKED_ARTIFACT_SOURCE_REFUSED');
  if (scope.some(path => existsSync(join(canonical.root, path)) && lstatSync(join(canonical.root, path)).isDirectory())) throw new Error('DELIVERY_EXACT_FILE_SCOPE_REQUIRED');
  const emptyHash = createHash('sha256').update('').digest('hex');
  // Explicit commit mode copies only blobs; legacy integration restores its changed files.
  const restored = committed || integration ? mkdtempSync(join(parentDirectory, 'accepted-source-')) : undefined;
  let runtime: ReturnType<typeof createImmutablePrivateRuntime>;
  try {
  const working = integration && !committed ? canonical.snapshot() : sourceBefore;
  const sources = new Map<string, string>();
  for (const [path, value] of Object.entries(sourceBefore.files)) {
    let sourcePath = join(canonical.root, path);
    if (restored && (committed || working.files[path] !== value)) {
      normalizeWorkspacePath(path, 'accepted source');
      sourcePath = join(restored, path);
      mkdirSync(dirname(sourcePath), { recursive: true, mode: 0o700 });
      const bytes = committed?.blobs.get(path) ?? execFileSync('git', ['-C', canonical.root, 'cat-file', 'blob', `${baseCommit}:${path}`], { maxBuffer: 32 * 1024 * 1024 });
      if (createHash('sha256').update(bytes).digest('hex') !== value.split(':')[1]) throw new Error('DELIVERY_CANDIDATE_COPY_MISMATCH');
      writeFileSync(sourcePath, bytes, { flag: 'wx', mode: value.startsWith('100755:') ? 0o700 : 0o600 });
    }
    sources.set(path, sourcePath);
  }
  runtime = createImmutablePrivateRuntime({ parent: parentDirectory, prefix: 'delivery-candidate-',
    files: Object.entries(sourceBefore.files).filter(([, value]) => value.split(':')[1] !== emptyHash).map(([path, value], index) => ({ key: `source-${index}`,
      sourcePath: sources.get(path)!, relativePath: path, executable: value.startsWith('100755:'),
      expectedDigest: value.split(':')[1], sourcePolicy: 'same-principal-cooperative-snapshot' })),
    directories: ['.metaharness/delivery'],
  });
  } finally { if (restored) rmSync(restored, { recursive: true, force: true }); }
  try {
    const unseal = (directory: string): void => {
      chmodSync(directory, 0o700);
      for (const entry of readdirSync(directory, { withFileTypes: true })) if (entry.isDirectory()) unseal(join(directory, entry.name));
    };
    unseal(runtime.root);
    // Empty private parents enable new scoped files without changing the frozen source snapshot.
    for (const path of scope) mkdirSync(dirname(join(runtime.root, path)), { recursive: true, mode: 0o700 });
    for (const [path, value] of Object.entries(sourceBefore.files)) if (value.split(':')[1] === emptyHash) {
      mkdirSync(dirname(join(runtime.root, path)), { recursive: true, mode: 0o700 });
      writeFileSync(join(runtime.root, path), '', { flag: 'wx', mode: value.startsWith('100755:') ? 0o500 : 0o400 });
    }
    for (const [path, value] of Object.entries(sourceBefore.files)) if (scope.some(root => path === root || path.startsWith(`${root}/`))) {
      chmodSync(join(runtime.root, path), value.startsWith('100755:') ? 0o700 : 0o600);
    }
    const context = candidateContext(runtime.root, canonical, sourceBefore, baseCommit, scope);
    if (context.snapshot().digest !== sourceBefore.digest) throw new Error('DELIVERY_CANDIDATE_COPY_MISMATCH');
    canonical.context.assert();
    if (canonical.context.head() !== baseCommit || (!committed && !integration && canonical.snapshot().digest !== sourceBefore.digest)) throw new Error('DELIVERY_CANDIDATE_SOURCE_DRIFT');
    const readPaths = [...new Set([...(input.readPaths ?? Object.keys(sourceBefore.files)), ...Object.keys(acceptedSource?.inputs ?? {})])];
    for (const path of readPaths) normalizeWorkspacePath(path, 'candidate read path');
    atomicJson(join(canonical.directory, `candidate-${hash(runtime.root)}.json`), { root: runtime.root, sourceBefore, baseCommit, scope,
      readPaths, declaredReadPaths: input.readPaths ?? null, resources: input.resources ?? [], acceptedSource: acceptedSource ?? null,
      cleanBase: !!committed || !!integration || git(canonical.root, 'status', '--porcelain') === '' });
    atomicJson(join(context.directory, 'candidate.json'), { canonicalRoot: canonical.root });
    return { harness: new DeliveryHarness(runtime.root, context), sourceBefore, ...(acceptedSource ? { acceptedSource } : {}), cleanup: runtime.cleanup };
  } catch (error) { runtime.cleanup(); throw error; }
}

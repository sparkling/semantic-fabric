// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { lstatSync, readFileSync, realpathSync } from 'node:fs';
import { isAbsolute, relative, resolve } from 'node:path';
import { runGitCommand, runGitCommandBytes } from './git-process.js';

const INVENTORY_PATH = 'tests/postgresql/observation-qualification-inputs-v1.tsv';
const CATEGORIES = Object.freeze([
  'adr', 'profile', 'queries', 'tests', 'fixture', 'runner', 'protocol',
] as const);
type Category = (typeof CATEGORIES)[number];
const MAX_INVENTORY_BYTES = 256 * 1024;
const MAX_SOURCE_BYTES = 8 * 1024 * 1024;
const MAX_INVENTORY_PATHS = 512;
const GIT_OBJECT = /^[a-f0-9]{40}$/;

export interface QualificationImageIdentity {
  readonly repository: 'postgres';
  readonly manifestDigest: string;
  readonly configDigest: string;
  readonly platform: 'linux/amd64';
  readonly inspectSha256: string;
}

export interface QualificationBuilderImageIdentity {
  readonly repository: 'rust';
  readonly manifestDigest: string;
  readonly configDigest: string;
  readonly platform: 'linux/amd64';
  readonly inspectSha256: string;
}

export interface QualificationProvenance {
  readonly source: {
    readonly commit: string;
    readonly tree: string;
    readonly cargoLockSha256: string;
    readonly probeArtifactSha256: string;
    readonly probeStdoutSha256: string;
  };
  readonly inputs: {
    readonly adrSha256: string;
    readonly profileSha256: string;
    readonly queriesSha256: string;
    readonly testsSha256: string;
    readonly fixtureSha256: string;
    readonly runnerSha256: string;
    readonly protocolSha256: string;
  };
  readonly toolchain: {
    readonly rustcVvSha256: string;
    readonly cargoVersionSha256: string;
    readonly targetTriple: 'x86_64-unknown-linux-gnu';
    readonly builderImage: QualificationBuilderImageIdentity;
  };
  readonly image: QualificationImageIdentity;
}

export async function collectQualificationProvenance(input: Readonly<{
  repositoryRoot: string;
  artifactPath: string;
  rustcVv: Buffer;
  cargoVersion: Buffer;
  builderImage: QualificationBuilderImageIdentity;
  probeStdoutSha256: string;
  image: QualificationImageIdentity;
}>): Promise<QualificationProvenance> {
  const root = canonicalRoot(input.repositoryRoot);
  await requireCleanTree(root);
  const [commit, tree] = await Promise.all([
    gitLine(root, ['rev-parse', '--verify', 'HEAD^{commit}']),
    gitLine(root, ['rev-parse', '--verify', 'HEAD^{tree}']),
  ]);
  if (!GIT_OBJECT.test(commit) || !GIT_OBJECT.test(tree)) {
    throw new Error('POSTGRES_QUALIFICATION_GIT_IDENTITY_INVALID');
  }
  const entries = await readInventoryAtCommit(root, commit);
  const grouped = groupedDigests(entries);
  const artifact = readRegularFile(root, input.artifactPath, 128 * 1024 * 1024);
  const cargoLock = await gitBlob(root, commit, 'Cargo.lock');
  validateToolchain(input.rustcVv, input.cargoVersion);
  await requireCleanTree(root);
  const [commitAfter, treeAfter] = await Promise.all([
    gitLine(root, ['rev-parse', '--verify', 'HEAD^{commit}']),
    gitLine(root, ['rev-parse', '--verify', 'HEAD^{tree}']),
  ]);
  if (commitAfter !== commit || treeAfter !== tree) {
    throw new Error('POSTGRES_QUALIFICATION_SOURCE_CHANGED_DURING_CAPTURE');
  }
  return Object.freeze({
    source: Object.freeze({
      commit,
      tree,
      cargoLockSha256: sha256(cargoLock),
      probeArtifactSha256: sha256(artifact),
      probeStdoutSha256: digest(input.probeStdoutSha256, 'probe stdout'),
    }),
    inputs: Object.freeze({
      adrSha256: grouped.adr,
      profileSha256: grouped.profile,
      queriesSha256: grouped.queries,
      testsSha256: grouped.tests,
      fixtureSha256: grouped.fixture,
      runnerSha256: grouped.runner,
      protocolSha256: grouped.protocol,
    }),
    toolchain: Object.freeze({
      rustcVvSha256: sha256(input.rustcVv),
      cargoVersionSha256: sha256(input.cargoVersion),
      targetTriple: 'x86_64-unknown-linux-gnu',
      builderImage: Object.freeze({ ...input.builderImage }),
    }),
    image: Object.freeze({ ...input.image }),
  });
}

export async function requireCleanQualificationSource(repositoryRoot: string): Promise<void> {
  await requireCleanTree(canonicalRoot(repositoryRoot));
}

/** Verify the source/input portion against immutable Git objects, not the checkout. */
export async function verifyQualificationSourceProvenance(
  repositoryRoot: string,
  provenance: QualificationProvenance,
): Promise<void> {
  const root = canonicalRoot(repositoryRoot);
  await requireCleanTree(root);
  const commit = gitObject(provenance.source.commit, 'source commit');
  const expectedTree = gitObject(provenance.source.tree, 'source tree');
  const currentCommit = await gitLine(root, ['rev-parse', '--verify', 'HEAD^{commit}']);
  if (!GIT_OBJECT.test(currentCommit)) {
    throw new Error('POSTGRES_QUALIFICATION_CURRENT_GIT_IDENTITY_INVALID');
  }
  await requireAncestor(root, commit, currentCommit);
  const tree = await gitLine(root, ['rev-parse', '--verify', `${commit}^{tree}`]);
  if (tree !== expectedTree) throw new Error('POSTGRES_QUALIFICATION_SOURCE_TREE_MISMATCH');
  const [sourceEntries, currentEntries] = await Promise.all([
    readInventoryAtCommit(root, commit),
    readInventoryAtCommit(root, currentCommit),
  ]);
  const expectedInputs: Record<Category, string> = {
    adr: provenance.inputs.adrSha256,
    profile: provenance.inputs.profileSha256,
    queries: provenance.inputs.queriesSha256,
    tests: provenance.inputs.testsSha256,
    fixture: provenance.inputs.fixtureSha256,
    runner: provenance.inputs.runnerSha256,
    protocol: provenance.inputs.protocolSha256,
  };
  assertInputDigests(groupedDigests(sourceEntries), expectedInputs, 'SOURCE');
  assertInputDigests(groupedDigests(currentEntries), expectedInputs, 'CURRENT');
  const [sourceCargoLock, currentCargoLock] = await Promise.all([
    gitBlob(root, commit, 'Cargo.lock'),
    gitBlob(root, currentCommit, 'Cargo.lock'),
  ]);
  const expectedCargoLock = digest(provenance.source.cargoLockSha256, 'Cargo lock');
  if (sha256(sourceCargoLock) !== expectedCargoLock || sha256(currentCargoLock) !== expectedCargoLock) {
    throw new Error('POSTGRES_QUALIFICATION_CARGO_LOCK_MISMATCH');
  }
  await requireCleanTree(root);
  if (await gitLine(root, ['rev-parse', '--verify', 'HEAD^{commit}']) !== currentCommit) {
    throw new Error('POSTGRES_QUALIFICATION_CURRENT_SOURCE_CHANGED_DURING_VERIFICATION');
  }
}

export interface QualificationInventoryRecord {
  readonly category: Category;
  readonly path: string;
}

interface InventoryEntry extends QualificationInventoryRecord {
  readonly bytes: Buffer;
}

async function readInventoryAtCommit(
  root: string,
  commit: string,
): Promise<readonly InventoryEntry[]> {
  const source = await gitBlob(root, commit, INVENTORY_PATH, MAX_INVENTORY_BYTES);
  const records = parsePostgresQualificationInventory(source);
  const blobs = await Promise.all(records.map((entry) => gitBlob(root, commit, entry.path)));
  return Object.freeze(records.map((entry, index) => Object.freeze({
    category: entry.category,
    path: entry.path,
    bytes: blobs[index],
  })));
}

export function parsePostgresQualificationInventory(
  source: Buffer,
): readonly QualificationInventoryRecord[] {
  const text = source.toString('utf8');
  if (!source.equals(Buffer.from(text, 'utf8')) || text.includes('\r') || text.includes('\0')
    || !text.endsWith('\n')) {
    throw new TypeError('POSTGRES_QUALIFICATION_INVENTORY_ENCODING_INVALID');
  }
  const lines = text.slice(0, -1).split('\n');
  if (lines.shift() !== 'category\tpath' || lines.length < CATEGORIES.length
    || lines.length > MAX_INVENTORY_PATHS) {
    throw new TypeError('POSTGRES_QUALIFICATION_INVENTORY_SHAPE_INVALID');
  }
  const seen = new Set<string>();
  const records = lines.map((line) => {
    const fields = line.split('\t');
    if (fields.length !== 2 || !CATEGORIES.includes(fields[0] as Category)) {
      throw new TypeError('POSTGRES_QUALIFICATION_INVENTORY_ROW_INVALID');
    }
    const category = fields[0] as Category;
    const path = checkedPath(fields[1]);
    const key = `${category}\t${path}`;
    if (seen.has(path)) throw new TypeError('POSTGRES_QUALIFICATION_INVENTORY_DUPLICATE_PATH');
    seen.add(path);
    return { category, path, key };
  });
  const sorted = [...records].sort((left, right) =>
    left.key < right.key ? -1 : left.key > right.key ? 1 : 0);
  if (records.some((entry, index) => entry.key !== sorted[index].key)
    || CATEGORIES.some((category) => !records.some((entry) => entry.category === category))) {
    throw new TypeError('POSTGRES_QUALIFICATION_INVENTORY_ORDER_INVALID');
  }
  if (!seen.has(INVENTORY_PATH)) {
    throw new TypeError('POSTGRES_QUALIFICATION_INVENTORY_SELF_BINDING_MISSING');
  }
  return Object.freeze(records.map(({ category, path }) => Object.freeze({ category, path })));
}

function digestEntries(category: Category, entries: readonly InventoryEntry[]): string {
  const body = entries.map((entry) => Object.freeze({
    path: entry.path,
    bytes: entry.bytes.length,
    sha256: sha256(entry.bytes),
  }));
  return sha256(Buffer.from(JSON.stringify({ category, entries: body }), 'utf8'));
}

function groupedDigests(entries: readonly InventoryEntry[]): Record<Category, string> {
  return Object.fromEntries(CATEGORIES.map((category) => [
    category, digestEntries(category, entries.filter((entry) => entry.category === category)),
  ])) as Record<Category, string>;
}

function assertInputDigests(
  actual: Record<Category, string>,
  expected: Record<Category, string>,
  scope: 'SOURCE' | 'CURRENT',
): void {
  for (const category of CATEGORIES) {
    if (digest(expected[category], `${category} inputs`) !== actual[category]) {
      throw new Error(
        `POSTGRES_QUALIFICATION_${scope}_${category.toUpperCase()}_INPUTS_MISMATCH`,
      );
    }
  }
}

async function requireAncestor(root: string, ancestor: string, descendant: string): Promise<void> {
  const result = await runGitCommand(root, [
    'merge-base', '--is-ancestor', gitObject(ancestor, 'source commit'),
    gitObject(descendant, 'current commit'),
  ], { maxOutputBytes: 1_024 });
  if (result.exitCode !== 0 || result.stdout !== '' || result.stderr !== '') {
    throw new Error('POSTGRES_QUALIFICATION_SOURCE_NOT_CURRENT_ANCESTOR');
  }
}

async function requireCleanTree(root: string): Promise<void> {
  const result = await runGitCommandBytes(root, [
    'status', '--porcelain=v1', '-z', '--untracked-files=all',
  ], { maxOutputBytes: 2 * 1024 * 1024 });
  if (result.exitCode !== 0 || result.stderr !== '' || result.stdout.length !== 0) {
    throw new Error('POSTGRES_QUALIFICATION_SOURCE_TREE_NOT_CLEAN');
  }
}

async function gitLine(root: string, args: readonly string[]): Promise<string> {
  const result = await runGitCommand(root, args, { maxOutputBytes: 4_096 });
  if (result.exitCode !== 0 || result.stderr !== '' || !result.stdout.endsWith('\n')) {
    throw new Error('POSTGRES_QUALIFICATION_GIT_COMMAND_FAILED');
  }
  return result.stdout.slice(0, -1);
}

async function gitBlob(
  root: string,
  commit: string,
  path: string,
  maximum = MAX_SOURCE_BYTES,
): Promise<Buffer> {
  const result = await runGitCommandBytes(
    root, ['cat-file', 'blob', `${gitObject(commit, 'source commit')}:${checkedPath(path)}`],
    { maxOutputBytes: maximum },
  );
  if (result.exitCode !== 0 || result.stderr !== '' || result.stdout.length < 1) {
    throw new Error('POSTGRES_QUALIFICATION_GIT_BLOB_INVALID');
  }
  return result.stdout;
}

function readRegularFile(root: string, path: string, maximum: number): Buffer {
  const absolute = resolve(root, checkedPath(path));
  const rel = relative(root, absolute);
  if (rel.startsWith('..') || isAbsolute(rel)) {
    throw new TypeError('POSTGRES_QUALIFICATION_PATH_ESCAPE');
  }
  const stat = lstatSync(absolute);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1
    || stat.size < 1 || stat.size > maximum
    || realpathSync(absolute) !== absolute) {
    throw new TypeError('POSTGRES_QUALIFICATION_SOURCE_FILE_INVALID');
  }
  return readFileSync(absolute);
}

function checkedPath(path: string): string {
  if (typeof path !== 'string' || path === '' || path.startsWith('/') || path.includes('\\')
    || path.includes('\0') || path.normalize('NFC') !== path
    || path.split('/').some((part) => part === '' || part === '.' || part === '..')) {
    throw new TypeError('POSTGRES_QUALIFICATION_PATH_INVALID');
  }
  return path;
}

function canonicalRoot(root: string): string {
  if (!isAbsolute(root) || resolve(root) !== root || realpathSync(root) !== root
    || !lstatSync(root).isDirectory()) {
    throw new TypeError('POSTGRES_QUALIFICATION_REPOSITORY_ROOT_INVALID');
  }
  return root;
}

function validateToolchain(rustc: Buffer, cargo: Buffer): void {
  const rustcText = rustc.toString('utf8');
  const cargoText = cargo.toString('utf8');
  if (!rustc.equals(Buffer.from(rustcText)) || !cargo.equals(Buffer.from(cargoText))
    || !/^rustc 1\.96\.0 \([a-f0-9]+ 2026-05-25\)\n/.test(rustcText)
    || !rustcText.includes('\ncommit-date: 2026-05-25\n')
    || !rustcText.includes('\nhost: x86_64-unknown-linux-gnu\n')
    || !rustcText.includes('\nrelease: 1.96.0\n')
    || !/^cargo 1\.96\.0 \([a-f0-9]+ 2026-05-25\)\n$/.test(cargoText)) {
    throw new Error('POSTGRES_QUALIFICATION_TOOLCHAIN_INVALID');
  }
}

function sha256(value: Buffer): string {
  return createHash('sha256').update(value).digest('hex');
}

function digest(value: string, label: string): string {
  if (!/^[a-f0-9]{64}$/.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`POSTGRES_QUALIFICATION_${label.toUpperCase().replaceAll(' ', '_')}_INVALID`);
  }
  return value;
}

function gitObject(value: string, label: string): string {
  if (!GIT_OBJECT.test(value)) {
    throw new TypeError(`POSTGRES_QUALIFICATION_${label.toUpperCase().replaceAll(' ', '_')}_INVALID`);
  }
  return value;
}

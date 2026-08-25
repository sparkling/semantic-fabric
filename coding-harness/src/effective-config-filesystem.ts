// SPDX-License-Identifier: MIT

import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  opendirSync,
  readSync,
  realpathSync,
  statSync,
  type BigIntStats,
} from 'node:fs';
import { isAbsolute, relative, resolve, sep } from 'node:path';
import { deepFreeze } from './contracts.js';
import {
  auditEffectiveConfiguration,
  type ConfigurationRepositorySnapshot,
  type ConfigurationScope,
  type EffectiveConfigurationAudit,
  type SurfaceKind,
  type SurfaceProvenance,
} from './effective-config.js';
import {
  deriveUpstreamDiagnostics,
  type CapturedUpstreamDiagnostic,
} from './effective-config-diagnostics.js';
import {
  assertConfigurationGitSnapshot,
  captureConfigurationGitSnapshot,
  configurationFileProvenance,
} from './effective-config-git.js';

const MAX_SURFACES = 1_000;
const MAX_SURFACE_BYTES = 1_000_000;
const MAX_TOTAL_BYTES = 20_000_000;
const MAX_DIRECTORY_ENTRIES = 10_000;
const MAX_DIRECTORY_DEPTH = 12;

interface CollectedSurface {
  readonly path: string;
  readonly kind: SurfaceKind;
  readonly provenance: SurfaceProvenance;
  readonly content: string;
}

interface ScopeCollection {
  readonly target: ConfigurationScope;
  inventoryComplete: boolean;
  readonly surfaces: Map<string, CollectedSurface>;
}

interface CachedFile {
  readonly content: string;
  readonly bytes: Buffer;
  readonly provenance: SurfaceProvenance;
  readonly trustworthy: boolean;
}

interface CollectionBudget {
  surfaces: number;
  bytes: number;
  entries: number;
  exhausted: boolean;
  readonly cache: Map<string, CachedFile>;
}

export interface FilesystemConfigurationAuditOptions {
  readonly repositoryRoot: string;
  readonly capturedDiagnostics: readonly CapturedUpstreamDiagnostic[];
}

export function auditEffectiveConfigurationFromFilesystem(
  options: FilesystemConfigurationAuditOptions,
): EffectiveConfigurationAudit {
  const collected = collectBundle(options);
  const audited = auditEffectiveConfiguration(collected.input);
  return deepFreeze({ ...audited, repositorySnapshot: collected.snapshot });
}

export function collectEffectiveConfigurationAuditInput(
  options: FilesystemConfigurationAuditOptions,
): Readonly<Record<string, unknown>> {
  return collectBundle(options).input;
}

function collectBundle(options: FilesystemConfigurationAuditOptions): Readonly<{
  input: Readonly<Record<string, unknown>>;
  snapshot: ConfigurationRepositorySnapshot;
}> {
  const repositoryRoot = canonicalDirectory(options.repositoryRoot);
  const snapshot = captureConfigurationGitSnapshot(repositoryRoot);
  const diagnostics = deriveUpstreamDiagnostics({
    repositoryRoot,
    snapshotDigest: snapshot.digest,
    captures: options.capturedDiagnostics,
  });
  const budget: CollectionBudget = {
    surfaces: 0, bytes: 0, entries: 0, exhausted: false, cache: new Map(),
  };
  const scopes = [
    collectScope(repositoryRoot, 'repository', snapshot, budget),
    collectScope(repositoryRoot, 'coding-harness', snapshot, budget),
  ];
  assertConfigurationGitSnapshot(repositoryRoot, snapshot);
  const input = Object.freeze({
    schemaVersion: 1,
    scopes: scopes.map((scope) => Object.freeze({
      target: scope.target,
      inventoryComplete: scope.inventoryComplete && !budget.exhausted,
      surfaces: [...scope.surfaces.values()].sort((left, right) =>
        left.path.localeCompare(right.path) || left.kind.localeCompare(right.kind)),
      upstreamDiagnostics: diagnostics.get(scope.target),
    })),
  });
  return Object.freeze({ input, snapshot });
}

function collectScope(
  repositoryRoot: string,
  target: ConfigurationScope,
  snapshot: ConfigurationRepositorySnapshot,
  budget: CollectionBudget,
): ScopeCollection {
  const collection: ScopeCollection = { target, inventoryComplete: true, surfaces: new Map() };
  const prefix = target === 'repository' ? '' : 'coding-harness/';
  for (const [path, kind] of fixedSurfaces(prefix)) {
    addIfPresent(repositoryRoot, collection, path, kind, snapshot, budget, false);
  }
  for (const [path, kind] of directorySurfaces(prefix)) {
    walkIfPresent(repositoryRoot, collection, path, kind, snapshot, budget);
  }
  for (const path of discoverLocalTargets(collection.surfaces.values(), target)) {
    if (path === null) {
      collection.inventoryComplete = false;
    } else {
      addIfPresent(repositoryRoot, collection, path, 'executable-target', snapshot, budget, true);
    }
  }
  return collection;
}

function fixedSurfaces(prefix: string): ReadonlyArray<readonly [string, SurfaceKind]> {
  return [
    [`${prefix}.mcp.json`, 'mcp-json'],
    [`${prefix}.mcp/servers.json`, 'mcp-json'],
    [`${prefix}.harness/mcp-policy.json`, 'harness-policy'],
    [`${prefix}.harness/claims.json`, 'harness-policy'],
    [`${prefix}.harness/manifest.json`, 'harness-policy'],
    [`${prefix}.claude/settings.json`, 'claude-settings'],
    [`${prefix}.claude/settings.local.json`, 'claude-settings'],
    [`${prefix}.claude/proven-config.json`, 'claude-settings'],
    [`${prefix}.codex/config.toml`, 'codex-config'],
    [`${prefix}.codex/AGENTS.override.md`, 'instruction'],
    [`${prefix}.agents/config.toml`, 'agent-config'],
    [`${prefix}AGENTS.md`, 'instruction'],
    [`${prefix}CLAUDE.md`, 'instruction'],
  ];
}

function directorySurfaces(prefix: string): ReadonlyArray<readonly [string, SurfaceKind]> {
  return [
    [`${prefix}.claude/helpers`, 'hook-target'],
    [`${prefix}.claude/commands`, 'skill'],
    [`${prefix}.claude/skills`, 'skill'],
    [`${prefix}.agents/skills`, 'skill'],
    [`${prefix}.codex/skills`, 'skill'],
  ];
}

function addIfPresent(
  repositoryRoot: string,
  collection: ScopeCollection,
  path: string,
  kind: SurfaceKind,
  snapshot: ConfigurationRepositorySnapshot,
  budget: CollectionBudget,
  required: boolean,
): void {
  if (budget.exhausted) {
    collection.inventoryComplete = false;
    return;
  }
  const absolute = resolve(repositoryRoot, path);
  if (!contains(repositoryRoot, absolute)) {
    collection.inventoryComplete = false;
    return;
  }
  const cached = budget.cache.get(path);
  let file = cached;
  if (file === undefined) {
    try {
      file = readStableTextFile(repositoryRoot, path, snapshot);
    } catch (error) {
      if (!required && isMissing(error)) return;
      collection.inventoryComplete = false;
      return;
    }
    if (budget.bytes + file.bytes.length > MAX_TOTAL_BYTES) {
      budget.exhausted = true;
      collection.inventoryComplete = false;
      return;
    }
    budget.bytes += file.bytes.length;
    budget.cache.set(path, file);
  }
  const key = `${path}\0${kind}`;
  if (collection.surfaces.has(key)) return;
  if (budget.surfaces >= MAX_SURFACES) {
    budget.exhausted = true;
    collection.inventoryComplete = false;
    return;
  }
  budget.surfaces += 1;
  if (!file.trustworthy) collection.inventoryComplete = false;
  collection.surfaces.set(key, Object.freeze({
    path, kind, provenance: file.provenance, content: file.content,
  }));
}

function walkIfPresent(
  repositoryRoot: string,
  collection: ScopeCollection,
  path: string,
  kind: SurfaceKind,
  snapshot: ConfigurationRepositorySnapshot,
  budget: CollectionBudget,
): void {
  const root = resolve(repositoryRoot, path);
  let rootStat;
  try {
    rootStat = lstatSync(root, { bigint: true });
  } catch (error) {
    if (isMissing(error)) return;
    collection.inventoryComplete = false;
    return;
  }
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink() || realpathSync(root) !== root) {
    collection.inventoryComplete = false;
    return;
  }
  const pending: Array<{ path: string; depth: number }> = [{ path: root, depth: 0 }];
  while (pending.length > 0 && !budget.exhausted) {
    const current = pending.pop()!;
    if (current.depth > MAX_DIRECTORY_DEPTH) {
      collection.inventoryComplete = false;
      continue;
    }
    const before = lstatSync(current.path, { bigint: true });
    let directory;
    try {
      directory = opendirSync(current.path);
      let entry;
      while ((entry = directory.readSync()) !== null) {
        budget.entries += 1;
        if (budget.entries > MAX_DIRECTORY_ENTRIES) {
          budget.exhausted = true;
          collection.inventoryComplete = false;
          break;
        }
        const entryPath = resolve(current.path, entry.name);
        if (!contains(root, entryPath)) {
          collection.inventoryComplete = false;
        } else if (entry.isDirectory()) {
          pending.push({ path: entryPath, depth: current.depth + 1 });
        } else if (entry.isFile()) {
          const relativePath = relative(repositoryRoot, entryPath).split(sep).join('/');
          addIfPresent(repositoryRoot, collection, relativePath, kind, snapshot, budget, true);
        } else {
          collection.inventoryComplete = false;
        }
      }
    } catch {
      collection.inventoryComplete = false;
    } finally {
      directory?.closeSync();
    }
    const after = lstatSync(current.path, { bigint: true });
    if (!sameStat(before, after)) collection.inventoryComplete = false;
  }
}

function readStableTextFile(
  repositoryRoot: string,
  path: string,
  snapshot: ConfigurationRepositorySnapshot,
): CachedFile {
  const absolute = resolve(repositoryRoot, path);
  const descriptor = openSync(absolute, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1n
      || before.size > BigInt(MAX_SURFACE_BYTES) || realpathSync(absolute) !== absolute) {
      throw new Error('HARNESS_EFFECTIVE_CONFIG_SURFACE_INVALID');
    }
    const bytes = Buffer.alloc(Number(before.size));
    let offset = 0;
    while (offset < bytes.length) {
      const count = readSync(descriptor, bytes, offset, bytes.length - offset, offset);
      if (count === 0) break;
      offset += count;
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (offset !== bytes.length || !sameStat(before, after)) {
      throw new Error('HARNESS_EFFECTIVE_CONFIG_SURFACE_CHANGED');
    }
    const content = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    if (content.includes('\0')) throw new Error('HARNESS_EFFECTIVE_CONFIG_SURFACE_NOT_TEXT');
    const provenance = configurationFileProvenance(repositoryRoot, path, bytes, snapshot);
    return Object.freeze({ content, bytes, ...provenance });
  } finally {
    closeSync(descriptor);
  }
}

function discoverLocalTargets(
  surfaces: Iterable<CollectedSurface>,
  scope: ConfigurationScope,
): Array<string | null> {
  const targets = new Set<string | null>();
  for (const surface of surfaces) {
    if (surface.kind !== 'mcp-json' && surface.kind !== 'claude-settings') continue;
    try {
      collectNodeTargets(JSON.parse(surface.content) as unknown, targets, scope);
    } catch {
      // The semantic audit records malformed JSON; no command can be inventoried safely.
      targets.add(null);
    }
  }
  return [...targets];
}

function collectNodeTargets(
  value: unknown,
  targets: Set<string | null>,
  scope: ConfigurationScope,
): void {
  if (value === null || typeof value !== 'object') return;
  if (Array.isArray(value)) {
    value.forEach((entry) => collectNodeTargets(entry, targets, scope));
    return;
  }
  const record = value as Record<string, unknown>;
  if (typeof record.command === 'string') {
    const command = executableName(record.command);
    if (command === 'node' && Array.isArray(record.args)) {
      targets.add(normalizeDiscoveredPath(record.args[0], scope));
    }
    const match = record.command.match(/(?:^|\s)node(?:\.exe)?\s+["']?([^\s"']+)/i);
    if (match !== null) targets.add(normalizeDiscoveredPath(match[1], scope));
  }
  Object.values(record).forEach((entry) => collectNodeTargets(entry, targets, scope));
}

function normalizeDiscoveredPath(value: unknown, scope: ConfigurationScope): string | null {
  if (typeof value !== 'string' || value.includes('\0') || value.includes('\\')
    || isAbsolute(value) || value === '..' || value.startsWith('../') || value.includes('/../')
    || value.includes('$') || value.includes('{')) return null;
  const normalized = value.replace(/^\.\//, '');
  if (normalized === '' || normalized.split('/').some((segment) => segment === '' || segment === '.')) return null;
  return scope === 'coding-harness' && !normalized.startsWith('coding-harness/')
    ? `coding-harness/${normalized}` : normalized;
}

function executableName(command: string): string {
  return command.replaceAll('\\', '/').split('/').at(-1)?.replace(/\.(?:cmd|exe)$/i, '').toLowerCase() ?? '';
}

function canonicalDirectory(path: string): string {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')
    || !statSync(path).isDirectory() || realpathSync(path) !== path) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_REPOSITORY_ROOT_INVALID');
  }
  return path;
}

function contains(root: string, path: string): boolean {
  const delta = relative(root, path);
  return delta === '' || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

function sameStat(left: BigIntStats, right: BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.nlink === right.nlink && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function isMissing(error: unknown): boolean {
  return error !== null && typeof error === 'object' && 'code' in error
    && (error as { code?: string }).code === 'ENOENT';
}

// SPDX-License-Identifier: MIT

const CRATES_IO_SOURCE = 'registry+https://github.com/rust-lang/crates.io-index';
const CRATE_NAME = /^[A-Za-z0-9_-]{1,64}$/;
const CRATE_VERSION = /^[0-9A-Za-z][0-9A-Za-z.+-]{0,127}$/;

export interface FrozenRegistryPackage {
  readonly name: string;
  readonly version: string;
}

export function parseFrozenCargoMetadata(
  stdout: string,
  workspaceRoot: string,
  targetRoot: string,
): readonly FrozenRegistryPackage[] {
  let value: unknown;
  try { value = JSON.parse(stdout); } catch {
    throw new Error('HARNESS_FROZEN_LOCK_METADATA_INVALID');
  }
  const metadata = record(value, 'HARNESS_FROZEN_LOCK_METADATA_INVALID');
  if (metadata.workspace_root !== workspaceRoot || metadata.target_directory !== targetRoot
    || !Array.isArray(metadata.packages) || !Array.isArray(metadata.workspace_members)) {
    throw new Error('HARNESS_FROZEN_LOCK_METADATA_BINDING_MISMATCH');
  }
  const resolve = record(metadata.resolve, 'HARNESS_FROZEN_LOCK_METADATA_RESOLVE_INVALID');
  if (!Array.isArray(resolve.nodes)) {
    throw new Error('HARNESS_FROZEN_LOCK_METADATA_RESOLVE_INVALID');
  }
  const resolvedIds = new Set<string>();
  for (const node of resolve.nodes) {
    const input = record(node, 'HARNESS_FROZEN_LOCK_METADATA_NODE_INVALID');
    if (typeof input.id !== 'string' || input.id.length === 0 || input.id.includes('\0')
      || resolvedIds.has(input.id)) {
      throw new Error('HARNESS_FROZEN_LOCK_METADATA_NODE_INVALID');
    }
    resolvedIds.add(input.id);
  }
  const packages = new Map<string, FrozenRegistryPackage>();
  const seenIds = new Set<string>();
  for (const raw of metadata.packages) {
    const input = record(raw, 'HARNESS_FROZEN_LOCK_METADATA_PACKAGE_INVALID');
    if (typeof input.id !== 'string' || seenIds.has(input.id)) {
      throw new Error('HARNESS_FROZEN_LOCK_METADATA_PACKAGE_INVALID');
    }
    seenIds.add(input.id);
    if (!resolvedIds.has(input.id) || input.source !== CRATES_IO_SOURCE) continue;
    if (typeof input.name !== 'string' || !CRATE_NAME.test(input.name)
      || typeof input.version !== 'string' || !CRATE_VERSION.test(input.version)) {
      throw new Error('HARNESS_FROZEN_LOCK_METADATA_PACKAGE_INVALID');
    }
    const key = `${input.name}\0${input.version}`;
    if (packages.has(key)) throw new Error('HARNESS_FROZEN_LOCK_METADATA_PACKAGE_DUPLICATE');
    packages.set(key, Object.freeze({ name: input.name, version: input.version }));
  }
  for (const id of resolvedIds) {
    if (!seenIds.has(id)) throw new Error('HARNESS_FROZEN_LOCK_METADATA_NODE_UNBOUND');
  }
  return Object.freeze([...packages.values()].sort(comparePackage));
}

function record(value: unknown, error: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) throw new Error(error);
  return value as Record<string, unknown>;
}

function comparePackage(left: FrozenRegistryPackage, right: FrozenRegistryPackage): number {
  if (left.name !== right.name) return left.name < right.name ? -1 : 1;
  if (left.version === right.version) return 0;
  return left.version < right.version ? -1 : 1;
}

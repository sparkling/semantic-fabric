// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
  normalizeWorkspacePath,
} from './contracts.js';

export const CONTROLLER_BUILD_PATH = 'coding-harness/.harness/controller-build.json' as const;

export interface ControllerBuildManifest {
  readonly schemaVersion: 1;
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly runtimeEntry: string;
  readonly harnessManifestDigest: string;
  readonly lockfileDigest: string;
  readonly outputs: Readonly<Record<string, string>>;
  readonly productionFiles: Readonly<Record<string, string>>;
  readonly runtimeTreeDigest: string;
}

export function parseControllerBuildManifest(value: unknown): ControllerBuildManifest {
  const input = asRecord(value, 'controller build manifest');
  assertExactKeys(input, [
    'schemaVersion', 'authority', 'runtimeEntry', 'harnessManifestDigest',
    'lockfileDigest', 'outputs', 'productionFiles', 'runtimeTreeDigest',
  ], 'controller build manifest');
  if (input.schemaVersion !== 1 || input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('HARNESS_CONTROLLER_BUILD_AUTHORITY_INVALID');
  }
  const runtimeEntry = runtimePath(input.runtimeEntry, 'runtimeEntry', 'output');
  const outputs = digestMap(input.outputs, 'outputs', 'output');
  const productionFiles = digestMap(input.productionFiles, 'productionFiles', 'production');
  if (!(runtimeEntry in outputs)) throw new Error('HARNESS_CONTROLLER_BUILD_ENTRY_UNBOUND');
  const body = {
    schemaVersion: 1,
    authority: DEVELOPMENT_AUTHORITY,
    runtimeEntry,
    harnessManifestDigest: digest(input.harnessManifestDigest, 'harnessManifestDigest'),
    lockfileDigest: digest(input.lockfileDigest, 'lockfileDigest'),
    outputs,
    productionFiles,
  } as const;
  const runtimeTreeDigest = digest(input.runtimeTreeDigest, 'runtimeTreeDigest');
  if (sha256(JSON.stringify(body)) !== runtimeTreeDigest) {
    throw new Error('HARNESS_CONTROLLER_BUILD_TREE_DIGEST_MISMATCH');
  }
  return deepFreeze({ ...body, runtimeTreeDigest });
}

function digestMap(
  value: unknown,
  label: string,
  kind: 'output' | 'production',
): Record<string, string> {
  const input = asRecord(value, `controller build manifest.${label}`);
  const entries = Object.entries(input);
  if (entries.length === 0 || entries.length > 5_000) {
    throw new TypeError(`HARNESS_CONTROLLER_BUILD_${label.toUpperCase()}_INVALID`);
  }
  const output: Record<string, string> = {};
  let previous = '';
  for (const [rawPath, rawDigest] of entries) {
    const path = runtimePath(rawPath, `${label} path`, kind);
    if (previous !== '' && path <= previous) {
      throw new Error('HARNESS_CONTROLLER_BUILD_PATH_ORDER_INVALID');
    }
    output[path] = digest(rawDigest, `${label}.${path}`);
    previous = path;
  }
  return output;
}

function runtimePath(value: unknown, label: string, kind: 'output' | 'production'): string {
  let path: string;
  try {
    path = normalizeWorkspacePath(asNonEmptyString(value, label), label);
  } catch {
    throw new TypeError(`HARNESS_CONTROLLER_BUILD_${kind.toUpperCase()}_PATH_INVALID`);
  }
  const valid = kind === 'output'
    ? path.startsWith('coding-harness/dist/') && path.endsWith('.js')
    : path.startsWith('coding-harness/node_modules/');
  if (!valid || Buffer.byteLength(path, 'utf8') > 500) {
    throw new TypeError(`HARNESS_CONTROLLER_BUILD_${kind.toUpperCase()}_PATH_INVALID`);
  }
  return path;
}

function digest(value: unknown, label: string): string {
  const parsed = asNonEmptyString(value, label);
  if (!SHA256_PATTERN.test(parsed) || parsed === '0'.repeat(64)) {
    throw new TypeError(`HARNESS_CONTROLLER_BUILD_${label.toUpperCase()}_INVALID`);
  }
  return parsed;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

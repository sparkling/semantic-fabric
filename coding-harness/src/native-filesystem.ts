// SPDX-License-Identifier: MIT

import {
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  asUniqueStrings,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  assertBoundaryCommandBinding,
  assertNoRouteEnvironment,
  parseBoundaryCommand,
  type BoundaryCommand,
} from './network.js';
import type { NativeHost } from './models/types.js';

export interface NativeFilesystemPolicy {
  readonly host: NativeHost;
  readonly workspaceRoot: string;
  readonly readOnlyRoots: readonly string[];
  readonly writablePaths: readonly string[];
  readonly maskedPaths: readonly string[];
  readonly hostFileConfidentiality: true;
  readonly emptyPrivateHome: true;
  readonly hostRootMounted: false;
}

export interface NativeFilesystemIsolationResult extends NativeFilesystemPolicy {
  readonly enforcement: 'os-filesystem-namespace';
  readonly mechanism: string;
  readonly mountManifestDigest: string;
  readonly command: BoundaryCommand;
}

export interface NativeModelFilesystemBoundary {
  isolate(command: BoundaryCommand, policy: NativeFilesystemPolicy): unknown;
}

const RESULT_KEYS = [
  'enforcement', 'mechanism', 'mountManifestDigest', 'host', 'workspaceRoot',
  'readOnlyRoots', 'writablePaths', 'hostFileConfidentiality', 'emptyPrivateHome',
  'hostRootMounted', 'maskedPaths', 'command',
] as const;
const MECHANISM = /^[a-z0-9][a-z0-9._-]{0,63}$/;

export function isolateNativeModelFilesystem(
  command: BoundaryCommand,
  policy: NativeFilesystemPolicy,
  boundary?: NativeModelFilesystemBoundary,
): NativeFilesystemIsolationResult {
  if (boundary === undefined) throw new Error('HARNESS_NATIVE_FILESYSTEM_BOUNDARY_REQUIRED');
  const input = asRecord(boundary.isolate(command, policy), 'native filesystem isolation result');
  assertExactKeys(input, RESULT_KEYS, 'native filesystem isolation result');
  if (input.enforcement !== 'os-filesystem-namespace'
    || input.host !== policy.host
    || input.workspaceRoot !== policy.workspaceRoot
    || input.hostFileConfidentiality !== true
    || input.emptyPrivateHome !== true
    || input.hostRootMounted !== false) {
    throw new Error('HARNESS_NATIVE_FILESYSTEM_BOUNDARY_INVALID');
  }
  const mechanism = asNonEmptyString(input.mechanism, 'native filesystem mechanism');
  if (!MECHANISM.test(mechanism)) throw new Error('HARNESS_NATIVE_FILESYSTEM_BOUNDARY_INVALID');
  const mountManifestDigest = asNonEmptyString(
    input.mountManifestDigest,
    'native filesystem mount manifest digest',
  );
  if (!SHA256_PATTERN.test(mountManifestDigest) || mountManifestDigest === '0'.repeat(64)) {
    throw new Error('HARNESS_NATIVE_FILESYSTEM_BOUNDARY_INVALID');
  }
  const readOnlyRoots = asUniqueStrings(input.readOnlyRoots, 'native filesystem readOnlyRoots');
  const writablePaths = asUniqueStrings(
    input.writablePaths,
    'native filesystem writablePaths',
    true,
  );
  const maskedPaths = asUniqueStrings(input.maskedPaths, 'native filesystem maskedPaths');
  if (!same(readOnlyRoots, policy.readOnlyRoots)
    || !same(writablePaths, policy.writablePaths)
    || !same(maskedPaths, policy.maskedPaths)) {
    throw new Error('HARNESS_NATIVE_FILESYSTEM_SCOPE_MISMATCH');
  }
  const bounded = parseBoundaryCommand(input.command, 'native filesystem command');
  assertBoundaryCommandBinding(
    command,
    bounded,
    true,
    'HARNESS_NATIVE_FILESYSTEM_COMMAND_MISMATCH',
  );
  assertNoRouteEnvironment(bounded.env, 'native filesystem command.env');
  return deepFreeze({
    enforcement: 'os-filesystem-namespace',
    mechanism,
    mountManifestDigest,
    host: policy.host,
    workspaceRoot: policy.workspaceRoot,
    readOnlyRoots,
    writablePaths,
    maskedPaths,
    hostFileConfidentiality: true,
    emptyPrivateHome: true,
    hostRootMounted: false,
    command: bounded,
  });
}

function same(left: readonly string[], right: readonly string[]): boolean {
  return JSON.stringify([...left].sort()) === JSON.stringify([...right].sort());
}

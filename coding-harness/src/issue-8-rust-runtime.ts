// SPDX-License-Identifier: MIT

import { SECURE_HARNESS_CONFIG } from './config.js';
import type { FrozenRegistryPackage } from './frozen-cargo-metadata.js';
import type { FrozenCargoLockFile } from './frozen-cargo-lock.js';
import { prepareIssue8RustClosure } from './rust-closure.js';
import { SystemdResourceBoundary } from './resource-boundary.js';
import { createRustOfflineProfile, type RustOfflineProfile } from './rust-sandbox.js';
import {
  ISSUE_8_LOCKED_REGISTRY_CONTENT_DIGEST,
  ISSUE_8_RUST_LIMITS,
  ISSUE_8_SYSTEM_PATHS,
  ISSUE_8_TARGET_TRIPLE,
} from './issue-8-system.js';

export interface Issue8LockedRustRuntime {
  readonly profile: RustOfflineProfile;
  readonly toolVersions: Readonly<Record<string, string>>;
}

export interface Issue8RustRuntimeFactory {
  readonly bootstrapEvidence: Readonly<Record<string, string>>;
  createBootstrapProfile(writableRoot: string): RustOfflineProfile;
  createLockedRuntime(
    writableRoot: string,
    lockfile: FrozenCargoLockFile,
    packages: readonly FrozenRegistryPackage[],
  ): Issue8LockedRustRuntime;
}

export function prepareIssue8RustRuntimeFactory(input: Readonly<{
  scratchRoot: string;
  cargoExtensionRoot: string;
}>): Issue8RustRuntimeFactory {
  const closure = prepareIssue8RustClosure({
    scratchRoot: input.scratchRoot,
    toolchainSource: ISSUE_8_SYSTEM_PATHS.toolchain,
    registrySource: ISSUE_8_SYSTEM_PATHS.registry,
  });
  const profile = (
    writableRoot: string,
    registryRoot: string,
    assertStable: () => void,
  ) => createRustOfflineProfile({
    writableRoot,
    cargoExecutable: closure.cargoExecutable,
    toolchainRoot: closure.toolchainRoot,
    registryRoot,
    registryKey: ISSUE_8_SYSTEM_PATHS.registryKey,
    cargoExtensionRoot: input.cargoExtensionRoot,
    bwrapExecutable: ISSUE_8_SYSTEM_PATHS.bwrap,
    resourceBoundary: new SystemdResourceBoundary({
      executablePath: ISSUE_8_SYSTEM_PATHS.systemdRun,
      systemctlPath: ISSUE_8_SYSTEM_PATHS.systemctl,
      terminationGraceMs: SECURE_HARNESS_CONFIG.limits.terminationGraceMs,
      sourceEnvironment: process.env,
    }),
    resourceLimits: ISSUE_8_RUST_LIMITS,
    assertClosureStable: assertStable,
  });
  return Object.freeze({
    bootstrapEvidence: closure.evidence,
    createBootstrapProfile: (writableRoot: string) =>
      profile(writableRoot, closure.registryRoot, closure.assertStable),
    createLockedRuntime(
      writableRoot: string,
      lockfile: FrozenCargoLockFile,
      packages: readonly FrozenRegistryPackage[],
    ) {
      const locked = closure.lock({
        lockfilePath: lockfile.sourcePath,
        lockfileDigest: lockfile.digest,
        packages,
        targetTriple: ISSUE_8_TARGET_TRIPLE,
        expectedContentDigest: ISSUE_8_LOCKED_REGISTRY_CONTENT_DIGEST,
      });
      return Object.freeze({
        profile: profile(writableRoot, locked.registryRoot, locked.assertStable),
        toolVersions: locked.evidence,
      });
    },
  });
}

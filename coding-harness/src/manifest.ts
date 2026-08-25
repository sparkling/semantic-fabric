// SPDX-License-Identifier: MIT

import type { HarnessConfig } from './contracts.js';
import {
  DEVELOPMENT_AUTHORITY,
  asInteger,
  asNonEmptyString,
  asRecord,
  asUniqueStrings,
  assertExactKeys,
  deepFreeze,
  normalizeWorkspacePath,
} from './contracts.js';

export interface HarnessManifest {
  schemaVersion: 1;
  name: 'semantic-fabric-coding-harness';
  authority: typeof DEVELOPMENT_AUTHORITY;
  runtime: {
    harness: '@metaharness/harness@0.2.0';
    router: '@metaharness/router@0.4.0';
    codexHost: '@metaharness/host-codex@0.1.2';
    claudeHost: '@metaharness/host-claude-code@0.1.2';
  };
  entrypoint: string;
  coordinationSurface: '.mcp.json';
  protectedPaths: string[];
  acceptanceTasks: string[];
  diagnostics: {
    minimumScore: 98;
    blindSurfaceOutcome: 'INCONCLUSIVE';
  };
  evolution: {
    eligible: false;
    minimumTrainingTasks: 5;
    minimumSealedHoldouts: 5;
    suiteFile: null;
  };
}

const RUNTIME = Object.freeze({
  harness: '@metaharness/harness@0.2.0',
  router: '@metaharness/router@0.4.0',
  codexHost: '@metaharness/host-codex@0.1.2',
  claudeHost: '@metaharness/host-claude-code@0.1.2',
} as const);

export function parseHarnessManifest(value: unknown, config: HarnessConfig): HarnessManifest {
  const input = asRecord(value, 'harness manifest');
  assertExactKeys(input, [
    'schemaVersion', 'name', 'authority', 'runtime', 'entrypoint', 'coordinationSurface',
    'protectedPaths', 'acceptanceTasks', 'diagnostics', 'evolution',
  ], 'harness manifest');
  if (input.schemaVersion !== 1) throw new TypeError('harness manifest schemaVersion must be 1');
  if (input.name !== 'semantic-fabric-coding-harness') throw new TypeError('harness manifest name is invalid');
  if (input.authority !== DEVELOPMENT_AUTHORITY) throw new TypeError('harness manifest cannot grant promotion');
  if (input.coordinationSurface !== '.mcp.json') throw new TypeError('harness manifest must expose the real .mcp.json surface');

  const runtime = asRecord(input.runtime, 'harness manifest.runtime');
  assertExactKeys(runtime, Object.keys(RUNTIME), 'harness manifest.runtime');
  for (const [name, expected] of Object.entries(RUNTIME)) {
    if (runtime[name] !== expected) throw new TypeError(`harness manifest runtime ${name} is not pinned`);
  }
  const parsePaths = (raw: unknown, label: string) => asUniqueStrings(raw, label)
    .map((path, index) => normalizeWorkspacePath(path, `${label}[${index}]`));
  const protectedPaths = parsePaths(input.protectedPaths, 'harness manifest.protectedPaths');
  const expectedProtected = [...config.requiredProtectedPaths].sort();
  if (JSON.stringify([...protectedPaths].sort()) !== JSON.stringify(expectedProtected)) {
    throw new Error('HARNESS_MANIFEST_PROTECTED_PATHS_MISMATCH');
  }
  const acceptanceTasks = parsePaths(input.acceptanceTasks, 'harness manifest.acceptanceTasks');
  if (!acceptanceTasks.includes('coding-harness/config/issue-8-acceptance.json')) {
    throw new Error('HARNESS_MANIFEST_ISSUE_8_TASK_MISSING');
  }

  const diagnostics = asRecord(input.diagnostics, 'harness manifest.diagnostics');
  assertExactKeys(diagnostics, ['minimumScore', 'blindSurfaceOutcome'], 'harness manifest.diagnostics');
  if (asInteger(diagnostics.minimumScore, 'harness manifest.diagnostics.minimumScore') !== 98
    || diagnostics.blindSurfaceOutcome !== 'INCONCLUSIVE') {
    throw new TypeError('harness manifest diagnostic gates are invalid');
  }
  const evolution = asRecord(input.evolution, 'harness manifest.evolution');
  assertExactKeys(
    evolution,
    ['eligible', 'minimumTrainingTasks', 'minimumSealedHoldouts', 'suiteFile'],
    'harness manifest.evolution',
  );
  if (evolution.eligible !== false
    || evolution.minimumTrainingTasks !== 5
    || evolution.minimumSealedHoldouts !== 5
    || evolution.suiteFile !== null) {
    throw new TypeError('harness manifest evolution gate is invalid');
  }

  return deepFreeze({
    schemaVersion: 1,
    name: 'semantic-fabric-coding-harness',
    authority: DEVELOPMENT_AUTHORITY,
    runtime: { ...RUNTIME },
    entrypoint: normalizeWorkspacePath(
      asNonEmptyString(input.entrypoint, 'harness manifest.entrypoint'),
      'harness manifest.entrypoint',
    ),
    coordinationSurface: '.mcp.json',
    protectedPaths,
    acceptanceTasks,
    diagnostics: { minimumScore: 98, blindSurfaceOutcome: 'INCONCLUSIVE' },
    evolution: {
      eligible: false,
      minimumTrainingTasks: 5,
      minimumSealedHoldouts: 5,
      suiteFile: null,
    },
  });
}

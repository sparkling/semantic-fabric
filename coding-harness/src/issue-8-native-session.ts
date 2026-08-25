// SPDX-License-Identifier: MIT

import { homedir } from 'node:os';
import { dirname } from 'node:path';
import type { HarnessConfig } from './contracts.js';
import type { PreparedWorktrees } from './git-worktrees.js';
import type { Issue8NativeSession } from './issue-8-driver.js';
import {
  createTrustedNativeRuntime,
  type NativeRuntimeExecutablePaths,
} from './native-runtime.js';
import type { NativeResourceLimits } from './resource-boundary.js';

export const ISSUE_8_MODEL_TIMEOUT_MS = 600_000;

export interface Issue8NativeSessionOptions {
  readonly config: HarnessConfig;
  readonly controllerRoot: string;
  readonly runtimeParent: string;
  readonly prepared: PreparedWorktrees;
  readonly evaluatorPaths: readonly string[];
  readonly models: Readonly<{ codex: string; claude: string }>;
  readonly executables: NativeRuntimeExecutablePaths;
  readonly credentials: Readonly<{ codex: string; 'claude-code': string }>;
  readonly resourceLimits: NativeResourceLimits;
  readonly controllerEnvironment: Readonly<Record<string, string | undefined>>;
}

export async function createIssue8NativeSession(
  options: Issue8NativeSessionOptions,
): Promise<Issue8NativeSession> {
  if (ISSUE_8_MODEL_TIMEOUT_MS >= options.resourceLimits.runtimeSeconds * 1_000) {
    throw new Error('HARNESS_ISSUE_8_MODEL_TIMEOUT_NOT_NESTED');
  }
  const runRoot = preparedRunRoot(options.prepared);
  const runtime = createTrustedNativeRuntime({
    config: options.config,
    runtimeParent: options.runtimeParent,
    allowedWorkspaceRoots: [options.prepared.candidateRoot],
    workspaceRoot: options.prepared.candidateRoot,
    executables: options.executables,
    credentials: options.credentials,
    resourceLimits: options.resourceLimits,
    timeoutMs: ISSUE_8_MODEL_TIMEOUT_MS,
    maskedWorkspacePaths: options.evaluatorPaths,
    forbiddenMountRoots: [
      homedir(), options.controllerRoot, runRoot, options.runtimeParent,
      dirname(options.credentials.codex), dirname(options.credentials['claude-code']),
    ],
    controllerEnvironment: options.controllerEnvironment,
  });
  try {
    const preflight = await runtime.preflight(options.models, options.prepared.candidateRoot);
    const candidates: [
      Issue8NativeSession['candidates'][0],
      Issue8NativeSession['candidates'][1],
    ] = [{
      id: 'codex-native-subscription',
      host: 'codex' as const,
      model: options.models.codex,
      handles: ['architecture', 'implementation', 'repair', 'review'],
      run: unavailablePoolExecution,
    }, {
      id: 'claude-native-subscription',
      host: 'claude-code' as const,
      model: options.models.claude,
      handles: ['architecture', 'implementation', 'repair', 'review'],
      run: unavailablePoolExecution,
    }];
    return Object.freeze({
      candidates: Object.freeze(candidates),
      clients: runtime.clients,
      hosts: preflight.hosts,
      ledger: runtime.ledger,
      cleanup: runtime.cleanup,
    });
  } catch (error) {
    runtime.cleanup();
    throw error;
  }
}

async function unavailablePoolExecution(): Promise<never> {
  throw new Error('HARNESS_NATIVE_POOL_EXECUTION_MUST_USE_STRUCTURED_CLIENT');
}

function preparedRunRoot(prepared: PreparedWorktrees): string {
  const runRoot = dirname(prepared.candidateRoot);
  const roots = [
    prepared.candidateRoot,
    prepared.evaluatorRoot,
    ...Object.values(prepared.verifierRoots),
  ];
  if (roots.some((root) => dirname(root) !== runRoot)) {
    throw new Error('HARNESS_ISSUE_8_WORKTREE_ROOTS_DIVERGED');
  }
  return runRoot;
}

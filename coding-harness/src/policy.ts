// SPDX-License-Identifier: MIT

import { existsSync } from 'node:fs';
import { PolicyGate, allowTools, denyTools } from '@metaharness/harness';
import type { HarnessConfig, TaskContract } from './contracts.js';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asRecord,
  assertExactKeys,
  assertStructuredText,
  deepFreeze,
  normalizePublicHttpsOrigin,
  normalizeWorkspacePath,
  pathsOverlap,
} from './contracts.js';
import { runGitCommand } from './git-process.js';
import { countFileLines, resolveMutablePath, resolveWorkspacePath, sha256File } from './workspace.js';

export type PolicyActionKind = 'read' | 'write' | 'execute' | 'network' | 'promote';
export type PolicyGateName = 'path' | 'tool' | 'network' | 'authority' | 'protectedInput';

export interface PolicyAction {
  kind: PolicyActionKind;
  tool: string;
  path: string | null;
  origin: string | null;
  authority: string;
}

export interface GateDecision {
  allow: boolean;
  reasons: string[];
}

export interface ProtectedInputBoundary {
  capture(task: TaskContract, config: HarnessConfig): Promise<Readonly<Record<string, string>>>;
  verify(
    task: TaskContract,
    config: HarnessConfig,
    expected: Readonly<Record<string, string>>,
  ): Promise<GateDecision>;
}

export interface HarnessPolicyDecision {
  allow: boolean;
  gates: Record<PolicyGateName, GateDecision>;
  reasons: string[];
}

const PROMOTION_TOOLS = ['commit', 'merge', 'push', 'publish', 'deploy', 'release'];

export class HarnessPolicy {
  readonly #task: TaskContract;
  readonly #config: HarnessConfig;
  readonly #toolGate: PolicyGate;

  constructor(task: TaskContract, config: HarnessConfig) {
    this.#task = task;
    this.#config = config;
    this.#toolGate = new PolicyGate([
      allowTools(task.tools, 0.1),
      denyTools(PROMOTION_TOOLS, 1),
    ], 0.2);
  }

  evaluate(value: unknown): HarnessPolicyDecision {
    const action = parsePolicyAction(value);
    const gates: Record<PolicyGateName, GateDecision> = {
      path: this.#evaluatePath(action),
      tool: this.#evaluateTool(action),
      network: this.#evaluateNetwork(action),
      authority: this.#evaluateAuthority(action),
      protectedInput: this.#evaluateProtectedInput(action),
    };
    const reasons = Object.entries(gates)
      .filter(([, decision]) => !decision.allow)
      .flatMap(([gate, decision]) => decision.reasons.map((reason) => `${gate}: ${reason}`));
    return deepFreeze({ allow: Object.values(gates).every((gate) => gate.allow), gates, reasons });
  }

  #evaluatePath(action: PolicyAction): GateDecision {
    if (action.kind !== 'read' && action.kind !== 'write') {
      return action.path === null ? allow('no filesystem path requested') : deny('non-filesystem action supplied a path');
    }
    if (action.path === null) return deny(`${action.kind} actions require an exact path`);
    try {
      const path = normalizeWorkspacePath(action.path, 'action.path');
      const declared = action.kind === 'read'
        ? [...this.#task.readablePaths, ...this.#task.mutablePaths, ...this.#task.protectedPaths]
        : this.#task.mutablePaths;
      if (!declared.includes(path)) return deny(`${path} is not on the exact ${action.kind} allowlist`);
      if (action.kind === 'read') {
        resolveWorkspacePath(this.#task.workspaceRoot, path, {
          requireRegularFile: true,
          rejectHardlinks: true,
        });
      } else {
        resolveMutablePath(this.#task.workspaceRoot, path);
      }
      return allow(`${path} is an exact declared ${action.kind} path`);
    } catch (error) {
      return deny(errorMessage(error));
    }
  }

  #evaluateTool(action: PolicyAction): GateDecision {
    const decision = this.#toolGate.evaluate({ tool: action.tool, args: { kind: action.kind } });
    const kindMismatch = (action.kind === 'read' && action.tool !== 'read_file')
      || (action.kind === 'write' && !['write_file', 'apply_patch'].includes(action.tool))
      || (action.kind === 'execute' && ['read_file', 'write_file', 'apply_patch'].includes(action.tool));
    if (kindMismatch) return deny(`tool ${action.tool} cannot perform ${action.kind}`);
    return { allow: decision.allow, reasons: decision.reasons };
  }

  #evaluateNetwork(action: PolicyAction): GateDecision {
    if (action.kind !== 'network') {
      return action.origin === null ? allow('no network origin requested') : deny('origin declared outside a network action');
    }
    if (action.origin === null) return deny('network actions require an origin');
    let origin: string;
    try {
      origin = normalizePublicHttpsOrigin(action.origin, 'action.origin');
    } catch (error) {
      return deny(errorMessage(error));
    }
    if (this.#task.network.mode === 'offline') return deny('candidate task is offline');
    if (!this.#task.network.allowedOrigins.includes(origin)) return deny(`${origin} is not declared by the task`);
    if (this.#task.network.mode === 'dependency-resolution') {
      return origin === new URL(this.#config.approvedRegistry).origin
        ? allow('approved dependency registry')
        : deny('dependency resolution is registry-only');
    }
    return this.#config.firstPartyOrigins.includes(origin)
      ? allow('configured first-party model origin')
      : deny('indirect model gateway is prohibited');
  }

  #evaluateAuthority(action: PolicyAction): GateDecision {
    if (action.kind === 'promote') return deny('the harness has no promotion authority');
    return action.authority === DEVELOPMENT_AUTHORITY
      ? allow(DEVELOPMENT_AUTHORITY)
      : deny('authority must remain development-only-no-promotion');
  }

  #evaluateProtectedInput(action: PolicyAction): GateDecision {
    if (action.kind !== 'write' || action.path === null) return allow('protected inputs are not being mutated');
    try {
      const path = normalizeWorkspacePath(action.path, 'action.path');
      const protectedPath = this.#task.protectedPaths.find((candidate) => pathsOverlap(path, candidate));
      return protectedPath ? deny(`${path} overlaps protected input ${protectedPath}`) : allow('no protected overlap');
    } catch (error) {
      return deny(errorMessage(error));
    }
  }
}

export async function captureProtectedInputs(
  task: TaskContract,
  config: HarnessConfig,
): Promise<Readonly<Record<string, string>>> {
  const digests: Record<string, string> = {};
  for (const path of [...task.protectedPaths].sort()) {
    const tracked = await runGitCommand(
      task.workspaceRoot,
      ['ls-files', '--error-unmatch', '--', path],
      {
        timeoutMs: Math.min(10_000, config.limits.maxTimeoutMs),
        maxOutputBytes: Math.min(1_000_000, config.limits.maxOutputBytes),
      },
    );
    if (tracked.exitCode !== 0 || tracked.stdout.trim() !== path) {
      throw new Error(`protected input is not tracked: ${path}`);
    }
    const absolute = resolveWorkspacePath(task.workspaceRoot, path, {
      requireRegularFile: true,
      rejectHardlinks: true,
    });
    digests[path] = sha256File(absolute);
  }
  assertProtectedInputSnapshot(task, digests);
  return deepFreeze(digests);
}

export async function verifyProtectedInputs(
  task: TaskContract,
  config: HarnessConfig,
  expected: Readonly<Record<string, string>>,
): Promise<GateDecision> {
  try {
    assertProtectedInputSnapshot(task, expected);
    const current = await captureProtectedInputs(task, config);
    const expectedPaths = Object.keys(expected).sort();
    const currentPaths = Object.keys(current).sort();
    if (JSON.stringify(expectedPaths) !== JSON.stringify(currentPaths)) {
      return deny('protected input path set changed');
    }
    const changed = expectedPaths.filter((path) => expected[path] !== current[path]);
    return changed.length === 0 ? allow('protected input digests match') : deny(`protected inputs changed: ${changed.join(', ')}`);
  } catch (error) {
    return deny(errorMessage(error));
  }
}

export const DEFAULT_PROTECTED_INPUT_BOUNDARY: ProtectedInputBoundary = Object.freeze({
  capture: captureProtectedInputs,
  verify: verifyProtectedInputs,
});

export function assertProtectedInputSnapshot(
  task: TaskContract,
  snapshot: Readonly<Record<string, string>>,
): void {
  const expectedPaths = [...task.protectedPaths].sort();
  const actualPaths = Object.keys(snapshot).sort();
  if (JSON.stringify(actualPaths) !== JSON.stringify(expectedPaths)) {
    throw new Error('HARNESS_PROTECTED_INPUT_PATH_SET_MISMATCH');
  }
  if (actualPaths.some((path) => !SHA256_PATTERN.test(snapshot[path]))) {
    throw new Error('HARNESS_PROTECTED_INPUT_DIGEST_INVALID');
  }
}

export async function listTrackedPaths(task: TaskContract, config: HarnessConfig): Promise<readonly string[]> {
  const result = await runGitCommand(
    task.workspaceRoot,
    ['ls-files', '-z', '--'],
    {
      timeoutMs: Math.min(10_000, config.limits.maxTimeoutMs),
      maxOutputBytes: config.limits.maxOutputBytes,
    },
  );
  if (result.exitCode !== 0) {
    throw new Error('HARNESS_POLICY_TRACKED_PATH_ENUMERATION_FAILED');
  }
  return deepFreeze([...new Set(result.stdout.split('\0').filter(Boolean))].sort());
}

export function auditMutableOutputs(
  task: TaskContract,
  config: HarnessConfig,
  trackedAtStart: readonly string[],
): GateDecision {
  const failures: string[] = [];
  for (const path of task.mutablePaths) {
    try {
      const absolute = resolveMutablePath(task.workspaceRoot, path);
      if (!existsSync(absolute)) continue;
      if (!trackedAtStart.includes(path) && countFileLines(absolute) > config.limits.maxNewFileLines) {
        failures.push(`${path} exceeds ${config.limits.maxNewFileLines} lines`);
      }
    } catch (error) {
      failures.push(`${path}: ${errorMessage(error)}`);
    }
  }
  return failures.length === 0 ? allow('mutable outputs remain within policy') : deny(...failures);
}

function parsePolicyAction(value: unknown): PolicyAction {
  const input = asRecord(value, 'action');
  assertExactKeys(input, ['kind', 'tool', 'path', 'origin', 'authority'], 'action');
  const kind = input.kind;
  if (kind !== 'read' && kind !== 'write' && kind !== 'execute' && kind !== 'network' && kind !== 'promote') {
    throw new TypeError('action.kind is invalid');
  }
  const nullableString = (entry: unknown, label: string) => entry === null ? null : assertStructuredText(entry, label);
  return {
    kind,
    tool: assertStructuredText(input.tool, 'action.tool'),
    path: nullableString(input.path, 'action.path'),
    origin: nullableString(input.origin, 'action.origin'),
    authority: assertStructuredText(input.authority, 'action.authority'),
  };
}

function allow(...reasons: string[]): GateDecision {
  return { allow: true, reasons };
}

function deny(...reasons: string[]): GateDecision {
  return { allow: false, reasons };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

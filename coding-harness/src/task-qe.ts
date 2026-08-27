// SPDX-License-Identifier: MIT

import { AgenticQeLcovGapEvidenceAdapter } from './agentic-qe-lcov.js';
import { SystemAgenticQeMcpRunner } from './agentic-qe-mcp-runner.js';
import { AgenticQeSastEvidenceAdapter } from './agentic-qe-sast.js';
import type {
  LcovGapQeBinding,
  SastQeBinding,
  TaskQeBinding,
} from './acceptance-task-v3.js';
import { assertTaskQeBindings } from './acceptance-task-v3.js';
import type { CandidateBuild } from './candidate.js';
import { SECURE_HARNESS_CONFIG } from './config.js';
import {
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import { IndependentRustLcovGenerator } from './independent-rust-lcov.js';
import { runAbortableCohort } from './parallel.js';
import type { RustOfflineProfile } from './rust-sandbox.js';

export interface TaskQePackageIdentity {
  readonly version: string;
  readonly treeSha256: string;
}

export const LEGACY_TASK_QE_PACKAGE_IDENTITY = expectedPackageIdentity({
  version: '3.13.10',
  treeSha256: '9e08c960bc1d8150d3814c2b4395762ec640afa5cdd8cbf216fafc255ed1d7a7',
});

export interface TaskQeContext {
  readonly controlledRoot: string;
  readonly candidateRoot: string;
  readonly outputRoot: string;
  readonly rustProfile: RustOfflineProfile;
}

export type TaskQeCollector = (
  build: CandidateBuild,
  context: TaskQeContext,
  signal?: AbortSignal,
) => Promise<readonly unknown[]>;

export type TaskQeCollectorFactory = (input: Readonly<{
  taskId: string;
  runId: string;
  qeBindings: readonly TaskQeBinding[];
}>) => TaskQeCollector;

export interface TaskQeCollectorOptions {
  readonly taskId: string;
  readonly runId: string;
  readonly qeBindings: readonly TaskQeBinding[];
  readonly snapshotParent: string;
  readonly nodeExecutable: string;
  readonly bwrapExecutable: string;
  readonly packageRoot: string;
  readonly mcpExecutable: string;
}

export interface TaskQeCaptures {
  captureLcov(binding: LcovGapQeBinding, signal: AbortSignal): Promise<unknown>;
  captureSast(binding: SastQeBinding, signal: AbortSignal): Promise<unknown>;
}

export async function collectTaskQeBindings(
  bindings: readonly TaskQeBinding[],
  captures: TaskQeCaptures,
  signal?: AbortSignal,
): Promise<readonly unknown[]> {
  assertTaskQeBindings(bindings);
  const tasks = bindings.map((binding) => async (cohortSignal: AbortSignal) => {
    if (binding.profile === 'lcov-gap' && binding.collector === 'rust-lcov') {
      return await captures.captureLcov(binding, cohortSignal);
    }
    if (binding.profile === 'sast' && binding.collector === 'agentic-qe-sast') {
      return await captures.captureSast(binding, cohortSignal);
    }
    throw new Error('HARNESS_TASK_QE_BINDING_INVALID');
  });
  return await runAbortableCohort(tasks, signal);
}

export function createTaskQeCollector(options: Readonly<TaskQeCollectorOptions>): TaskQeCollector {
  return createTaskQeCollectorForPackage(options, LEGACY_TASK_QE_PACKAGE_IDENTITY);
}

export function createTaskQeCollectorForPackage(
  options: Readonly<TaskQeCollectorOptions>,
  packageIdentity: Readonly<TaskQePackageIdentity>,
): TaskQeCollector {
  assertTaskQeBindings(options.qeBindings);
  const expected = expectedPackageIdentity(packageIdentity);
  const bindings = deepFreeze(options.qeBindings.map(cloneBinding));
  const runner = new SystemAgenticQeMcpRunner({
    nodeExecutable: options.nodeExecutable,
    aqeMcpExecutable: options.mcpExecutable,
    aqePackageRoot: options.packageRoot,
    bwrapExecutable: options.bwrapExecutable,
  });
  assertTaskQeRunnerIdentity(runner.identityEvidence(), expected);
  const lcov = new AgenticQeLcovGapEvidenceAdapter({ runner });
  const sast = new AgenticQeSastEvidenceAdapter({
    runner,
    snapshotParent: options.snapshotParent,
  });

  return async (build, context, signal) => await collectTaskQeBindings(bindings, {
    captureLcov: async (binding, cohortSignal) => {
      const generator = new IndependentRustLcovGenerator({
        config: SECURE_HARNESS_CONFIG,
        rustProfile: context.rustProfile,
        packageName: binding.packageName,
        testTarget: binding.testTarget,
      });
      const artifact = await generator.capture({
        controlledRoot: context.controlledRoot,
        candidateRoot: context.candidateRoot,
        candidateTree: build.candidate.tree,
        outputRoot: context.outputRoot,
        signal: cohortSignal,
      });
      return await lcov.capture({
        taskId: options.taskId,
        runId: options.runId,
        candidateTree: build.candidate.tree,
        candidateRoot: context.candidateRoot,
        lcov: artifact,
      }, cohortSignal);
    },
    captureSast: async (_binding, cohortSignal) => await sast.capture({
      taskId: options.taskId,
      runId: options.runId,
      candidateTree: build.candidate.tree,
      candidateRoot: context.candidateRoot,
    }, cohortSignal),
  }, signal);
}

export function assertTaskQeRunnerIdentity(identity: Readonly<{
  package: Readonly<{ version: string; treeSha256: string }>;
}>, expected: Readonly<TaskQePackageIdentity> = LEGACY_TASK_QE_PACKAGE_IDENTITY): void {
  const packageIdentity = expectedPackageIdentity(expected);
  if (identity.package.version !== packageIdentity.version
    || identity.package.treeSha256 !== packageIdentity.treeSha256) {
    throw new Error('HARNESS_TASK_QE_IDENTITY_MISMATCH');
  }
}

function expectedPackageIdentity(value: unknown): TaskQePackageIdentity {
  const input = asRecord(value, 'task QE expected package identity');
  assertExactKeys(input, ['version', 'treeSha256'], 'task QE expected package identity');
  const version = asNonEmptyString(input.version, 'task QE expected package identity.version');
  const treeSha256 = asNonEmptyString(
    input.treeSha256,
    'task QE expected package identity.treeSha256',
  );
  if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/.test(version)
    || !SHA256_PATTERN.test(treeSha256)) {
    throw new TypeError('HARNESS_TASK_QE_EXPECTED_IDENTITY_INVALID');
  }
  return deepFreeze({ version, treeSha256 });
}

function cloneBinding(binding: TaskQeBinding): TaskQeBinding {
  return binding.profile === 'lcov-gap'
    ? {
        profile: 'lcov-gap', collector: 'rust-lcov',
        packageName: binding.packageName, testTarget: binding.testTarget,
      }
    : { profile: 'sast', collector: 'agentic-qe-sast' };
}

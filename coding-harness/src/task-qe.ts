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
import { deepFreeze } from './contracts.js';
import { IndependentRustLcovGenerator } from './independent-rust-lcov.js';
import { runAbortableCohort } from './parallel.js';
import type { RustOfflineProfile } from './rust-sandbox.js';

const EXPECTED_AGENTIC_QE_VERSION = '3.13.10';
const EXPECTED_AGENTIC_QE_TREE =
  '9e08c960bc1d8150d3814c2b4395762ec640afa5cdd8cbf216fafc255ed1d7a7';

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

export function createTaskQeCollector(options: Readonly<{
  taskId: string;
  runId: string;
  qeBindings: readonly TaskQeBinding[];
  snapshotParent: string;
  nodeExecutable: string;
  bwrapExecutable: string;
  packageRoot: string;
  mcpExecutable: string;
}>): TaskQeCollector {
  assertTaskQeBindings(options.qeBindings);
  const bindings = deepFreeze(options.qeBindings.map(cloneBinding));
  const runner = new SystemAgenticQeMcpRunner({
    nodeExecutable: options.nodeExecutable,
    aqeMcpExecutable: options.mcpExecutable,
    aqePackageRoot: options.packageRoot,
    bwrapExecutable: options.bwrapExecutable,
  });
  assertTaskQeRunnerIdentity(runner.identityEvidence());
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
}>): void {
  if (identity.package.version !== EXPECTED_AGENTIC_QE_VERSION
    || identity.package.treeSha256 !== EXPECTED_AGENTIC_QE_TREE) {
    throw new Error('HARNESS_TASK_QE_IDENTITY_MISMATCH');
  }
}

function cloneBinding(binding: TaskQeBinding): TaskQeBinding {
  return binding.profile === 'lcov-gap'
    ? {
        profile: 'lcov-gap', collector: 'rust-lcov',
        packageName: binding.packageName, testTarget: binding.testTarget,
      }
    : { profile: 'sast', collector: 'agentic-qe-sast' };
}

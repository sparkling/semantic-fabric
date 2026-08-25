// SPDX-License-Identifier: MIT

import { AgenticQeLcovGapEvidenceAdapter } from './agentic-qe-lcov.js';
import { SystemAgenticQeMcpRunner } from './agentic-qe-mcp-runner.js';
import { AgenticQeSastEvidenceAdapter } from './agentic-qe-sast.js';
import type { CandidateBuild } from './candidate.js';
import { IndependentRustLcovGenerator } from './independent-rust-lcov.js';
import { runAbortableCohort } from './parallel.js';
import { SECURE_HARNESS_CONFIG } from './config.js';
import type { RustOfflineProfile } from './rust-sandbox.js';

const EXPECTED_AGENTIC_QE_VERSION = '3.13.10';
const EXPECTED_AGENTIC_QE_TREE =
  '9e08c960bc1d8150d3814c2b4395762ec640afa5cdd8cbf216fafc255ed1d7a7';

export interface Issue8QeContext {
  readonly controlledRoot: string;
  readonly candidateRoot: string;
  readonly outputRoot: string;
  readonly rustProfile: RustOfflineProfile;
}

export function createIssue8QeCollector(options: Readonly<{
  taskId: string;
  runId: string;
  snapshotParent: string;
  nodeExecutable: string;
  bwrapExecutable: string;
  packageRoot: string;
  mcpExecutable: string;
}>) {
  const runner = new SystemAgenticQeMcpRunner({
    nodeExecutable: options.nodeExecutable,
    aqeMcpExecutable: options.mcpExecutable,
    aqePackageRoot: options.packageRoot,
    bwrapExecutable: options.bwrapExecutable,
  });
  const identity = runner.identityEvidence();
  if (identity.package.version !== EXPECTED_AGENTIC_QE_VERSION
    || identity.package.treeSha256 !== EXPECTED_AGENTIC_QE_TREE) {
    throw new Error('HARNESS_ISSUE_8_AGENTIC_QE_IDENTITY_MISMATCH');
  }
  const lcov = new AgenticQeLcovGapEvidenceAdapter({ runner });
  const sast = new AgenticQeSastEvidenceAdapter({
    runner,
    snapshotParent: options.snapshotParent,
  });

  return async (
    build: CandidateBuild,
    context: Issue8QeContext,
    signal?: AbortSignal,
  ) => await runAbortableCohort([
    async (cohortSignal) => {
      const generator = new IndependentRustLcovGenerator({
        config: SECURE_HARNESS_CONFIG,
        rustProfile: context.rustProfile,
        packageName: 'sf-conformance',
        testTarget: 'issue_8_binding_pruning',
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
    async (cohortSignal) => await sast.capture({
      taskId: options.taskId,
      runId: options.runId,
      candidateTree: build.candidate.tree,
      candidateRoot: context.candidateRoot,
    }, cohortSignal),
  ] as const, signal);
}

// SPDX-License-Identifier: MIT

import { APPROVED_NPM_REGISTRY, parseHarnessConfig } from './contracts.js';

export const SECURE_HARNESS_CONFIG = parseHarnessConfig({
  schemaVersion: 1,
  authority: 'development-only-no-promotion',
  approvedRegistry: APPROVED_NPM_REGISTRY,
  firstPartyOrigins: [
    'https://api.anthropic.com',
    'https://api.openai.com',
    'https://chatgpt.com',
    'https://claude.ai',
  ],
  allowedTools: ['read_file', 'write_file', 'apply_patch', 'git', 'node', 'npm', 'cargo', 'rustc'],
  requiredProtectedPaths: [
    '.mcp.json',
    'coding-harness/.claude-plugin/plugin.json',
    'coding-harness/.harness/controller-build.json',
    'coding-harness/.harness/manifest.json',
    'coding-harness/CLAUDE.md',
    'coding-harness/README.md',
    'coding-harness/config/issue-8-acceptance.json',
    'coding-harness/package.json',
    'coding-harness/package-lock.json',
    'coding-harness/scripts/deny-publish.mjs',
    'coding-harness/scripts/harden-build.mjs',
    'coding-harness/scripts/launch-issue-8.mjs',
    'coding-harness/src/agents/architect.ts',
    'coding-harness/src/agents/implementer.ts',
    'coding-harness/src/agents/reviewer.ts',
    'coding-harness/src/agents/test-writer.ts',
    'coding-harness/src/agentic-qe-lcov.ts',
    'coding-harness/src/agentic-qe-lcov-response.ts',
    'coding-harness/src/agentic-qe-mcp-identity.ts',
    'coding-harness/src/agentic-qe-mcp-protocol.ts',
    'coding-harness/src/agentic-qe-mcp-request.ts',
    'coding-harness/src/agentic-qe-mcp-runner.ts',
    'coding-harness/src/agentic-qe-sast.ts',
    'coding-harness/src/agentic-qe-sast-response.ts',
    'coding-harness/src/acceptance-task.ts',
    'coding-harness/src/acceptance-runner.ts',
    'coding-harness/src/candidate.ts',
    'coding-harness/src/candidate-types.ts',
    'coding-harness/src/candidate-gates.ts',
    'coding-harness/src/config.ts',
    'coding-harness/src/contracts.ts',
    'coding-harness/src/controller-attestation.ts',
    'coding-harness/src/controller-build.ts',
    'coding-harness/src/evidence.ts',
    'coding-harness/src/effective-config.ts',
    'coding-harness/src/effective-config-command.ts',
    'coding-harness/src/effective-config-diagnostics.ts',
    'coding-harness/src/effective-config-filesystem.ts',
    'coding-harness/src/effective-config-git.ts',
    'coding-harness/src/evaluator.ts',
    'coding-harness/src/frozen-cargo-lock.ts',
    'coding-harness/src/git-materialization.ts',
    'coding-harness/src/git-process.ts',
    'coding-harness/src/git-protected-boundary.ts',
    'coding-harness/src/git-worktrees.ts',
    'coding-harness/src/independent-rust-lcov.ts',
    'coding-harness/src/index.ts',
    'coding-harness/src/issue-8-driver.ts',
    'coding-harness/src/issue-8-native-session.ts',
    'coding-harness/src/issue-8-program.ts',
    'coding-harness/src/issue-8-qe.ts',
    'coding-harness/src/issue-8-system.ts',
    'coding-harness/src/kernel.ts',
    'coding-harness/src/manifest.ts',
    'coding-harness/src/model-context.ts',
    'coding-harness/src/model-controller.ts',
    'coding-harness/src/native-client.ts',
    'coding-harness/src/native-egress.ts',
    'coding-harness/src/native-filesystem.ts',
    'coding-harness/src/native-proxy-launcher.ts',
    'coding-harness/src/native-system-filesystem.ts',
    'coding-harness/src/models/environment.ts',
    'coding-harness/src/models/index.ts',
    'coding-harness/src/models/native-adapters.ts',
    'coding-harness/src/models/native-adapter-contracts.ts',
    'coding-harness/src/models/recovery.ts',
    'coding-harness/src/models/review.ts',
    'coding-harness/src/models/routing.ts',
    'coding-harness/src/models/types.ts',
    'coding-harness/src/native-process.ts',
    'coding-harness/src/native-process-contracts.ts',
    'coding-harness/src/native-runtime-ledger.ts',
    'coding-harness/src/native-runtime.ts',
    'coding-harness/src/network.ts',
    'coding-harness/src/policy.ts',
    'coding-harness/src/parallel.ts',
    'coding-harness/src/programme-acceptance.ts',
    'coding-harness/src/process.ts',
    'coding-harness/src/receipts.ts',
    'coding-harness/src/resource-boundary.ts',
    'coding-harness/src/repository-operations.ts',
    'coding-harness/src/repository-command-evidence.ts',
    'coding-harness/src/repository-command-runner.ts',
    'coding-harness/src/repository-options.ts',
    'coding-harness/src/rust-closure.ts',
    'coding-harness/src/rust-sandbox.ts',
    'coding-harness/src/sandbox.ts',
    'coding-harness/src/workspace.ts',
    'coding-harness/src/writable-overlays.ts',
    'coding-harness/tsconfig.json',
    'coding-harness/vitest.config.ts',
    'docs/adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md',
  ],
  environment: {
    allow: [
      'PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL', 'TERM', 'COLORTERM', 'CI',
      'GIT_CONFIG_NOSYSTEM', 'CARGO_HOME', 'CARGO_INCREMENTAL', 'CARGO_NET_OFFLINE',
      'CARGO_TARGET_DIR', 'SYSTEMROOT', 'COMSPEC', 'PATHEXT', 'WINDIR',
    ],
    denyExact: [
      'OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'OPENROUTER_API_KEY', 'REQUESTY_API_KEY',
      'HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY',
    ],
    denyPrefixes: ['OPENROUTER_', 'REQUESTY_'],
    denySuffixes: ['_BASE_URL', '_API_BASE', '_PROXY'],
  },
  limits: {
    maxTimeoutMs: 1_800_000,
    maxOutputBytes: 10_000_000,
    maxNewFileLines: 500,
    terminationGraceMs: 500,
  },
  evolution: {
    minimumTrainingTasks: 5,
    minimumSealedHoldouts: 5,
  },
});

export interface EvolutionEligibility {
  eligible: boolean;
  trainingTasks: number;
  sealedHoldouts: number;
  reason: string;
}

export function checkEvolutionEligibility(
  trainingTaskIds: readonly string[],
  sealedHoldoutIds: readonly string[],
): EvolutionEligibility {
  const training = new Set(trainingTaskIds);
  const holdouts = new Set(sealedHoldoutIds);
  const overlap = [...training].some((id) => holdouts.has(id));
  const opaque = [...training, ...holdouts].every((id) => /^[A-Za-z0-9_-]{8,128}$/.test(id));
  const eligible = training.size >= 5 && holdouts.size >= 5 && !overlap && opaque;
  return {
    eligible,
    trainingTasks: training.size,
    sealedHoldouts: holdouts.size,
    reason: eligible
      ? '5+5 distinct opaque task IDs present; evaluator eligibility still requires independent review'
      : 'requires at least 5 distinct training IDs and 5 distinct sealed holdout IDs with no overlap',
  };
}

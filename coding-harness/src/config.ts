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
    'coding-harness/.claude-plugin/plugin.json',
    'coding-harness/package.json',
    'coding-harness/package-lock.json',
    'coding-harness/src/config.ts',
    'coding-harness/src/contracts.ts',
    'coding-harness/src/index.ts',
    'coding-harness/src/policy.ts',
    'coding-harness/src/process.ts',
    'coding-harness/src/receipts.ts',
    'coding-harness/src/workspace.ts',
    'coding-harness/vitest.config.ts',
    'docs/adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md',
  ],
  environment: {
    allow: [
      'PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL', 'TERM', 'COLORTERM', 'CI',
      'GIT_CONFIG_NOSYSTEM', 'SYSTEMROOT', 'COMSPEC', 'PATHEXT', 'WINDIR',
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

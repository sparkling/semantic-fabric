// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { asClosedRecord, assertExactKeys } from '../src/contracts.js';
import { auditProjectMcpLauncherAdmission } from '../src/effective-config-command.js';
import { parseJsonWithoutDuplicateKeys } from '../src/strict-json.js';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(harnessRoot, '..');
const launcherSurfaces = [
  '.mcp.json',
  '.agents/config.toml',
] as const;
const manifest = parseJsonWithoutDuplicateKeys(readFileSync(
  resolve(harnessRoot, '.harness/manifest.json'), 'utf8',
), 'canonical harness manifest') as {
  authority: string;
  protectedPaths: string[];
  runtime: { claudeHost: string; codexHost: string };
  evolution: { eligible: boolean; suiteFile: string | null };
};
const runtimeBoundary = {
  product: 'rust-cargo-only', developmentEvidence: ['node', 'npm'],
};
const evolution = { enabled: false, eligible: false };
const expectedProfile = {
  schemaVersion: 1,
  authority: 'curated-project-bootstrap',
  name: 'semantic-fabric',
  languages: ['rust'],
  hasMcp: true,
  hasClaude: true,
  hasCodex: true,
  hasCi: true,
  buildCommands: ['cargo build --workspace --locked'],
  testCommands: ['cargo test --workspace --locked'],
  tokens: [
    'rust', 'cargo', 'workspace', 'semantic-data', 'mcp', 'ruflo',
    'metaharness', 'claude', 'codex', 'postgresql', 'ci',
  ],
  nativeHosts: ['claude-code', 'codex'],
  mcpAuthority: {
    scope: 'external-user-level', transport: 'stdio', projectLaunchers: 'default-denied',
  },
  runtimeBoundary,
  evolution,
};
const expectedPlan = {
  schemaVersion: 1,
  authority: 'curated-project-bootstrap',
  name: 'semantic-fabric-harness',
  hosts: ['claude-code', 'codex'],
  hostAuthentication: {
    'claude-code': 'native-anthropic-subscription', codex: 'native-openai-subscription',
  },
  template: 'vertical:coding',
  archetypeId: 'rust-crate-harness',
  archetypeSelection: 'explicit-project-override',
  agents: ['architect', 'implementer', 'reviewer', 'test-writer'],
  skills: ['plan-change'],
  commands: ['doctor', 'review-diff'],
  mcp: 'off',
  projectMcpLaunchers: 'default-denied',
  externalCoordination: { scope: 'external-user-level', transport: 'stdio' },
  policy: {
    defaultDeny: true, allowNetwork: false, allowShell: false, allowFileWrite: false,
    requireApprovalForDangerous: true, toolTimeoutMs: 30_000, maxToolCallsPerTurn: 8,
    auditLog: true,
  },
  riskProfile: 'shell gated, network gated, file-write read-scoped',
  suggestedCommands: [
    { command: 'cargo build --workspace --locked', trust: 'inferred', execution: 'disabled' },
    { command: 'cargo test --workspace --locked', trust: 'inferred', execution: 'disabled' },
  ],
  runtimeBoundary,
  evolution,
};

function parseExactBootstrap(serialized: string, expected: unknown, label: string): unknown {
  const parsed = parseJsonWithoutDuplicateKeys(serialized, label);
  assertExactValue(parsed, expected, label);
  return parsed;
}

function assertExactValue(actual: unknown, expected: unknown, label: string): void {
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual) || actual.length !== expected.length) {
      throw new TypeError(`${label} array shape is invalid`);
    }
    expected.forEach((entry, index) => assertExactValue(actual[index], entry, `${label}[${index}]`));
    return;
  }
  if (expected !== null && typeof expected === 'object') {
    const expectedRecord = expected as Record<string, unknown>;
    const actualRecord = asClosedRecord(actual, label);
    assertExactKeys(actualRecord, Object.keys(expectedRecord), label);
    for (const [key, value] of Object.entries(expectedRecord)) {
      assertExactValue(actualRecord[key], value, `${label}.${key}`);
    }
    return;
  }
  if (!Object.is(actual, expected)) throw new TypeError(`${label} value is invalid`);
}

describe('current tracked MCP launcher admission', () => {
  it('default-denies project launchers and protects every inspected surface', () => {
    const result = auditProjectMcpLauncherAdmission({
      mcpJson: readFileSync(resolve(repository, '.mcp.json'), 'utf8'),
      agentConfig: readFileSync(resolve(repository, '.agents/config.toml'), 'utf8'),
    });
    expect(result.status).toBe('PASS');
    expect(result.scope).toBe('tracked-project-mcp-launchers-only');
    expect(result.findings).toEqual([]);
    for (const path of [
      ...launcherSurfaces,
      'coding-harness/__tests__/effective-config-current-state.test.ts',
    ]) {
      expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths).toContain(path);
      expect(manifest.protectedPaths).toContain(path);
    }
  });

  it('keeps bootstrap facts aligned with the canonical dual-host manifest', () => {
    const profileBlob = readFileSync(resolve(repository, 'repo-profile.json'), 'utf8');
    const planBlob = readFileSync(resolve(repository, 'harness-plan.json'), 'utf8');
    const profile = parseExactBootstrap(profileBlob, expectedProfile, 'repo profile');
    const plan = parseExactBootstrap(planBlob, expectedPlan, 'harness plan');
    const hosts = [
      manifest.runtime.claudeHost.includes('host-claude-code') ? 'claude-code' : '',
      manifest.runtime.codexHost.includes('host-codex') ? 'codex' : '',
    ];

    expect(manifest.authority).toBe('development-only-no-promotion');
    expect(profile).toEqual(expectedProfile);
    expect(plan).toEqual(expectedPlan);
    expect(expectedProfile.nativeHosts).toEqual(hosts);
    expect(expectedPlan.hosts).toEqual(hosts);
    expect(manifest.evolution).toEqual({
      eligible: false, minimumTrainingTasks: 5, minimumSealedHoldouts: 5, suiteFile: null,
    });

    for (const [label, blob, expected] of [
      ['repo profile', profileBlob, expectedProfile],
      ['harness plan', planBlob, expectedPlan],
    ] as const) {
      const duplicate = blob.replace('"schemaVersion": 1', '"schemaVersion": 1, "schemaVersion": 1');
      expect(() => parseExactBootstrap(duplicate, expected, label)).toThrow(/duplicate JSON key/);
      for (const forbidden of ['provider', 'apiKey', 'openRouter']) {
        const extended = blob.replace(/\n}\s*$/, `,\n  "${forbidden}": "undeclared"\n}\n`);
        expect(() => parseExactBootstrap(extended, expected, label)).toThrow(/invalid keys/);
      }
    }
  });

  it.each([
    '[mcp_servers.ruflo]\ncommand = "npx"\n',
    '["mcp_servers".ruflo]\ncommand = "npx"\n',
    '["mcp\\u005fservers".ruflo]\ncommand = "npx"\n',
    "['mcp_servers'.ruflo]\ncommand = 'npx'\n",
    'mcp_servers.ruflo = { command = "npx" }\n',
  ])('rejects TOML MCP launcher form %#', (agentConfig) => {
    const result = auditProjectMcpLauncherAdmission({
      mcpJson: '{"mcpServers":{}}', agentConfig,
    });

    expect(result.status).toBe('FAIL');
    expect(result.findings).toContain('PROJECT_AGENT_CONFIG_CHANGED');
    expect(result.findings).toContain('PROJECT_AGENT_CONFIG_MCP_DECLARED');
  });

  it.each([
    '{"mcpServers":{"ruflo":{"command":"npx","args":["ruflo@3.38.20"]}}}',
    '{"mcpServers":{},"mcpServers":{"hidden":{"command":"node"}}}',
    '{"mcpServers":{},"unexpected":true}',
  ])('rejects nonempty, ambiguous, or extended MCP JSON form %#', (mcpJson) => {
    const result = auditProjectMcpLauncherAdmission({
      mcpJson,
      agentConfig: readFileSync(resolve(repository, '.agents/config.toml'), 'utf8'),
    });
    expect(result.status).toBe('FAIL');
    expect(result.findings.length).toBeGreaterThan(0);
  });
});

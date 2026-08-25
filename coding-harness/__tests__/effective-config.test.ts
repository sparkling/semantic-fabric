// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import { auditEffectiveConfiguration } from '../src/effective-config.js';

const cleanDiagnostic = (target: 'repository' | 'coding-harness', mcpEnabled: boolean) => ({
  target,
  mcpEnabled,
  worstSeverity: 'info',
  verdict: 'clean',
});

function scope(
  target: 'repository' | 'coding-harness',
  surfaces: unknown[] = [],
  mcpEnabled = false,
) {
  return {
    target,
    inventoryComplete: true,
    surfaces,
    upstreamDiagnostic: cleanDiagnostic(target, mcpEnabled),
  };
}

function surface(
  path: string,
  kind: 'mcp-json' | 'claude-settings' | 'hook-target' | 'skill' | 'executable-target',
  content: string,
  provenance: 'tracked-clean' | 'tracked-dirty' | 'untracked' | 'ignored' = 'tracked-clean',
) {
  return { path, kind, provenance, content };
}

describe('effective configuration audit', () => {
  it('passes a complete inventory with pinned launchers and consistent upstream visibility', () => {
    const result = auditEffectiveConfiguration({
      schemaVersion: 1,
      scopes: [
        scope('repository', [surface('.mcp.json', 'mcp-json', JSON.stringify({
          mcpServers: {
            ruflo: { command: 'npx', args: ['-y', '@claude-flow/cli@3.0.0', 'mcp', 'start'] },
          },
        }))], true),
        scope('coding-harness'),
      ],
    });

    expect(result.status).toBe('PASS');
    expect(result.complete).toBe(true);
    expect(result.servers).toMatchObject([
      { name: 'ruflo', packageSelector: '@claude-flow/cli@3.0.0', packagePinned: true },
    ]);
    expect(result.surfaces[0]?.digest).toMatch(/^[a-f0-9]{64}$/);
  });

  it('reports real floating and mutable surfaces while marking a blind scanner inconclusive', () => {
    const result = auditEffectiveConfiguration({
      schemaVersion: 1,
      scopes: [
        scope('repository', [
          surface('.mcp.json', 'mcp-json', JSON.stringify({
            mcpServers: {
              qe: { command: 'npx', args: ['-y', 'agentic-qe@latest'] },
              legacy: { command: 'node', args: ['semantic-fabric-harness/bin/cli.js', 'mcp', 'start'] },
            },
          }), 'tracked-dirty'),
          surface(
            'semantic-fabric-harness/bin/cli.js',
            'executable-target',
            '#!/usr/bin/env node',
            'ignored',
          ),
        ]),
        scope('coding-harness', [
          surface('coding-harness/.claude/skills/evolve/SKILL.md', 'skill', 'Run metaharness evolve.', 'untracked'),
        ]),
      ],
    });

    expect(result.status).toBe('INCONCLUSIVE');
    expect(result.complete).toBe(false);
    expect(result.findings.map(({ code }) => code)).toEqual(expect.arrayContaining([
      'FLOATING_NPX_SELECTOR',
      'MUTABLE_EXECUTION_SURFACE',
      'LOCAL_TARGET_NOT_TRACKED_CLEAN',
      'EVOLUTION_COMMAND_SURFACE',
      'UPSTREAM_BLIND_TO_MCP_SURFACE',
    ]));
  });

  it('treats a high-severity clean verdict as an inconclusive upstream contradiction', () => {
    const harnessScope = scope('coding-harness');
    harnessScope.upstreamDiagnostic = {
      ...harnessScope.upstreamDiagnostic,
      worstSeverity: 'high',
      verdict: 'clean',
    };
    const result = auditEffectiveConfiguration({
      schemaVersion: 1,
      scopes: [scope('repository'), harnessScope],
    });

    expect(result.status).toBe('INCONCLUSIVE');
    expect(result.findings.map(({ code }) => code)).toContain('UPSTREAM_VERDICT_CONFLICT');
  });

  it('fails deterministic server-name collisions when visibility is complete', () => {
    const result = auditEffectiveConfiguration({
      schemaVersion: 1,
      scopes: [
        scope('repository', [
          surface('.mcp.json', 'mcp-json', JSON.stringify({
            mcpServers: { shared: { command: 'npx', args: ['-y', 'first-server@1.0.0'] } },
          })),
          surface('.claude/settings.json', 'claude-settings', JSON.stringify({
            mcpServers: { shared: { command: 'npx', args: ['-y', 'second-server@1.0.0'] } },
          })),
        ], true),
        scope('coding-harness'),
      ],
    });

    expect(result.status).toBe('FAIL');
    expect(result.findings.map(({ code }) => code)).toContain('MCP_SERVER_NAME_COLLISION');
  });

  it('fails closed on dirty hook targets and executable evolution instructions', () => {
    const result = auditEffectiveConfiguration({
      schemaVersion: 1,
      scopes: [
        scope('repository', [
          surface('.claude/helpers/hook-handler.cjs', 'hook-target', 'export {};', 'tracked-dirty'),
        ]),
        scope('coding-harness', [
          surface('coding-harness/.claude/skills/evolve/SKILL.md', 'skill', 'Use metaharness evolve.', 'untracked'),
        ]),
      ],
    });

    expect(result.status).toBe('FAIL');
    expect(result.findings.map(({ code }) => code)).toEqual(expect.arrayContaining([
      'MUTABLE_EXECUTION_SURFACE',
      'EVOLUTION_COMMAND_SURFACE',
    ]));
  });

  it('marks malformed declared configuration inconclusive instead of clean', () => {
    const result = auditEffectiveConfiguration({
      schemaVersion: 1,
      scopes: [
        scope('repository', [surface('.mcp.json', 'mcp-json', '{invalid')]),
        scope('coding-harness'),
      ],
    });

    expect(result.status).toBe('INCONCLUSIVE');
    expect(result.findings.map(({ code }) => code)).toContain('CONFIG_PARSE_FAILURE');
  });
});

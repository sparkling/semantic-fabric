// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { auditProjectMcpLauncherAdmission } from '../src/effective-config-command.js';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(harnessRoot, '..');
const launcherSurfaces = [
  '.mcp.json',
  '.agents/config.toml',
] as const;

describe('current tracked MCP launcher admission', () => {
  it('default-denies project launchers and protects every inspected surface', () => {
    const result = auditProjectMcpLauncherAdmission({
      mcpJson: readFileSync(resolve(repository, '.mcp.json'), 'utf8'),
      agentConfig: readFileSync(resolve(repository, '.agents/config.toml'), 'utf8'),
    });
    const manifest = JSON.parse(readFileSync(
      resolve(harnessRoot, '.harness/manifest.json'), 'utf8',
    )) as { protectedPaths: string[] };

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

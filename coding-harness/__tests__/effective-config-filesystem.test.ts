// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import {
  existsSync,
  linkSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { auditEffectiveConfigurationFromFilesystem } from '../src/effective-config-filesystem.js';
import type { CapturedUpstreamDiagnostic } from '../src/effective-config-diagnostics.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('effective configuration filesystem collection', () => {
  it('reads and classifies real surfaces without executing discovered commands', () => {
    const root = repository();
    const marker = join(root, 'executed.marker');
    mkdirSync(join(root, 'legacy'), { recursive: true });
    writeFileSync(join(root, 'legacy/server.mjs'), `import { writeFileSync } from 'node:fs'; writeFileSync(${JSON.stringify(marker)}, 'bad');\n`);
    writeFileSync(join(root, '.gitignore'), 'legacy/\ncoding-harness/.claude/\n');
    writeFileSync(join(root, '.mcp.json'), JSON.stringify({
      mcpServers: {
        floating: { command: 'npx', args: ['-y', 'agentic-qe@latest'] },
        legacy: { command: 'node', args: ['legacy/server.mjs'] },
      },
    }));
    mkdirSync(join(root, 'coding-harness/.claude/skills/evolve'), { recursive: true });
    writeFileSync(
      join(root, 'coding-harness/.claude/skills/evolve/SKILL.md'),
      'Run metaharness evolve.\n',
    );
    git(root, ['add', '--', '.gitignore', '.mcp.json']);
    git(root, ['commit', '--quiet', '-m', 'configuration']);
    writeFileSync(
      join(root, '.mcp.json'),
      `${readFileSync(join(root, '.mcp.json'), 'utf8')}\n`,
    );

    const result = auditEffectiveConfigurationFromFilesystem({
      repositoryRoot: root,
      capturedDiagnostics: diagnostics(root, false),
    });

    expect(result.status).toBe('INCONCLUSIVE');
    expect(result.surfaces).toEqual(expect.arrayContaining([
      expect.objectContaining({ path: '.mcp.json', provenance: 'tracked-dirty' }),
      expect.objectContaining({ path: 'legacy/server.mjs', provenance: 'ignored' }),
      expect.objectContaining({
        path: 'coding-harness/.claude/skills/evolve/SKILL.md',
        provenance: 'ignored',
      }),
    ]));
    expect(result.findings.map(({ code }) => code)).toEqual(expect.arrayContaining([
      'FLOATING_NPX_SELECTOR',
      'LOCAL_TARGET_NOT_TRACKED_CLEAN',
      'EVOLUTION_COMMAND_SURFACE',
      'UPSTREAM_BLIND_TO_MCP_SURFACE',
    ]));
    expect(existsSync(marker)).toBe(false);
    expect(result.upstreamDiagnostics[0]?.rawDigest).toMatch(/^[a-f0-9]{64}$/);
    expect(result.repositorySnapshot).toMatchObject({ repositoryRoot: root });
    expect(result.repositorySnapshot?.digest).toMatch(/^[a-f0-9]{64}$/);
  });

  it('includes local settings, Codex/agent config, shell helpers, and every role', () => {
    const root = repository();
    mkdirSync(join(root, '.claude/helpers'), { recursive: true });
    mkdirSync(join(root, '.codex'), { recursive: true });
    mkdirSync(join(root, '.agents'), { recursive: true });
    writeFileSync(join(root, '.gitignore'), '.claude/settings.local.json\n.codex/\n');
    writeFileSync(join(root, '.claude/settings.local.json'), '{}\n');
    writeFileSync(join(root, '.codex/config.toml'), 'model = "local"\n');
    writeFileSync(join(root, '.agents/config.toml'), 'approval_policy = "never"\n');
    writeFileSync(join(root, '.claude/helpers/launch.sh'), '#!/bin/sh\nexit 0\n');
    writeFileSync(join(root, '.mcp.json'), JSON.stringify({
      mcpServers: { local: { command: 'node', args: ['.claude/helpers/launch.sh'] } },
    }));
    git(root, ['add', '--', '.gitignore', '.agents/config.toml', '.claude/helpers/launch.sh', '.mcp.json']);
    git(root, ['commit', '--quiet', '-m', 'configuration surfaces']);

    const result = auditEffectiveConfigurationFromFilesystem({
      repositoryRoot: root,
      capturedDiagnostics: diagnostics(root, true),
    });

    expect(result.surfaces).toEqual(expect.arrayContaining([
      expect.objectContaining({ path: '.claude/settings.local.json', provenance: 'ignored' }),
      expect.objectContaining({ path: '.codex/config.toml', kind: 'codex-config' }),
      expect.objectContaining({ path: '.agents/config.toml', kind: 'agent-config' }),
      expect.objectContaining({ path: '.claude/helpers/launch.sh', kind: 'hook-target' }),
      expect.objectContaining({ path: '.claude/helpers/launch.sh', kind: 'executable-target' }),
    ]));
  });

  it('marks symlinks, hardlinks, and invalid UTF-8 as incomplete without following them', () => {
    const root = repository();
    mkdirSync(join(root, '.claude/helpers'), { recursive: true });
    const regular = join(root, '.claude/helpers/regular.sh');
    writeFileSync(regular, '#!/bin/sh\n');
    linkSync(regular, join(root, '.claude/helpers/hardlink.sh'));
    symlinkSync('/etc/passwd', join(root, '.claude/helpers/symlink.sh'));
    writeFileSync(join(root, '.claude/helpers/invalid.bin'), Buffer.from([0xc3, 0x28]));

    const result = auditEffectiveConfigurationFromFilesystem({
      repositoryRoot: root,
      capturedDiagnostics: diagnostics(root, false),
    });

    expect(result.status).toBe('INCONCLUSIVE');
    expect(result.complete).toBe(false);
    expect(result.findings.map(({ code }) => code)).toContain('INCOMPLETE_SURFACE_INVENTORY');
    expect(result.surfaces.some(({ path }) => path.endsWith('symlink.sh'))).toBe(false);
  });
});

function repository(): string {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-config-fs-'));
  roots.push(root);
  mkdirSync(join(root, 'coding-harness'), { recursive: true });
  git(root, ['init', '--quiet']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  git(root, ['commit', '--quiet', '--allow-empty', '-m', 'baseline']);
  return root;
}

function diagnostics(root: string, mcpEnabled: boolean): CapturedUpstreamDiagnostic[] {
  return [
    diagnostic(root, 'repository', 'mcp-scan', mcpEnabled),
    diagnostic(root, 'repository', 'threat-model', mcpEnabled),
    diagnostic(root, 'coding-harness', 'mcp-scan', false),
    diagnostic(root, 'coding-harness', 'threat-model', false),
  ];
}

function diagnostic(
  root: string,
  target: CapturedUpstreamDiagnostic['target'],
  tool: CapturedUpstreamDiagnostic['tool'],
  mcpEnabled: boolean,
): CapturedUpstreamDiagnostic {
  const dir = target === 'repository' ? root : join(root, 'coding-harness');
  const data = tool === 'mcp-scan'
    ? { dir, mcpEnabled, worst: 'info' }
    : { dir, mcpInUse: mcpEnabled, worst: 'info', verdict: 'clean' };
  return {
    target,
    tool,
    toolVersion: 'ruflo-metaharness@0.1.1/metaharness@0.3.0',
    exitCode: 0,
    rawOutput: JSON.stringify({ success: true, data, degraded: false, exitCode: 0 }),
  };
}

function git(cwd: string, args: string[]): void {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
}

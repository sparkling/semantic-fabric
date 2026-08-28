// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import {
  PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
  PROGRAMME_V5_CONTROLLER_REQUIRED_PATHS,
  controllerExecutionPaths,
} from '../src/controller-attestation.js';
import {
  normalizeAcceptanceTaskPath,
  parseHarnessManifest,
  selectAcceptanceTaskPath,
} from '../src/manifest.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(readFileSync(resolve(root, '.harness/manifest.json'), 'utf8')) as unknown;

interface WorkflowRunScript {
  line: number;
  script: string;
}

interface CargoCommand {
  line: number;
  command: string;
  words: string[];
  subcommand: string;
}

const DEPENDENCY_RESOLVING_CARGO_SUBCOMMANDS = new Set([
  'add',
  'audit',
  'bench',
  'build',
  'check',
  'clippy',
  'doc',
  'fetch',
  'fix',
  'generate-lockfile',
  'install',
  'metadata',
  'package',
  'publish',
  'remove',
  'run',
  'rustc',
  'rustdoc',
  'test',
  'tree',
  'update',
  'vendor',
]);
const NON_RESOLVING_CARGO_SUBCOMMANDS = new Set(['fmt']);
const CARGO_GLOBAL_OPTIONS_WITH_VALUE = new Set(['--color', '--config', '-C', '-Z']);

function workflowRunScripts(source: string): WorkflowRunScript[] {
  const lines = source.split(/\r?\n/);
  const scripts: WorkflowRunScript[] = [];

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] ?? '';
    const match = /^(\s*)(?:-\s+)?run:\s*(.*?)\s*$/.exec(line);
    if (!match) continue;

    const value = match[2] ?? '';
    const blockHeader = /^[|>](?:[+-][1-9]?|[1-9][+-]?)?\s*(?:#.*)?$/.exec(value);
    if (blockHeader) {
      const keyIndent = (match[1] ?? '').length;
      const blockLines: string[] = [];
      let blockIndex = index + 1;
      while (blockIndex < lines.length) {
        const blockLine = lines[blockIndex] ?? '';
        const indentation = /^\s*/.exec(blockLine)?.[0].length ?? 0;
        if (blockLine.trim() !== '' && indentation <= keyIndent) break;
        blockLines.push(blockLine);
        blockIndex += 1;
      }
      scripts.push({
        line: index + 1,
        script: value.startsWith('>') ? blockLines.join(' ') : blockLines.join('\n'),
      });
      index = blockIndex - 1;
      continue;
    }

    if (value.startsWith('|') || value.startsWith('>') || value === '') {
      throw new Error(`CI_WORKFLOW_RUN_SCALAR_UNSUPPORTED at line ${index + 1}: ${line.trim()}`);
    }
    scripts.push({ line: index + 1, script: value });
  }

  return scripts;
}

function shellWords(command: string): string[] {
  return (command.match(/"(?:\\.|[^"])*"|'[^']*'|[^\s]+/g) ?? []).map((word) => {
    if ((word.startsWith('"') && word.endsWith('"'))
      || (word.startsWith("'") && word.endsWith("'"))) {
      return word.slice(1, -1);
    }
    return word;
  });
}

function cargoSubcommand(words: string[], line: number, command: string): string {
  let index = 1;
  if (words[index]?.startsWith('+')) index += 1;

  while (index < words.length) {
    const word = words[index] ?? '';
    const option = word.split('=', 1)[0] ?? word;
    if (!word.startsWith('-')) return word;
    if (CARGO_GLOBAL_OPTIONS_WITH_VALUE.has(option) && !word.includes('=')) index += 1;
    index += 1;
  }

  throw new Error(`CI_WORKFLOW_CARGO_SUBCOMMAND_MISSING at line ${line}: ${command}`);
}

function workflowCargoCommands(source: string): CargoCommand[] {
  return workflowRunScripts(source).flatMap(({ line, script }) => {
    const logicalScript = script.replace(/\\\r?\n[ \t]*/g, ' ');
    return logicalScript.split(/\r?\n/).flatMap((physicalLine) => {
      const trimmed = physicalLine.trim();
      if (trimmed === '' || trimmed.startsWith('#')) return [];

      const cargoMentions = [...trimmed.matchAll(/(?:^|[\s;&|])cargo(?=\s|$)/g)].length;
      if (cargoMentions === 0) return [];
      if (/\s#/.test(trimmed)) {
        throw new Error(`CI_WORKFLOW_CARGO_COMMENT_UNSUPPORTED at line ${line}: ${trimmed}`);
      }

      const commands = trimmed
        .split(/\s*(?:&&|\|\||[;|])\s*/)
        .filter((command) => command.startsWith('cargo '));
      if (commands.length !== cargoMentions) {
        throw new Error(
          `CI_WORKFLOW_CARGO_COMMAND_SHAPE_UNSUPPORTED at line ${line}: ${trimmed}; `
          + 'Cargo invocations must be direct shell commands separated by a newline, &&, ||, ;, or |',
        );
      }

      return commands.map((command) => {
        const words = shellWords(command);
        return { line, command, words, subcommand: cargoSubcommand(words, line, command) };
      });
    });
  });
}

describe('canonical harness manifest', () => {
  it('matches the protected runtime config and exposes the actual coordination surface', () => {
    const parsed = parseHarnessManifest(manifest, SECURE_HARNESS_CONFIG);
    expect(parsed.coordinationSurface).toBe('.mcp.json');
    expect(parsed.diagnostics.programmeAcceptanceThreshold).toBe(98);
    expect(parsed.evolution).toMatchObject({ eligible: false, suiteFile: null });
  });

  it('rejects a reduced protected set or synthetic clean diagnostic', () => {
    expect(() => parseHarnessManifest({
      ...(manifest as object),
      protectedPaths: ['coding-harness/package.json'],
    }, SECURE_HARNESS_CONFIG)).toThrow('HARNESS_MANIFEST_PROTECTED_PATHS_MISMATCH');
    expect(() => parseHarnessManifest({
      ...(manifest as object),
      diagnostics: {
        programmeAcceptanceThreshold: 98,
        upstreamScores: 'diagnostic-only',
        blindSurfaceOutcome: 'CLEAN',
      },
    }, SECURE_HARNESS_CONFIG)).toThrow(/diagnostic gates/);
  });

  it('selects one normalized manifest-bound acceptance task', () => {
    const parsed = parseHarnessManifest(manifest, SECURE_HARNESS_CONFIG);
    const taskPath = 'coding-harness/config/issue-8-acceptance.json';
    const programmeV5TaskPath = 'coding-harness/config/programme-v5-acceptance.json';

    expect(normalizeAcceptanceTaskPath(taskPath)).toBe(taskPath);
    expect(selectAcceptanceTaskPath(parsed, taskPath)).toBe(taskPath);
    expect(normalizeAcceptanceTaskPath(programmeV5TaskPath)).toBe(programmeV5TaskPath);
    expect(selectAcceptanceTaskPath(parsed, programmeV5TaskPath)).toBe(programmeV5TaskPath);
    for (const invalid of [
      '',
      '/coding-harness/config/issue-8-acceptance.json',
      './coding-harness/config/issue-8-acceptance.json',
      'coding-harness/config/../config/issue-8-acceptance.json',
      'coding-harness\\config\\issue-8-acceptance.json',
      'coding-harness/config/issue-8-acceptance.json\0',
      'coding-harness/config/Issue-8-acceptance.json',
      'coding-harness/config/issué-8-acceptance.json',
      'coding-harness/config/issue-8-acceptance.json/',
      'coding-harness/config/issue-8.json',
      'docs/issue-8-acceptance.json',
    ]) {
      expect(() => normalizeAcceptanceTaskPath(invalid)).toThrow();
    }
    expect(() => selectAcceptanceTaskPath(
      parsed,
      'coding-harness/config/m0-reproducibility-acceptance.json',
    )).toThrow('HARNESS_MANIFEST_TASK_NOT_LISTED');
  });

  it('requires every acceptance task to be a unique protected controller input', () => {
    const input = structuredClone(manifest as Record<string, unknown>) as Record<string, any>;
    input.acceptanceTasks = [
      ...input.acceptanceTasks,
      'coding-harness/config/m0-reproducibility-acceptance.json',
    ];
    expect(() => parseHarnessManifest(input, SECURE_HARNESS_CONFIG))
      .toThrow('HARNESS_MANIFEST_TASK_NOT_PROTECTED');

    input.acceptanceTasks = [input.acceptanceTasks[0], input.acceptanceTasks[0]];
    expect(() => parseHarnessManifest(input, SECURE_HARNESS_CONFIG)).toThrow(/duplicates/);
  });

  it('fails closed when any trusted programme-v5 execution source is omitted', () => {
    const paths = SECURE_HARNESS_CONFIG.requiredProtectedPaths;
    expect(controllerExecutionPaths(paths, PROGRAMME_V5_ACCEPTANCE_TASK_PATH)).toEqual(
      expect.arrayContaining([...PROGRAMME_V5_CONTROLLER_REQUIRED_PATHS]),
    );
    for (const required of PROGRAMME_V5_CONTROLLER_REQUIRED_PATHS) {
      expect(() => controllerExecutionPaths(
        paths.filter((path) => path !== required),
        PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
      )).toThrow('HARNESS_CONTROLLER_EXECUTION_MANIFEST_INCOMPLETE');
    }
  });

  it('protects every tracked ADR, Cargo manifest, CI workflow, and publication control', () => {
    const repository = resolve(root, '..');
    const listed = spawnSync('git', ['-C', repository, 'ls-files', '-z'], {
      encoding: 'utf8',
    });
    expect(listed.status, listed.stderr).toBe(0);
    const paths = listed.stdout.split('\0').filter(Boolean);
    const governed = paths.filter((path) =>
      path === 'Cargo.lock'
      || path === 'Cargo.toml'
      || path.endsWith('/Cargo.toml')
      || (path.startsWith('docs/adr/') && path.endsWith('.md'))
      || path.startsWith('.github/workflows/'));
    governed.push(
      '.gitignore', 'AGENTS.md', 'CLAUDE.md', 'LICENSE-APACHE', 'LICENSE-MIT',
      'README.md', 'rust-toolchain.toml',
    );
    expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths).toEqual(
      expect.arrayContaining(governed),
    );
  });

  it('tracks, does not ignore, and protects the root Cargo.lock', () => {
    const repository = resolve(root, '..');
    const tracked = spawnSync(
      'git',
      ['-C', repository, 'ls-files', '--error-unmatch', '--', 'Cargo.lock'],
      { encoding: 'utf8' },
    );
    expect(
      tracked.status,
      `Cargo.lock must be tracked by git: ${tracked.stderr.trim() || 'not present in the index'}`,
    ).toBe(0);

    const ignored = spawnSync(
      'git',
      ['-C', repository, 'check-ignore', '--no-index', '--quiet', '--', 'Cargo.lock'],
      { encoding: 'utf8' },
    );
    expect(
      ignored.status,
      `Cargo.lock must not match a git ignore rule: ${ignored.stderr.trim() || 'matched an ignore rule'}`,
    ).toBe(1);
    expect(
      SECURE_HARNESS_CONFIG.requiredProtectedPaths,
      'Cargo.lock must be inside the canonical protected boundary',
    ).toContain('Cargo.lock');
  });

  it('locks every dependency-resolving Cargo command and pins cargo-audit exactly', () => {
    const repository = resolve(root, '..');
    const workflowPath = resolve(repository, '.github/workflows/ci.yml');
    const commands = workflowCargoCommands(readFileSync(workflowPath, 'utf8'));
    const unclassified = commands.filter(({ subcommand }) =>
      !DEPENDENCY_RESOLVING_CARGO_SUBCOMMANDS.has(subcommand)
      && !NON_RESOLVING_CARGO_SUBCOMMANDS.has(subcommand));
    expect(
      unclassified.map(({ line, command }) => `line ${line}: ${command}`),
      'Every Cargo command in ci.yml must be explicitly classified as dependency-resolving or lock-exempt',
    ).toEqual([]);

    const resolving = commands.filter(({ subcommand }) =>
      DEPENDENCY_RESOLVING_CARGO_SUBCOMMANDS.has(subcommand));
    const unlocked = resolving.filter(({ words }) => {
      const separator = words.indexOf('--');
      const cargoArguments = separator === -1 ? words : words.slice(0, separator);
      return !cargoArguments.includes('--locked');
    });
    expect(
      unlocked.map(({ line, command }) => `line ${line}: ${command}`),
      'Every dependency-resolving Cargo command in ci.yml must pass --locked before any `--` separator',
    ).toEqual([]);

    const auditInstallers = commands.filter(({ subcommand, words }) =>
      subcommand === 'install' && words.includes('cargo-audit'));
    expect(
      auditInstallers.map(({ line, command }) => `line ${line}: ${command}`),
      'ci.yml must contain exactly one cargo-audit installation command',
    ).toHaveLength(1);
    const auditVersions = auditInstallers.flatMap(({ words }) => words.flatMap((word, index) => {
      if (word === '--version') return [words[index + 1] ?? '<missing>'];
      if (word.startsWith('--version=')) return [word.slice('--version='.length)];
      return [];
    }));
    expect(
      auditVersions,
      'cargo-audit must be pinned with exactly one --version value of 0.22.2',
    ).toEqual(['0.22.2']);
  });
});

// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
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
const mcpPolicy = JSON.parse(
  readFileSync(resolve(root, '../.harness/mcp-policy.json'), 'utf8'),
) as unknown;

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
const EXACT_NODE_VERSION = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/;
const PINNED_ACTION = /^[^@\s]+@[0-9a-f]{40}$/;
const PINNED_IMAGE = /@sha256:[0-9a-f]{64}$/;
const UNIXODBC_VERSION = '2.3.12-1ubuntu0.24.04.1';

function yamlScalar(value: string): string {
  const scalar = value.replace(/\s+#.*$/, '').trim();
  if ((scalar.startsWith("'") && scalar.endsWith("'"))
    || (scalar.startsWith('"') && scalar.endsWith('"'))) {
    return scalar.slice(1, -1);
  }
  return scalar;
}

function workflowSources(repository: string): ReadonlyArray<Readonly<{
  path: string;
  source: string;
}>> {
  const directory = resolve(repository, '.github/workflows');
  return readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && /\.ya?ml$/.test(entry.name))
    .map((entry) => ({
      path: `.github/workflows/${entry.name}`,
      source: readFileSync(resolve(directory, entry.name), 'utf8'),
    }))
    .sort((left, right) => left.path.localeCompare(right.path));
}

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
  it('keeps the MCP surface development-only and default-deny', () => {
    expect(mcpPolicy).toEqual({
      schemaVersion: 1,
      authority: 'development-only-no-promotion',
      architectureDecision: 'docs/adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md',
      defaultDeny: true,
      allowNetwork: false,
      allowShell: false,
      allowFileWrite: false,
      requireApprovalForDangerous: true,
      toolTimeoutMs: 30_000,
      maxToolCallsPerTurn: 8,
      auditLog: true,
    });

    const scanner = resolve(root, 'node_modules/metaharness/dist/harness-bin.js');
    const repository = resolve(root, '..');
    const scan = spawnSync(process.execPath, [scanner, 'mcp-scan', repository, '--json'], {
      encoding: 'utf8',
    });
    expect(scan.status, scan.stderr).toBe(0);
    expect(JSON.parse(scan.stdout)).toMatchObject({
      dir: repository,
      mcpEnabled: true,
      findings: [{ id: 'clean', severity: 'info' }],
      worst: 'info',
    });
  });

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

  it('installs and invokes readiness tooling only from the committed npm lock', () => {
    const workflow = readFileSync(resolve(root, '../.github/workflows/ci.yml'), 'utf8');
    const lines = workflow.split(/\r?\n/);
    const start = lines.indexOf('  readiness:');
    const end = lines.findIndex((line, index) => index > start && /^  [\w-]+:\s*$/.test(line));
    expect(start, 'ci.yml must define a readiness job').toBeGreaterThan(-1);
    const readiness = lines.slice(start, end === -1 ? undefined : end).join('\n');

    expect(readiness).not.toMatch(/\b(?:npx|npm\s+exec)\b/);
    expect(readiness).not.toMatch(/(?:OPENROUTER|OPENAI|ANTHROPIC).*API_KEY/);
    expect(readiness.match(
      /npm --prefix coding-harness ci --ignore-scripts --include=dev --omit=optional/g,
    )).toHaveLength(1);
    expect(readiness.match(/\.\/coding-harness\/node_modules\/\.bin\/metaharness\b/g))
      .toHaveLength(2);
    expect(readiness.match(/\.\/coding-harness\/node_modules\/\.bin\/harness\b/g))
      .toHaveLength(2);
  });

  it('pins every workflow action, hosted runner, service image, Node runtime, and apt tool', () => {
    const repository = resolve(root, '..');
    const violations: string[] = [];
    let unixOdbcInstallers = 0;

    for (const workflow of workflowSources(repository)) {
      const lines = workflow.source.split(/\r?\n/);
      const matrixNodes = lines.flatMap((line) => {
        const match = /^\s*(?:-\s+)?node:\s*(.+?)\s*$/.exec(line);
        return match ? [yamlScalar(match[1] ?? '')] : [];
      });
      let setupNodeActions = 0;
      let nodeVersionInputs = 0;

      for (let index = 0; index < lines.length; index += 1) {
        const line = lines[index] ?? '';
        const location = `${workflow.path}:${index + 1}`;
        const action = /^\s*(?:-\s+)?uses:\s*(.+?)\s*$/.exec(line);
        if (action) {
          const value = yamlScalar(action[1] ?? '');
          if (!PINNED_ACTION.test(value)) {
            violations.push(`${location}: action ref must end in a 40-hex commit SHA`);
          }
          if (value.startsWith('actions/setup-node@')) setupNodeActions += 1;
        }

        const runner = /^\s*runs-on:\s*(.+?)\s*$/.exec(line);
        const runnerValue = runner ? yamlScalar(runner[1] ?? '') : '';
        if (runner && !runnerValue.includes('self-hosted') && /(?:^|[\s[,'"])[^\s\],'"]+-latest(?:$|[\s\],'"])/.test(runnerValue)) {
          violations.push(`${location}: hosted runner family must not use a *-latest label`);
        }

        const image = /^\s*image:\s*(.+?)\s*$/.exec(line);
        if (image && !PINNED_IMAGE.test(yamlScalar(image[1] ?? ''))) {
          violations.push(`${location}: service image must end in a sha256 digest`);
        }

        const node = /^\s*node-version:\s*(.+?)\s*$/.exec(line);
        if (node) {
          nodeVersionInputs += 1;
          const value = yamlScalar(node[1] ?? '');
          const exactMatrix = value === '${{ matrix.node }}'
            && matrixNodes.length > 0 && matrixNodes.every((entry) => EXACT_NODE_VERSION.test(entry));
          if (!EXACT_NODE_VERSION.test(value) && !exactMatrix) {
            violations.push(`${location}: Node version must be exact major.minor.patch`);
          }
        }
      }
      if (setupNodeActions !== nodeVersionInputs) {
        violations.push(`${workflow.path}: every setup-node action must bind one exact node-version`);
      }

      for (const { line, script } of workflowRunScripts(workflow.source)) {
        if (!/\bunixodbc-dev(?:=|\s|$)/.test(script)) continue;
        unixOdbcInstallers += 1;
        const words = shellWords(script);
        const packages = words.filter((word) => word === 'unixodbc-dev'
          || word.startsWith('unixodbc-dev='));
        if (!words.includes('--no-install-recommends')
          || packages.length !== 1 || packages[0] !== `unixodbc-dev=${UNIXODBC_VERSION}`) {
          violations.push(
            `${workflow.path}:${line}: unixodbc-dev must use the exact reviewed version and no recommends`,
          );
        }
      }
    }

    expect(violations, 'Every workflow supply-chain input must be immutable').toEqual([]);
    expect(unixOdbcInstallers, 'Exactly one workflow must install unixodbc-dev').toBe(1);
  });
});

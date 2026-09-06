// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import {
  allGates,
  formatOutputs,
  parseDiffOutput,
  readChangedChanges,
  readQualificationPaths,
  selectForChanges,
  selectForPaths,
  selectFromGit,
} from '../scripts/select-ci-gates.mjs';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(harnessRoot, '..');
const manifestPath = resolve(harnessRoot, '.harness/manifest.json');
const qualificationInventoryPath = resolve(
  repository, 'tests/postgresql/observation-qualification-inputs-v1.tsv',
);
const temporaryDirectories: string[] = [];

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function gates(
  rust: boolean,
  coding_harness: boolean,
  supervisor = false,
  acl_replay = false,
  pg_observation = false,
) {
  return { rust, coding_harness, supervisor, acl_replay, pg_observation };
}

describe('fail-closed CI impact selector', () => {
  it.each([
    [['crates/sf-core/src/lib.rs'], gates(true, false)],
    [['docs/adr/ADR-0049-exact-recursive-property-path-fixed-points.md'], gates(false, true)],
    [['coding-harness/src/model-controller.ts'], gates(false, true)],
    [['coding-harness/supervisor-service/src/index.ts'], gates(false, true, true, true)],
    [['tests/capabilities/catalog-v1.json'], gates(true, true)],
    [['docs/capability-matrix.md'], gates(true, true)],
    [['Cargo.lock'], gates(true, true, false, false, true)],
    [['rust-toolchain.toml'], gates(true, true, false, false, true)],
    [['crates/new-provider/Cargo.toml'], gates(true, true)],
    [['crates/sf-conformance/Cargo.toml'], gates(true, true, false, false, true)],
    [['README.md'], gates(true, true)],
    [['coding-harness/package.json'], gates(true, true, false, false, true)],
    [['docs/adr/ADR-0051-postgresql-16-public-observed-schema-profile.md'],
      gates(true, true, false, false, true)],
    [['coding-harness/src/postgres-observation-qualification.ts'],
      gates(true, true, false, false, true)],
    [['crates/sf-sql/src/introspect/postgres/observation/catalog_sql.rs'],
      gates(true, true, false, false, true)],
    [['crates/sf-sql/src/introspect/postgres/legacy_sql.rs'],
      gates(true, true, false, false, true)],
    [['crates/sf-sql/src/introspect/postgres.rs'],
      gates(true, true, false, false, true)],
    [['tests/postgresql/observation-qualification-protocol-v1.json'],
      gates(true, true, false, false, true)],
    [['tests/postgresql/postgresql-16-observation-qualification-receipt-pair-v1.json'],
      gates(true, true, false, false, true)],
    [['.github/workflows/ci.yml'], allGates()],
    [['coding-harness/scripts/select-ci-gates.mjs'], allGates()],
  ] as const)('classifies %j conservatively', (paths, expected) => {
    expect(selectForPaths([...paths])).toEqual(expected);
  });

  it('uses monotonic OR and preserves gate implications', () => {
    expect(selectForPaths([
      'crates/sf-core/src/lib.rs',
      'coding-harness/src/model-controller.ts',
    ])).toEqual(gates(true, true));
    expect(selectForPaths([
      'docs/adr/ADR-0048-rust-production-and-node-evidence-runtime-boundary.md',
      'coding-harness/supervisor-service/README.md',
    ])).toEqual(gates(false, true, true, true));
    expect(selectForPaths([
      'coding-harness/src/postgres-observation-qualification-cli.ts',
    ])).toEqual(gates(true, true, false, false, true));
    expect(selectForPaths([
      'crates/sf-core/src/lib.rs', 'unclassified/new-surface.txt',
    ])).toEqual(allGates());
  });

  it('runs the capture closure gate for reachable source membership changes only', () => {
    expect(selectForChanges([
      { status: 'M', path: 'crates/sf-sparql/src/lib.rs' },
    ])).toEqual(gates(true, false));
    for (const packageName of ['sf-bench', 'sf-core', 'sf-mapping', 'sf-sparql', 'sf-sql']) {
      for (const status of ['A', 'D', 'T'] as const) {
        expect(selectForChanges([
          { status, path: `crates/${packageName}/src/new-module.rs` },
        ])).toEqual(gates(true, true));
        expect(selectForChanges([
          { status, path: `crates/${packageName}/build.rs` },
        ])).toEqual(gates(true, true));
      }
    }
    expect(selectForChanges([
      { status: 'A', path: 'crates/unreachable/src/new-module.rs' },
    ])).toEqual(gates(true, false));
    expect(selectForChanges([
      { status: 'R', path: 'crates/sf-sparql/src/new-module.rs' },
    ])).toEqual(allGates());
  });

  it.each([
    [],
    ['../Cargo.toml'],
    ['/Cargo.toml'],
    ['coding-harness-evil/src/index.ts'],
    ['crates//sf-core/lib.rs'],
    ['crates/./sf-core/lib.rs'],
    ['crates/sf-core/../sf-sql/lib.rs'],
    ['crates\\sf-core\\lib.rs'],
    ['docs/adr/bad\npath.md'],
    ['docs/cafe\u0301.md'],
    ['Cargo.toml', 'Cargo.toml'],
    ['x'.repeat(4_097)],
  ])('selects every gate for empty, malformed, or unknown paths: %j', (paths) => {
    expect(selectForPaths(paths)).toEqual(allGates());
  });

  it('strictly decodes bounded NUL-delimited Git output', () => {
    expect(parseDiffOutput(Buffer.from('D\0old.rs\0A\0new.rs\0'))).toEqual([
      { status: 'D', path: 'old.rs' },
      { status: 'A', path: 'new.rs' },
    ]);
    for (const bytes of [
      Buffer.alloc(0),
      Buffer.from('unterminated'),
      Buffer.from('M\0missing-path-pair\0A\0'),
      Buffer.from('R100\0old.rs\0new.rs\0'),
      Buffer.from([0xff, 0x00]),
      Buffer.alloc(1_048_577, 0x61),
    ]) {
      expect(() => parseDiffOutput(bytes)).toThrow(/CI_SELECTOR_DIFF_INVALID|encoded data/);
    }
  });

  it('emits only fixed boolean outputs', () => {
    expect(formatOutputs(gates(true, false, true, true, false))).toBe(
      'rust=true\ncoding_harness=false\nsupervisor=true\nacl_replay=true\npg_observation=false\n',
    );
    expect(() => formatOutputs({ rust: 'true' })).toThrow(/OUTPUT_INVALID/);
  });

  it('strictly parses the self-bound qualification input inventory', () => {
    const directory = mkdtempSync(join(tmpdir(), 'semantic-fabric-ci-inventory-'));
    temporaryDirectories.push(directory);
    const inventory = join(directory, 'inputs.tsv');
    const self = 'tests/postgresql/observation-qualification-inputs-v1.tsv';
    writeFileSync(inventory, `category\tpath\nprotocol\t${self}\nqueries\tcustom/query.sql\n`);
    expect(readQualificationPaths(inventory)).toEqual([self, 'custom/query.sql']);
    expect(selectForPaths(['custom/query.sql'], [], readQualificationPaths(inventory)))
      .toEqual(gates(true, true, false, false, true));
    for (const invalid of [
      '',
      `category\tpath\nunknown\t${self}\n`,
      'category\tpath\nqueries\tcustom/query.sql\n',
      `category\tpath\r\nprotocol\t${self}\r\n`,
      `category\tpath\nprotocol\t${self}\nprotocol\t${self}\n`,
      `category\tpath\nqueries\tcustom/query.sql\nprotocol\t${self}\n`,
    ]) {
      writeFileSync(inventory, invalid);
      expect(() => readQualificationPaths(inventory)).toThrow(/QUALIFICATION_INVENTORY_INVALID|PATH_INVALID/);
    }
  });

  it('selects the harness for every protected manifest path', () => {
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as {
      protectedPaths: string[];
    };
    expect(manifest.protectedPaths.length).toBeGreaterThan(100);
    for (const path of manifest.protectedPaths) {
      expect(selectForPaths([path], manifest.protectedPaths).coding_harness, path).toBe(true);
    }
  });

  it('selects PostgreSQL qualification for every self-bound input path', () => {
    const qualificationPaths = readQualificationPaths(qualificationInventoryPath);
    expect(qualificationPaths.length).toBeGreaterThan(90);
    for (const path of qualificationPaths) {
      expect(selectForPaths([path], [], qualificationPaths), path).toMatchObject({
        rust: true, coding_harness: true, pg_observation: true,
      });
    }
  });

  it('selects supervisor and ACL replay for every tracked supervisor path', () => {
    const tracked = git(repository, ['ls-files', 'coding-harness/supervisor-service/'])
      .stdout.trim().split('\n').filter(Boolean);
    expect(tracked.length).toBeGreaterThan(100);
    for (const path of tracked) {
      expect(selectForPaths([path])).toMatchObject({
        coding_harness: true, supervisor: true, acl_replay: true,
      });
    }
  });

  it('uses exact commits, exposes rename endpoints, and fails closed on Git errors', () => {
    const fixture = gitFixture();
    const base = commit(fixture, 'base');
    mkdirSync(join(fixture, 'crates/sf-core/src'), { recursive: true });
    writeFileSync(join(fixture, 'crates/sf-core/src/old.rs'), 'old\n');
    git(fixture, ['add', '.']);
    const withOld = commit(fixture, 'old path');
    git(fixture, ['mv', 'crates/sf-core/src/old.rs', 'crates/sf-core/src/new.rs']);
    const renamed = commit(fixture, 'rename');
    expect(readChangedChanges({ repository: fixture, baseSha: withOld, headSha: renamed })
      .sort((left, right) => left.path.localeCompare(right.path)))
      .toEqual([
        { status: 'A', path: 'crates/sf-core/src/new.rs' },
        { status: 'D', path: 'crates/sf-core/src/old.rs' },
      ]);

    const manifest = join(fixture, 'manifest.json');
    const qualificationInventory = join(fixture, 'qualification-inputs.tsv');
    writeFileSync(manifest, '{"protectedPaths":["Cargo.toml"]}\n');
    writeFileSync(qualificationInventory,
      'category\tpath\nprotocol\ttests/postgresql/observation-qualification-inputs-v1.tsv\n');
    expect(selectFromGit({
      eventName: 'pull_request', repository: fixture,
      baseSha: withOld, headSha: renamed, manifestPath: manifest,
      qualificationInventoryPath: qualificationInventory,
    })).toEqual(gates(true, true));
    expect(selectFromGit({
      eventName: 'push', repository: fixture,
      baseSha: withOld, headSha: renamed, manifestPath: manifest,
      qualificationInventoryPath: qualificationInventory,
    })).toEqual(allGates());
    expect(selectFromGit({
      eventName: 'pull_request', repository: fixture,
      baseSha: 'f'.repeat(40), headSha: renamed, manifestPath: manifest,
      qualificationInventoryPath: qualificationInventory,
    })).toEqual(allGates());
    expect(selectFromGit({
      eventName: 'pull_request', repository: fixture,
      baseSha: renamed, headSha: withOld, manifestPath: manifest,
      qualificationInventoryPath: qualificationInventory,
    })).toEqual(allGates());
    expect(selectFromGit({
      eventName: 'pull_request', repository: fixture,
      baseSha: base, headSha: base, manifestPath: manifest,
      qualificationInventoryPath: qualificationInventory,
    })).toEqual(allGates());
  });

  it('keeps the workflow base-controlled, fail-open-to-run, and stably aggregated', () => {
    const workflow = readFileSync(resolve(repository, '.github/workflows/ci.yml'), 'utf8');
    const changesJob = workflow.split('  changes:')[1]?.split('\n  coding-harness:')[0] ?? '';
    const qualificationJob = workflow
      .split('  postgresql-observation-qualification:')[1]?.split('\n  build:')[0] ?? '';
    expect(workflow).toContain('merge_group:');
    expect(workflow).toContain('types: [checks_requested]');
    expect(workflow).toContain('permissions:\n  contents: read');
    expect(workflow).toContain('cancel-in-progress: ${{ github.event_name == \'pull_request\' }}');
    expect(workflow).not.toContain('pull_request_target:');
    expect(workflow).not.toMatch(/^\s+paths(?:-ignore)?:/m);
    expect(changesJob).toContain('fetch-depth: 0');
    expect(changesJob).toContain(
      'git show "${BASE_SHA}:coding-harness/scripts/select-ci-gates.mjs"',
    );
    expect(changesJob).toContain(
      'git show "${BASE_SHA}:coding-harness/.harness/manifest.json"',
    );
    expect(changesJob).toContain(
      'git show "${BASE_SHA}:tests/postgresql/observation-qualification-inputs-v1.tsv"',
    );
    expect(changesJob).toContain('--qualification-inventory "$qualification_inventory"');
    expect(changesJob).toContain('HEAD_SHA: ${{ github.sha }}');
    expect(changesJob).toContain("'pg_observation=true'");
    expect(changesJob).toContain("grep -Eq '^pg_observation=(true|false)$'");
    for (const [job, output] of [
      ['coding-harness', 'coding_harness'],
      ['supervisor-service', 'supervisor'],
      ['postgresql-public-acl-replay', 'acl_replay'],
      ['postgresql-observation-qualification', 'pg_observation'],
      ['build', 'rust'],
    ]) {
      expect(workflow.split(`  ${job}:`)).toHaveLength(2);
      expect(workflow).toContain(
        `needs.changes.result != 'success' || needs.changes.outputs.${output} != 'false'`,
      );
      const resultReference = job.includes('-')
        ? `needs['${job}'].result`
        : `needs.${job}.result`;
      expect(workflow.split(resultReference)).toHaveLength(2);
    }
    expect(workflow).toContain('  required:\n    name: required');
    expect(workflow).toContain('    if: ${{ always() }}');
    expect(workflow).toContain(
      'test "$SELECT_ACL_REPLAY" != true || test "$SELECT_SUPERVISOR" = true',
    );
    expect(workflow).toContain(
      'test "$SELECT_SUPERVISOR" != true || test "$SELECT_CODING_HARNESS" = true',
    );
    expect(workflow).toContain(
      'test "$SELECT_PG_OBSERVATION" != true || test "$SELECT_RUST" = true',
    );
    expect(workflow).toContain(
      'test "$SELECT_PG_OBSERVATION" != true || test "$SELECT_CODING_HARNESS" = true',
    );
    expect(qualificationJob).toContain('fetch-depth: 0');
    expect(qualificationJob).toContain('persist-credentials: false');
    expect(qualificationJob).not.toContain('cargo fetch --locked');
    expect(qualificationJob).not.toContain('rustup show');
    expect(qualificationJob).not.toContain('Swatinem/rust-cache');
    expect(qualificationJob).toContain('npm --prefix coding-harness ci --ignore-scripts');
    expect(qualificationJob).toContain('npm --prefix coding-harness run build');
    expect(qualificationJob).toContain(
      'docker pull --platform linux/amd64 "$RUST_BUILDER_IMAGE"',
    );
    expect(qualificationJob).toContain('docker pull --platform linux/amd64 "$POSTGRES_IMAGE"');
    expect(qualificationJob).toContain(
      'npm --prefix coding-harness run postgres-observation-qualification -- replay --patch "${{ matrix.postgres_patch }}"',
    );
    expect(qualificationJob.match(
      /npm --prefix coding-harness run postgres-observation-qualification/g,
    )).toHaveLength(1);
    expect(qualificationJob).not.toContain(
      'npm --prefix coding-harness run postgres-observation-qualification -- check',
    );
    const protocol = JSON.parse(readFileSync(resolve(
      repository, 'tests/postgresql/observation-qualification-protocol-v1.json',
    ), 'utf8')) as {
      builder: { reference: string };
      images: Array<{ patch: string; reference: string }>;
    };
    expect(qualificationJob).toContain(`RUST_BUILDER_IMAGE: ${protocol.builder.reference}`);
    expect(protocol.images).toHaveLength(2);
    expect(new Set(protocol.images.map(({ patch }) => patch))).toEqual(
      new Set(['16.9', '16.15']),
    );
    expect(qualificationJob.match(/^          - node:/gm)).toHaveLength(4);
    const workflowImages = [...qualificationJob.matchAll(
      /^            postgres_image:\s+(\S+)$/gm,
    )].map((match) => match[1]);
    expect(workflowImages).toHaveLength(4);
    expect(new Set(workflowImages)).toEqual(
      new Set(protocol.images.map(({ reference }) => reference)),
    );
    for (const reference of workflowImages) {
      expect(reference).toMatch(/^postgres@sha256:[0-9a-f]{64}$/);
    }
    for (const image of protocol.images) {
      for (const node of ['20.0.0', '24.14.1']) {
        expect(qualificationJob).toContain(
          `- node: '${node}'\n            postgres_patch: '${image.patch}'\n            postgres_image: ${image.reference}`,
        );
      }
    }
    expect(qualificationJob).not.toMatch(/postgres_image:\s+postgres:\d/m);
    expect(qualificationJob).not.toContain('product-mock');
    expect(qualificationJob).not.toContain('POSTGRES_PASSWORD');
    expect(workflow).not.toContain('needs.readiness.result');
  });

  it('bootstraps a trusted four-output base by selecting all five gates', () => {
    const workflow = readFileSync(resolve(repository, '.github/workflows/ci.yml'), 'utf8');
    const changesJob = workflow.split('  changes:')[1]?.split('\n  coding-harness:')[0] ?? '';
    const indented = changesJob.split('        run: |\n')[1] ?? '';
    expect(indented).not.toBe('');
    const script = indented.split('\n').map((line) => (
      line.startsWith('          ') ? line.slice(10) : line
    )).join('\n');

    const fixture = gitFixture();
    mkdirSync(join(fixture, 'coding-harness/.harness'), { recursive: true });
    mkdirSync(join(fixture, 'coding-harness/scripts'), { recursive: true });
    mkdirSync(join(fixture, 'tests/postgresql'), { recursive: true });
    writeFileSync(join(fixture, 'coding-harness/.harness/manifest.json'),
      '{"protectedPaths":["Cargo.toml"]}\n');
    writeFileSync(join(fixture, 'coding-harness/scripts/select-ci-gates.mjs'),
      "process.stdout.write('rust=false\\ncoding_harness=true\\nsupervisor=true\\nacl_replay=true\\n');\n");
    writeFileSync(join(fixture, 'tests/postgresql/observation-qualification-inputs-v1.tsv'),
      'category\tpath\nprotocol\ttests/postgresql/observation-qualification-inputs-v1.tsv\n');
    git(fixture, ['add', '.']);
    const baseSha = commit(fixture, 'legacy four-output selector');
    mkdirSync(join(fixture, 'coding-harness/supervisor-service/src'), { recursive: true });
    writeFileSync(join(fixture, 'coding-harness/supervisor-service/src/index.ts'), 'export {};\n');
    git(fixture, ['add', '.']);
    const headSha = commit(fixture, 'supervisor-only change');
    const runnerTemp = join(fixture, 'runner-temp');
    const output = join(runnerTemp, 'github-output');
    mkdirSync(runnerTemp);
    const result = spawnSync('bash', ['-c', script], {
      cwd: fixture,
      encoding: 'utf8',
      env: {
        ...process.env,
        EVENT_NAME: 'pull_request',
        BASE_SHA: baseSha,
        HEAD_SHA: headSha,
        GITHUB_OUTPUT: output,
        GITHUB_WORKSPACE: fixture,
        RUNNER_TEMP: runnerTemp,
      },
    });
    expect(result.status, result.stderr).toBe(0);
    expect(readFileSync(output, 'utf8')).toBe(formatOutputs(allGates()));
  });
});

function gitFixture(): string {
  const directory = mkdtempSync(join(tmpdir(), 'semantic-fabric-ci-selector-'));
  temporaryDirectories.push(directory);
  git(directory, ['init', '--initial-branch=main']);
  git(directory, ['config', 'user.name', 'Selector Test']);
  git(directory, ['config', 'user.email', 'selector@example.invalid']);
  writeFileSync(join(directory, 'README.md'), 'fixture\n');
  git(directory, ['add', '.']);
  return directory;
}

function commit(directory: string, message: string): string {
  git(directory, ['commit', '--allow-empty', '-m', message]);
  return git(directory, ['rev-parse', 'HEAD']).stdout.trim();
}

function git(directory: string, args: string[]): { stdout: string } {
  const result = spawnSync('git', ['-C', directory, ...args], { encoding: 'utf8' });
  expect(result.status, result.stderr).toBe(0);
  return { stdout: result.stdout };
}

// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(harnessRoot, '..');
const manifest = JSON.parse(
  readFileSync(resolve(harnessRoot, '.harness/manifest.json'), 'utf8'),
) as { protectedPaths: string[] };
const capabilityCatalog = JSON.parse(
  readFileSync(resolve(repository, 'tests/capabilities/catalog-v1.json'), 'utf8'),
) as { cells: Array<{ id: string; status: string; verification: string }> };

const M0_AUTHORITY_PATHS = [
  '.cargo/audit.toml',
  '.harness/mcp-policy.json',
  'coding-harness/__tests__/m0-authority.test.ts',
  'coding-harness/__tests__/programme-capture-claim-io-v1.test.ts',
  'coding-harness/__tests__/programme-capture-claim-record-v1.test.ts',
  'coding-harness/__tests__/programme-capture-host-capabilities-v1.test.ts',
  'coding-harness/__tests__/programme-capture-host-preflight-v1.test.ts',
  'coding-harness/__tests__/programme-envelope-v5.test.ts',
  'coding-harness/__tests__/programme-policy-v5.test.ts',
  'coding-harness/src/programme-capture-claim-io-v1.ts',
  'coding-harness/src/programme-capture-claim-record-v1.ts',
  'coding-harness/src/programme-capture-host-authority-v1.ts',
  'coding-harness/src/programme-capture-host-observation-v1.ts',
  'coding-harness/src/programme-capture-host-preflight-v1.ts',
  'coding-harness/src/programme-capture-input-attestation-record-v1.ts',
  'coding-harness/src/programme-capture-runner-profile-v1.ts',
  'crates/sf-bench/config/performance-scenarios-v1.tsv',
  'crates/sf-bench/src/bin/sf-performance-receipt.rs',
  'crates/sf-bench/src/driver.rs',
  'crates/sf-bench/src/lib.rs',
  'crates/sf-bench/src/mem.rs',
  'crates/sf-bench/src/workload.rs',
  'crates/sf-bench/tests/performance_capture.rs',
  'crates/sf-bench/tests/performance_cli.rs',
  'crates/sf-bench/tests/performance_paths.rs',
  'crates/sf-bench/tests/performance_platform.rs',
  'crates/sf-bench/tests/performance_profile.rs',
  'crates/sf-bench/tests/performance_receipt.rs',
  'crates/sf-bench/tests/performance_stats.rs',
  'crates/sf-bench/tests/performance_subprocess.rs',
  'crates/sf-conformance/src/bin/capability-matrix.rs',
  'crates/sf-conformance/src/bin/rdb2rdf-execution-receipt.rs',
  'crates/sf-conformance/src/bin/rdb2rdf-inventory.rs',
  'crates/sf-conformance/src/bin/rust-closure-receipt.rs',
  'crates/sf-conformance/src/bin/sparql-protocol-regression-baseline.rs',
  'crates/sf-conformance/src/bin/sparql-query-regression-baseline.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/authority.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/artifact_pair.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/capture.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/cargo.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/elf.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/format.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/format/records.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/host_link_authority.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/host_link_authority/tests.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/linker.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/linker/capture_tests.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/mod.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/model.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/model/link.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/process.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/producer.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/producer_paths.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/producer_paths/link_input.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/receipt_file.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/sandbox.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/sandbox_environment.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/source.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/source_blobs.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/source_tree.rs',
  'crates/sf-conformance/src/binary_artifact_receipt/tests.rs',
  'crates/sf-conformance/src/capability_catalog.rs',
  'crates/sf-conformance/src/capability_model.rs',
  'crates/sf-conformance/src/capability_render.rs',
  'crates/sf-conformance/src/execution_receipt.rs',
  'crates/sf-conformance/src/execution_receipt/format.rs',
  'crates/sf-conformance/src/execution_receipt/tests.rs',
  'crates/sf-conformance/src/graph.rs',
  'crates/sf-conformance/src/inventory.rs',
  'crates/sf-conformance/src/inventory/format.rs',
  'crates/sf-conformance/src/inventory/policy.rs',
  'crates/sf-conformance/src/lib.rs',
  'crates/sf-conformance/src/manifest.rs',
  'crates/sf-conformance/src/oracle.rs',
  'crates/sf-conformance/src/pg.rs',
  'crates/sf-conformance/src/pg/outcome.rs',
  'crates/sf-conformance/src/pg/tests.rs',
  'crates/sf-conformance/src/runner.rs',
  'crates/sf-conformance/src/rust_closure_receipt.rs',
  'crates/sf-conformance/src/rust_closure_receipt/controlled.rs',
  'crates/sf-conformance/src/rust_closure_receipt/controlled_tests.rs',
  'crates/sf-conformance/src/sealed_suite.rs',
  'crates/sf-conformance/src/sqlite.rs',
  'crates/sf-conformance/tests/capability_matrix.rs',
  'crates/sf-conformance/tests/differential_graphs.rs',
  'crates/sf-conformance/tests/differential_paths.rs',
  'crates/sf-conformance/tests/differential_star.rs',
  'crates/sf-conformance/tests/differential_tree.rs',
  'crates/sf-conformance/tests/rdb2rdf_execution_receipt.rs',
  'crates/sf-conformance/tests/rdb2rdf_inventory.rs',
  'crates/sf-conformance/tests/rdb2rdf_runner_seal.rs',
  'crates/sf-conformance/tests/regression_baseline_cli.rs',
  'crates/sf-conformance/tests/rust_closure_receipt.rs',
  'crates/sf-conformance/tests/w3c_pg_suite.rs',
  'crates/sf-serve/tests/endpoint.rs',
  'crates/sf-sparql/tests/e2e.rs',
  'docs/capability-matrix.json',
  'docs/capability-matrix.md',
  'docs/plans/sota-application-completion-programme.md',
  'tests/capabilities/catalog-v1.json',
  'tests/capabilities/schema-v1.json',
  'tests/rust-dependency-closure.tsv',
  'tests/sparql/protocol/inventory.tsv',
  'tests/sparql/protocol/sqlite-expected-regression-baseline.tsv',
  'tests/sparql/query/inventory.tsv',
  'tests/sparql/query/sqlite-expected-regression-baseline.tsv',
  'tests/w3c/rdb2rdf/inventory.tsv',
  'tests/w3c/rdb2rdf/postgresql-execution-receipt.tsv',
  'tests/w3c/rdb2rdf/sqlite-execution-receipt.tsv',
] as const;

const EXPECTED_ARTIFACT_INTERFACE_PATHS = [
  'crates/sf-conformance/src/bin/current-sf-cli-artifact-observation.rs',
  'crates/sf-conformance/tests/binary_artifact_receipt.rs',
] as const;

const REQUIRED_CI_COMMANDS = [
  'cargo run --locked -p sf-conformance --bin rdb2rdf-inventory -- --check',
  'cargo run --locked -p sf-conformance --bin rdb2rdf-execution-receipt -- --check',
  'cargo run --locked -p sf-conformance --bin rdb2rdf-execution-receipt -- --backend postgresql --check',
  'cargo run --locked -p sf-conformance --features evidence-receipts --bin sparql-query-regression-baseline -- --check',
  'cargo run --locked -p sf-conformance --features evidence-receipts --bin sparql-protocol-regression-baseline -- --check',
  'cargo run --locked -p sf-conformance --features evidence-receipts --bin rust-closure-receipt -- --check',
  'cargo run --locked -p sf-bench --features performance-receipts --bin sf-performance-receipt -- check-scenarios',
  'cargo run --locked -p sf-conformance --bin capability-matrix -- --check',
] as const;

const REQUIRED_FEATURE_CLIPPY = [
  'cargo clippy --locked -p sf-conformance --features evidence-receipts --all-targets -- -D warnings',
  'cargo clippy --locked -p sf-bench --features performance-receipts --all-targets -- -D warnings',
] as const;

const REQUIRED_ARTIFACT_OBSERVATION_TEST =
  'cargo test --locked -p sf-conformance --features evidence-receipts --lib --test binary_artifact_receipt --test regression_baseline_cli --test rust_closure_receipt -- --test-threads=1';

describe('M0 protected authority and CI contract', () => {
  it('protects every tracked authority in both config and manifest', () => {
    expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths)
      .toEqual(expect.arrayContaining([
        ...M0_AUTHORITY_PATHS,
        ...EXPECTED_ARTIFACT_INTERFACE_PATHS,
      ]));
    expect(manifest.protectedPaths).toEqual(expect.arrayContaining([
      ...M0_AUTHORITY_PATHS,
      ...EXPECTED_ARTIFACT_INTERFACE_PATHS,
    ]));

    const tracked = spawnSync(
      'git', [
        '-C', repository, 'ls-files', '--error-unmatch', '--',
        ...M0_AUTHORITY_PATHS,
        ...EXPECTED_ARTIFACT_INTERFACE_PATHS,
      ],
      { encoding: 'utf8' },
    );
    expect(tracked.status, tracked.stderr).toBe(0);
  });

  it('automatically protects every tracked capability and receipt authority', () => {
    for (const directory of [
      'crates/sf-bench/src/performance/',
      'crates/sf-conformance/src/execution_receipt/',
      'crates/sf-conformance/src/regression_receipt/',
      'crates/sf-conformance/src/rust_closure_receipt/',
      'tests/capabilities/',
      'tests/sparql/',
    ]) {
      const listed = spawnSync(
        'git', ['-C', repository, 'ls-files', directory], { encoding: 'utf8' },
      );
      expect(listed.status, listed.stderr).toBe(0);
      const paths = listed.stdout.split(/\r?\n/).filter(Boolean);
      expect(paths.length).toBeGreaterThan(0);
      expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths)
        .toEqual(expect.arrayContaining(paths));
      expect(manifest.protectedPaths).toEqual(expect.arrayContaining(paths));
    }

    // The receipt may grow into a directory module. If it does, every tracked
    // child becomes protected without relying on a hand-maintained source list.
    const artifactDirectory = 'crates/sf-conformance/src/binary_artifact_receipt/';
    const listed = spawnSync(
      'git', ['-C', repository, 'ls-files', artifactDirectory], { encoding: 'utf8' },
    );
    expect(listed.status, listed.stderr).toBe(0);
    const artifactPaths = listed.stdout.split(/\r?\n/).filter(Boolean);
    expect(artifactPaths.length).toBeGreaterThan(0);
    expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths)
      .toEqual(expect.arrayContaining(artifactPaths));
    expect(manifest.protectedPaths).toEqual(expect.arrayContaining(artifactPaths));
  });

  it('runs each read-only authority check exactly once and in dependency order', () => {
    const workflow = readFileSync(resolve(repository, '.github/workflows/ci.yml'), 'utf8');
    const positions = REQUIRED_CI_COMMANDS.map((command) => {
      expect(workflow.split(command)).toHaveLength(2);
      return workflow.indexOf(command);
    });
    expect(positions).toEqual([...positions].sort((left, right) => left - right));
    for (const binary of [
      'rdb2rdf-inventory',
      'rdb2rdf-execution-receipt',
      'sparql-query-regression-baseline',
      'sparql-protocol-regression-baseline',
      'rust-closure-receipt',
      'sf-performance-receipt',
      'capability-matrix',
    ]) {
      expect(workflow).not.toMatch(new RegExp(`--bin ${binary} -- --generate`));
    }
    expect(workflow.match(
      /cargo run --locked -p sf-bench --features performance-receipts --bin sf-performance-receipt/g,
    )).toHaveLength(1);
    expect(workflow).not.toMatch(
      /--bin sf-performance-receipt -- (?:capture-baseline|capture-candidate)\b/,
    );
    expect(workflow.split(REQUIRED_ARTIFACT_OBSERVATION_TEST)).toHaveLength(2);
    expect(workflow).not.toMatch(
      /cargo run[^\n]*--bin current-sf-cli-artifact-observation\b/,
    );
  });

  it('fails CI on stale controller attestation and feature-only Clippy findings', () => {
    const workflow = readFileSync(resolve(repository, '.github/workflows/ci.yml'), 'utf8');
    const build = 'npm --prefix coding-harness run build';
    const attestation =
      'git diff --exit-code -- coding-harness/.harness/controller-build.json';
    expect(workflow.split(build)).toHaveLength(2);
    expect(workflow.split(attestation)).toHaveLength(2);
    expect(workflow.indexOf(attestation)).toBeGreaterThan(workflow.indexOf(build));
    for (const command of REQUIRED_FEATURE_CLIPPY) {
      expect(workflow.split(command)).toHaveLength(2);
    }
    expect(workflow.indexOf(REQUIRED_FEATURE_CLIPPY[0])).toBeLessThan(
      workflow.indexOf('cargo test --locked -p sf-conformance --features evidence-receipts'),
    );
    expect(workflow.indexOf(REQUIRED_FEATURE_CLIPPY[1])).toBeLessThan(
      workflow.indexOf('cargo test --locked -p sf-bench --features performance-receipts'),
    );
  });

  it('keeps the minimal production artifact planned until its release gates close', () => {
    const cell = capabilityCatalog.cells.find(
      ({ id }) => id === 'minimal-production-artifact-generic',
    );
    expect(cell).toMatchObject({ status: 'planned', verification: 'source-only' });
  });
});

// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  DEVELOPMENT_AUTHORITY,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  METAHARNESS_DIAGNOSTICS_PATH,
  parseMetaHarnessDiagnosticSnapshot,
  type MetaHarnessDiagnosticSnapshot,
} from './metaharness-diagnostics.js';
import {
  PROGRAMME_ACCEPTANCE_DIMENSIONS,
  scoreProgrammeAcceptance,
  type ProgrammeAcceptanceResult,
  type ProgrammeDimensionId,
} from './programme-acceptance.js';
import {
  RECEIPT_FAILURE_CODES,
  failureCodeForReason,
} from './failure-code.js';
import {
  ReceiptChain,
  digestValue,
  type Receipt,
  type ReceiptStatus,
} from './receipts.js';

const ISSUE_8_TASK_ID = 'bprune_8_20260825';
const ISSUE_8_SOURCE = 'crates/sf-sparql/src/unfold.rs';
const ISSUE_8_EVALUATOR = 'crates/sf-conformance/tests/issue_8_binding_pruning.rs';
const MANIFEST = 'coding-harness/.harness/manifest.json';
const TASK = 'coding-harness/config/issue-8-acceptance.json';
// Schema v4 was issued without a policy fingerprint. Freeze its original
// protected-path floor from controller commit 47b9224 so replay never depends
// on the live, expanding controller policy.
export const ISSUE_8_V4_POLICY_SOURCE_COMMIT =
  '47b9224a5fd4e8548c2a0c6c3c479e2e0abdc742';
export const ISSUE_8_V4_REQUIRED_PROTECTED_PATHS = Object.freeze([
  '.github/workflows/ci.yml',
  '.gitignore',
  '.mcp.json',
  'AGENTS.md',
  'BENCHMARKS.md',
  'CLAUDE.md',
  'COMPARISON.md',
  'Cargo.toml',
  'LICENSE-APACHE',
  'LICENSE-MIT',
  'README.md',
  'coding-harness/.claude-plugin/plugin.json',
  'coding-harness/.harness/controller-build.json',
  'coding-harness/.harness/manifest.json',
  'coding-harness/CLAUDE.md',
  'coding-harness/LICENSE',
  'coding-harness/README.md',
  'coding-harness/config/issue-8-acceptance.json',
  'coding-harness/config/metaharness-diagnostics.json',
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
  'coding-harness/src/failure-code.ts',
  'coding-harness/src/frozen-cargo-lock.ts',
  'coding-harness/src/frozen-cargo-metadata.ts',
  'coding-harness/src/git-materialization.ts',
  'coding-harness/src/git-process.ts',
  'coding-harness/src/git-protected-boundary.ts',
  'coding-harness/src/git-worktrees.ts',
  'coding-harness/src/independent-rust-lcov.ts',
  'coding-harness/src/index.ts',
  'coding-harness/src/issue-8-driver.ts',
  'coding-harness/src/issue-8-native-session.ts',
  'coding-harness/src/issue-8-program.ts',
  'coding-harness/src/issue-8-programme-envelope.ts',
  'coding-harness/src/issue-8-rust-runtime.ts',
  'coding-harness/src/issue-8-qe.ts',
  'coding-harness/src/issue-8-system.ts',
  'coding-harness/src/kernel.ts',
  'coding-harness/src/manifest.ts',
  'coding-harness/src/metaharness-diagnostics.ts',
  'coding-harness/src/model-context.ts',
  'coding-harness/src/model-controller.ts',
  'coding-harness/src/native-client.ts',
  'coding-harness/src/native-egress-audit.ts',
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
  'coding-harness/src/native-process-execution.ts',
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
  'coding-harness/src/rust-registry-closure.ts',
  'coding-harness/src/rust-sandbox.ts',
  'coding-harness/src/sandbox.ts',
  'coding-harness/src/workspace.ts',
  'coding-harness/src/writable-overlays.ts',
  'coding-harness/tsconfig.json',
  'coding-harness/vitest.config.ts',
  'crates/sf-bench/Cargo.toml',
  'crates/sf-cli/Cargo.toml',
  'crates/sf-conformance/Cargo.toml',
  'crates/sf-core/Cargo.toml',
  'crates/sf-mapping/Cargo.toml',
  'crates/sf-serve/Cargo.toml',
  'crates/sf-sparql/Cargo.toml',
  'crates/sf-sql/Cargo.toml',
  'docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md',
  'docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md',
  'docs/adr/ADR-0003-shared-core-two-frontend-architecture.md',
  'docs/adr/ADR-0004-oxigraph-rdf-sparql-substrate.md',
  'docs/adr/ADR-0005-conformance-and-benchmark-harness.md',
  'docs/adr/ADR-0006-crate-layout-and-performance-model.md',
  'docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md',
  'docs/adr/ADR-0008-reasoning-strategy.md',
  'docs/adr/ADR-0010-security-and-resource-governance.md',
  'docs/adr/ADR-0011-observability-and-configuration.md',
  'docs/adr/ADR-0012-test-strategy.md',
  'docs/adr/ADR-0014-production-hardening-backlog.md',
  'docs/adr/ADR-0015-datatype-dialect-correctness.md',
  'docs/adr/ADR-0017-provenance-lineage.md',
  'docs/adr/ADR-0018-security-edge.md',
  'docs/adr/ADR-0019-rdf-sparql-shacl-12-readiness.md',
  'docs/adr/ADR-0020-outstanding-sota-optimisations.md',
  'docs/adr/ADR-0021-ontop-parity-program.md',
  'docs/adr/ADR-0022-ws-g-ontop-optimizer-test-port.md',
  'docs/adr/ADR-0023-query-ir-architecture-flat-ucq-vs-iq-tree.md',
  'docs/adr/ADR-0024-executor-backend-abstraction.md',
  'docs/adr/ADR-0025-ontop-parity-residue-closure.md',
  'docs/adr/ADR-0026-agentic-qe-fleet-adoption.md',
  'docs/adr/ADR-0027-qe-fleet-load-test-plan.md',
  'docs/adr/ADR-0028-full-corpus-audit-ontop-parity-ecosystem-gaps-sparql12-coverage.md',
  'docs/adr/ADR-0029-rdf-star-mapping-extension-rml-star-vocabulary-basic-encoding.md',
  'docs/adr/ADR-0030-metaharness-darwin-mode-dev-process-adoption.md',
  'docs/adr/ADR-0031-rdf-star-query-rewrite-quoted-triple-patterns-basic-encoding.md',
  'docs/adr/ADR-0032-rdf-12-soundness-completeness-native-reification.md',
  'docs/adr/ADR-0033-path-as-derived-table-join-composition.md',
  'docs/adr/ADR-0034-virtual-graph-set-semantics-bgp-dedup.md',
  'docs/adr/ADR-0035-variable-graph-querying.md',
  'docs/adr/ADR-0036-correctness-first-open-issue-remediation.md',
  'docs/adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md',
  'docs/plans/open-issues-ruflo-metaharness-implementation-plan.md',
  'harness-plan.json',
  'repo-profile.json',
  'rust-toolchain.toml',
  'scratch/sqlx-spike/Cargo.toml',
]);
export const ISSUE_8_V4_POLICY_DIGEST = digestValue(ISSUE_8_V4_REQUIRED_PROTECTED_PATHS);
const REQUIRED_TOOL_EVIDENCE = Object.freeze([
  'bootstrapSource',
  'bootstrapControllerStoreDigest',
  'bootstrapBuildManifestDigest',
  'bootstrapRuntimeTreeDigest',
  'bootstrapNodeDigest',
  'bootstrapGitDigest',
  'controllerExecutionDigest',
  'controllerBuildManifestDigest',
  'controllerRuntimeTreeDigest',
  'controllerManifestDigest',
  'controllerTaskDigest',
  'cargo',
  'cargoLlvmCov',
  'node',
  'codex',
  'claude',
  'bwrap',
  'systemdRun',
  'systemctl',
  'agenticQeMcp',
]);
export const ISSUE_8_SAFE_TRANSACTION_REASON_CODES = RECEIPT_FAILURE_CODES;

interface ReceiptChainDocument {
  schemaVersion: 3;
  receipts: readonly Receipt[];
}

export interface Issue8ProgrammeEnvelope {
  schemaVersion: 4;
  authority: typeof DEVELOPMENT_AUTHORITY;
  receiptChain: ReceiptChainDocument;
  diagnosticBlob: string;
  diagnosticBlobDigest: string;
  programmeAcceptance: ProgrammeAcceptanceResult;
  programmeAcceptanceDigest: string;
  envelopeDigest: string;
}

export function createIssue8ProgrammeEnvelope(
  receipt: Receipt,
  diagnosticBlob: string,
): Issue8ProgrammeEnvelope {
  const chain = verifyOneReceipt({ schemaVersion: 3, receipts: [receipt] });
  return assembleEnvelope(chain, diagnosticBlob);
}

export function serializeIssue8ProgrammeEnvelope(envelope: Issue8ProgrammeEnvelope): string {
  const verified = parseIssue8ProgrammeEnvelope(JSON.stringify(envelope));
  return `${JSON.stringify(verified, null, 2)}\n`;
}

export function parseIssue8ProgrammeEnvelope(serialized: string): Issue8ProgrammeEnvelope {
  let value: unknown;
  try {
    value = JSON.parse(serialized);
  } catch {
    throw new TypeError('issue #8 programme envelope is not valid JSON');
  }
  const input = asRecord(value, 'issue #8 programme envelope');
  assertExactKeys(input, [
    'schemaVersion', 'authority', 'receiptChain', 'diagnosticBlob',
    'diagnosticBlobDigest', 'programmeAcceptance', 'programmeAcceptanceDigest',
    'envelopeDigest',
  ], 'issue #8 programme envelope');
  if (input.schemaVersion !== 4 || input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('issue #8 programme envelope identity is invalid');
  }
  const chain = verifyOneReceipt(input.receiptChain);
  const expected = assembleEnvelope(chain, text(input.diagnosticBlob));
  if (digestValue(input.programmeAcceptance) !== expected.programmeAcceptanceDigest
    || input.diagnosticBlobDigest !== expected.diagnosticBlobDigest
    || input.programmeAcceptanceDigest !== expected.programmeAcceptanceDigest
    || input.envelopeDigest !== expected.envelopeDigest) {
    throw new Error('HARNESS_ISSUE_8_PROGRAMME_ENVELOPE_DIGEST_INVALID');
  }
  return expected;
}

export function finalizeIssue8ProgrammeOutcome(input: Readonly<{
  transactionStatus: ReceiptStatus;
  transactionReason: string | null;
  envelope: Issue8ProgrammeEnvelope;
}>): Readonly<{ status: ReceiptStatus; reason: string | null }> {
  const receipt = input.envelope.receiptChain.receipts[0];
  if (receipt === undefined || receipt.status !== input.transactionStatus) {
    throw new Error('HARNESS_ISSUE_8_TRANSACTION_STATUS_MISMATCH');
  }
  if (input.transactionStatus === 'pass') {
    if (input.transactionReason !== null) {
      throw new Error('HARNESS_ISSUE_8_PASS_REASON_INVALID');
    }
    if (input.envelope.programmeAcceptance.status === 'ACCEPTED') {
      return deepFreeze({ status: 'pass', reason: null });
    }
    return deepFreeze({
      status: 'gated',
      reason: 'HARNESS_ISSUE_8_PROGRAMME_ACCEPTANCE_REJECTED',
    });
  }
  const reason = failureCodeForReason(input.transactionReason);
  if (receipt.failureCode !== reason) throw new Error('HARNESS_ISSUE_8_FAILURE_CODE_MISMATCH');
  return deepFreeze({ status: input.transactionStatus, reason });
}

function assembleEnvelope(
  chain: ReceiptChain,
  diagnosticBlob: string,
): Issue8ProgrammeEnvelope {
  const receipt = chain.entries()[0];
  if (receipt === undefined) throw new Error('HARNESS_ISSUE_8_PROGRAMME_RECEIPT_MISSING');
  const diagnosticBlobDigest = createHash('sha256').update(diagnosticBlob, 'utf8').digest('hex');
  if (receipt.protectedInputs[METAHARNESS_DIAGNOSTICS_PATH] !== diagnosticBlobDigest) {
    throw new Error('HARNESS_ISSUE_8_PROGRAMME_DIAGNOSTIC_BLOB_MISMATCH');
  }
  let rawDiagnostics: unknown;
  try { rawDiagnostics = JSON.parse(diagnosticBlob); } catch {
    throw new TypeError('issue #8 programme diagnostic blob is not valid JSON');
  }
  const diagnosticEvidence = parseMetaHarnessDiagnosticSnapshot(rawDiagnostics);
  const receiptChain = JSON.parse(chain.export()) as ReceiptChainDocument;
  const programmeAcceptance = scoreIssue8Receipt(receipt, diagnosticEvidence);
  const programmeAcceptanceDigest = digestValue(programmeAcceptance);
  const body = {
    schemaVersion: 4 as const,
    authority: DEVELOPMENT_AUTHORITY,
    receiptChain,
    diagnosticBlob,
    diagnosticBlobDigest,
    programmeAcceptance,
    programmeAcceptanceDigest,
  };
  return deepFreeze({ ...body, envelopeDigest: digestValue(body) });
}

function verifyOneReceipt(value: unknown): ReceiptChain {
  const chain = ReceiptChain.import(JSON.stringify(value));
  if (chain.length !== 1) throw new Error('HARNESS_ISSUE_8_PROGRAMME_RECEIPT_COUNT_INVALID');
  return chain;
}

function scoreIssue8Receipt(
  receipt: Receipt,
  diagnostics: MetaHarnessDiagnosticSnapshot,
): ProgrammeAcceptanceResult {
  const protectedGate = [...ISSUE_8_V4_REQUIRED_PROTECTED_PATHS, ISSUE_8_EVALUATOR]
    .every((path) => path in receipt.protectedInputs);
  const toolGate = REQUIRED_TOOL_EVIDENCE.every((name) => name in receipt.toolVersions)
    && receipt.toolVersions.bootstrapSource === 'verified-packed-private-runtime';
  const attempt = receipt.recovery.repairCount;
  const verifier = (stage: string) => `attempt-${attempt}:${stage}` in receipt.verifierDigests;
  const redGate = receipt.taskId === ISSUE_8_TASK_ID
    && ISSUE_8_EVALUATOR in receipt.protectedInputs
    && 'red-baseline' in receipt.verifierDigests
    && receipt.commands.some(({ stage, exitCode }) => stage === 'red-baseline' && exitCode === 101);
  const evolutionGate = MANIFEST in receipt.protectedInputs
    && receipt.toolVersions.controllerManifestDigest === receipt.protectedInputs[MANIFEST]
    && receipt.toolVersions.controllerTaskDigest === receipt.protectedInputs[TASK];
  const patchedGate = receipt.status === 'pass'
    && receipt.admittedPaths.length === 1 && receipt.admittedPaths[0] === ISSUE_8_SOURCE
    && receipt.patchDigest !== null && Object.keys(receipt.artifactDigests).length > 0
    && verifier('public') && verifier('independent') && verifier('regression')
    && Object.keys(receipt.verifierDigests).some((key) => key.startsWith(`attempt-${attempt}:mutation`));
  const nativeGate = receipt.hosts.length === 2
    && new Set(receipt.hosts.map(({ host }) => host)).size === 2
    && receipt.critiqueDigests.length > 0 && receipt.reviewDigests.length === 2
    && receipt.coordination.nativeEvidenceDigests.length >= 4
    && receipt.coordination.nativeRuntimeEvidenceDigest !== null;
  const reliabilityGate = receipt.status === 'pass'
    && !receipt.recovery.cancelled && receipt.recovery.breakerState === 'closed'
    && receipt.commands.every((command) => !command.timedOut && !command.cancelled
      && !command.outputLimitExceeded && command.signal === null && command.spawnErrorDigest === null);
  const rufloQeGate = receipt.coordination.swarmId !== null
    && receipt.coordination.taskId !== null
    && receipt.coordination.agenticQeEvidenceDigests.length === 2
    && receipt.toolVersions.agenticQe === '3.13.10#sast-only-flat-v1+lcov-gap'
    && typeof receipt.toolVersions.rufloHive === 'string'
    && typeof receipt.toolVersions.rufloConsensus === 'string';

  const gates: Record<ProgrammeDimensionId, readonly [boolean, unknown]> = {
    'policy-and-supply-chain-safety': [protectedGate && toolGate, {
      authority: receipt.authority, protectedInputs: receipt.protectedInputs,
      toolVersions: receipt.toolVersions,
    }],
    'evaluator-integrity': [redGate, {
      evaluator: receipt.identities.evaluator, redBaseline: receipt.verifierDigests['red-baseline'],
      independent: receipt.verifierDigests[`attempt-${attempt}:independent`],
    }],
    'evolution-containment': [evolutionGate, {
      manifest: receipt.protectedInputs[MANIFEST], task: receipt.protectedInputs[TASK],
      controllerManifest: receipt.toolVersions.controllerManifestDigest,
    }],
    'patched-candidate-verification': [patchedGate, {
      candidate: receipt.identities.candidate, patch: receipt.patchDigest,
      artifacts: receipt.artifactDigests, verifiers: receipt.verifierDigests,
    }],
    'dual-host-control-plane': [nativeGate, {
      hosts: receipt.hosts, critiques: receipt.critiqueDigests, reviews: receipt.reviewDigests,
      native: receipt.coordination.nativeEvidenceDigests,
      runtime: receipt.coordination.nativeRuntimeEvidenceDigest,
    }],
    'reliability-and-receipts': [reliabilityGate, {
      receipt: receipt.digest, recovery: receipt.recovery, commands: receipt.commands,
    }],
    'ruflo-and-qe-integration': [rufloQeGate, {
      coordination: receipt.coordination, route: receipt.route,
      hive: receipt.toolVersions.rufloHive, consensus: receipt.toolVersions.rufloConsensus,
      qe: receipt.toolVersions.agenticQe,
    }],
  };
  return scoreProgrammeAcceptance({
    schemaVersion: 2,
    authority: DEVELOPMENT_AUTHORITY,
    receiptDigest: receipt.digest,
    dimensions: Object.entries(PROGRAMME_ACCEPTANCE_DIMENSIONS).map(([id, maximumPoints]) => {
      const [passed, evidence] = gates[id as ProgrammeDimensionId];
      return {
        id, verifiedPoints: passed ? maximumPoints : 0, hardGatePassed: passed,
        evidenceDigests: [digestValue(evidence)],
      };
    }),
    upstreamDiagnostics: [
      ...diagnostics.targets.map((target) => ({
        target: target.target,
        implementation: `metaharness@${diagnostics.implementation.metaharness}` as const,
        success: target.success,
        degraded: target.degraded,
        exitCode: target.exitCode,
        scaffoldReady: target.scaffoldReady,
        hardConstraintsPassed: target.hardConstraintsPassed,
        hardConstraintsTotal: target.hardConstraintsTotal,
        harnessFit: target.harnessFit,
        evidenceDigest: digestValue(target),
      })),
    ],
  });
}

function text(value: unknown): string {
  if (typeof value !== 'string' || value.length < 1 || Buffer.byteLength(value, 'utf8') > 1_000_000) {
    throw new TypeError('issue #8 programme envelope.diagnosticBlob is invalid');
  }
  return value;
}

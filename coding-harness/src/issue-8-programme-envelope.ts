// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  DEVELOPMENT_AUTHORITY,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import { SECURE_HARNESS_CONFIG } from './config.js';
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
  const protectedGate = [...SECURE_HARNESS_CONFIG.requiredProtectedPaths, ISSUE_8_EVALUATOR]
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

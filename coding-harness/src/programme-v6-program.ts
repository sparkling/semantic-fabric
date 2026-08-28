// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  createProgrammeEnvelopeV6,
  finalizeProgrammeOutcomeV6,
  parseProgrammeEnvelopeV6,
  serializeProgrammeEnvelopeV6,
} from './programme-envelope-v6.js';
import type {
  ProgrammeV5DriverResult,
  ProgrammeV5PreparedTransaction,
} from './programme-v5-driver.js';
import { canonicalProgrammePolicyJson } from './programme-v5-driver-support.js';
import {
  assertAbsent,
  parseProgrammeV5Bootstrap,
  parseProgrammeV5Invocation,
  parseProgrammeV5PolicyReviewInvocation,
  readProgrammeV5Diagnostics,
  removeProgrammeV5Scratch,
  type ProgrammeV5BaseInvocation,
  type ProgrammeV5BootstrapEvidence,
  type ProgrammeV5Invocation,
} from './programme-v5-program-runtime.js';
import {
  claimProgrammeV6Execution,
  readProgrammeV6PolicyReviewReceipt,
} from './programme-v6-policy-anchor.js';
import {
  createProgrammeV6ScratchRoot,
  prepareProgrammeV6BaseExecution,
} from './programme-v6-base-execution.js';
import {
  PROGRAMME_V6_CLAIM_AUTHORITY_ROOT,
  assertProgrammeV6ArtifactAbsent,
  programmeV6ArtifactPath,
  writeProgrammeV6PrivateArtifact,
} from './programme-v6-receipt-io.js';
import {
  createFrozenProgrammePolicyV2,
  programmePolicyV2Fingerprint,
  verifyFrozenProgrammePolicyV2,
  type FrozenProgrammePolicyV2,
} from './programme-policy-v6.js';
import { verifyFrozenProgrammePolicyV1 } from './programme-policy-v5.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export { replayTrustedProgrammeV6 } from './programme-v6-replay.js';

export interface TrustedProgrammeV6PolicyReview {
  readonly policyBlob: string;
  readonly policyFingerprint: string;
}

export interface TrustedProgrammeV6Preparation {
  readonly policyBlob: string;
  execute(policyBlob: string): Promise<TrustedProgrammeV6Outcome>;
  abort(): Promise<void>;
}

export interface TrustedProgrammeV6Outcome {
  readonly status: 'pass' | 'fail' | 'gated' | 'cancelled';
  readonly reason: string | null;
  seal(): Promise<Readonly<{
    status: string;
    transactionStatus: 'pass' | 'fail' | 'gated' | 'cancelled';
    receiptPath: string;
    receiptDigest: string;
    candidateTransactionEvidenceDigest: string | null;
    programmeAcceptanceDigest: string;
    envelopeDigest: string;
    policyFingerprint: string;
    executionClaimDigest: string;
  }>>;
}

export function createReviewableProgrammeV6Policy(
  basePolicyBlob: string,
  basePolicyFingerprint: string,
): TrustedProgrammeV6PolicyReview {
  const base = verifyFrozenProgrammePolicyV1(
    parseJsonWithoutDuplicateKeys(basePolicyBlob, 'programme v6 base policy'),
    basePolicyFingerprint,
  );
  if (canonicalProgrammePolicyJson(base.snapshot) !== basePolicyBlob) {
    throw new Error('HARNESS_PROGRAMME_V6_BASE_POLICY_SERIALIZATION_INVALID');
  }
  const policy = createFrozenProgrammePolicyV2(base.snapshot, base.fingerprint);
  const policyFingerprint = programmePolicyV2Fingerprint(policy);
  const policyBlob = canonicalProgrammePolicyJson(policy);
  const verified = verifyFrozenProgrammePolicyV2(
    parseJsonWithoutDuplicateKeys(policyBlob, 'programme v6 reviewed policy'),
    policyFingerprint,
  );
  if (canonicalProgrammePolicyJson(verified.snapshot) !== policyBlob
    || sha256(policyBlob) !== policyFingerprint) {
    throw new Error('HARNESS_PROGRAMME_V6_POLICY_REVIEW_FINGERPRINT_MISMATCH');
  }
  return Object.freeze({ policyBlob, policyFingerprint });
}

export async function prepareReviewableProgrammeV6Policy(
  argv: readonly string[],
  rawBootstrap: unknown,
): Promise<TrustedProgrammeV6PolicyReview> {
  const invocation = parseProgrammeV5PolicyReviewInvocation(argv);
  const bootstrap = parseProgrammeV5Bootstrap(rawBootstrap);
  assertBootstrapBinding(invocation, bootstrap);
  const receiptPath = programmeV6ArtifactPath(
    invocation.repositoryRoot, invocation.runId, 'execution',
  );
  assertProgrammeV6ArtifactAbsent(receiptPath, 'HARNESS_PROGRAMME_V6_RECEIPT_EXISTS');
  const scratch = await createProgrammeV6ScratchRoot();
  let prepared: ProgrammeV5PreparedTransaction | undefined;
  let review: TrustedProgrammeV6PolicyReview | undefined;
  let failure: unknown;
  try {
    prepared = await prepareProgrammeV6BaseExecution(invocation, bootstrap, scratch);
    review = createReviewableProgrammeV6Policy(
      prepared.policyBlob, prepared.policyFingerprint,
    );
  } catch (error) { failure = error; }
  if (prepared !== undefined) {
    try { await prepared.abort(); } catch (cleanupError) {
      failure = failure === undefined
        ? cleanupError
        : new AggregateError(
            [failure, cleanupError],
            'HARNESS_PROGRAMME_V6_POLICY_REVIEW_AND_TRANSACTION_CLEANUP_FAILED',
          );
    }
  }
  try { await removeProgrammeV5Scratch(scratch, failure); } catch (cleanupError) {
    failure = failure === undefined
      ? cleanupError
      : new AggregateError(
          [failure, cleanupError],
          'HARNESS_PROGRAMME_V6_POLICY_REVIEW_AND_SCRATCH_CLEANUP_FAILED',
        );
  }
  if (failure !== undefined) throw failure;
  if (review === undefined) throw new Error('HARNESS_PROGRAMME_V6_POLICY_REVIEW_MISSING');
  return review;
}

export async function prepareTrustedProgrammeV6(
  argv: readonly string[],
  rawBootstrap: unknown,
  claimAuthorityRoot = PROGRAMME_V6_CLAIM_AUTHORITY_ROOT,
): Promise<TrustedProgrammeV6Preparation> {
  const invocation = parseProgrammeV5Invocation(argv);
  const bootstrap = parseProgrammeV5Bootstrap(rawBootstrap);
  assertBootstrapBinding(invocation, bootstrap);
  const receiptPath = programmeV6ArtifactPath(
    invocation.repositoryRoot, invocation.runId, 'execution',
  );
  assertProgrammeV6ArtifactAbsent(receiptPath, 'HARNESS_PROGRAMME_V6_RECEIPT_EXISTS');
  const policyReviewReceipt = readProgrammeV6PolicyReviewReceipt(invocation, bootstrap);
  const executionClaim = claimProgrammeV6Execution(
    invocation, policyReviewReceipt, claimAuthorityRoot,
  );
  const scratch = await createProgrammeV6ScratchRoot();
  let prepared: ProgrammeV5PreparedTransaction | undefined;
  let reviewed: TrustedProgrammeV6PolicyReview | undefined;
  try {
    prepared = await prepareProgrammeV6BaseExecution(invocation, bootstrap, scratch);
    reviewed = createReviewableProgrammeV6Policy(
      prepared.policyBlob, prepared.policyFingerprint,
    );
    assertExpectedPolicy(invocation, reviewed);
  } catch (error) {
    let failure: unknown = error;
    if (prepared !== undefined) {
      try { await prepared.abort(); } catch (cleanupError) {
        failure = new AggregateError(
          [failure, cleanupError],
          'HARNESS_PROGRAMME_V6_PREPARE_AND_TRANSACTION_CLEANUP_FAILED',
        );
      }
    }
    try { await removeProgrammeV5Scratch(scratch, failure); } catch (cleanupError) {
      failure = new AggregateError(
        [failure, cleanupError],
        'HARNESS_PROGRAMME_V6_PREPARE_AND_SCRATCH_CLEANUP_FAILED',
      );
    }
    throw failure;
  }
  if (prepared === undefined || reviewed === undefined) {
    throw new Error('HARNESS_PROGRAMME_V6_PREPARATION_MISSING');
  }
  let state: 'prepared' | 'executing' | 'closed' = 'prepared';
  let scratchPresent = true;
  const removeScratch = async (priorFailure?: unknown) => {
    if (!scratchPresent) return;
    if (await removeProgrammeV5Scratch(scratch, priorFailure)) scratchPresent = false;
  };
  const basePrepared = prepared;
  const outerReview = reviewed;
  return Object.freeze({
    policyBlob: outerReview.policyBlob,
    async execute(policyBlob: string) {
      if (state !== 'prepared') throw new Error('HARNESS_PROGRAMME_V6_EXECUTION_REUSED');
      state = 'executing';
      let result: ProgrammeV5DriverResult | undefined;
      let failure: unknown;
      let baseExecutionStarted = false;
      try {
        const supplied = verifySuppliedPolicy(policyBlob, outerReview.policyFingerprint);
        if (policyBlob !== outerReview.policyBlob
          || supplied.basePolicyFingerprint !== basePrepared.policyFingerprint
          || canonicalProgrammePolicyJson(supplied.basePolicy) !== basePrepared.policyBlob) {
          throw new Error('HARNESS_PROGRAMME_V6_BASE_POLICY_BINDING_MISMATCH');
        }
        baseExecutionStarted = true;
        result = await basePrepared.execute(
          canonicalProgrammePolicyJson(supplied.basePolicy), supplied.basePolicyFingerprint,
        );
      } catch (error) {
        failure = error;
      }
      if (!baseExecutionStarted) {
        try { await basePrepared.abort(); } catch (cleanupError) {
          failure = failure === undefined
            ? cleanupError
            : new AggregateError(
                [failure, cleanupError],
                'HARNESS_PROGRAMME_V6_POLICY_AND_TRANSACTION_CLEANUP_FAILED',
              );
        }
      }
      try { await removeScratch(failure); } catch (cleanupError) {
        failure = failure === undefined
          ? cleanupError
          : new AggregateError(
              [failure, cleanupError],
              'HARNESS_PROGRAMME_V6_EXECUTION_AND_SCRATCH_CLEANUP_FAILED',
            );
      }
      state = 'closed';
      if (failure !== undefined) throw failure;
      if (result === undefined) throw new Error('HARNESS_PROGRAMME_V6_RESULT_MISSING');
      return await createOutcome(
        invocation, result, outerReview, receiptPath, executionClaim.digest,
      );
    },
    async abort() {
      if (state !== 'prepared') return;
      state = 'closed';
      let failure: unknown;
      try { await basePrepared.abort(); } catch (error) { failure = error; }
      try { await removeScratch(failure); } catch (cleanupError) {
        failure = failure === undefined
          ? cleanupError
          : new AggregateError(
              [failure, cleanupError],
              'HARNESS_PROGRAMME_V6_ABORT_AND_SCRATCH_CLEANUP_FAILED',
            );
      }
      if (failure !== undefined) throw failure;
    },
  });
}

function verifySuppliedPolicy(
  policyBlob: string,
  expectedFingerprint: string,
): FrozenProgrammePolicyV2 {
  const policy = verifyFrozenProgrammePolicyV2(
    parseJsonWithoutDuplicateKeys(policyBlob, 'programme v6 supplied policy'),
    expectedFingerprint,
  );
  if (canonicalProgrammePolicyJson(policy.snapshot) !== policyBlob) {
    throw new Error('HARNESS_PROGRAMME_V6_POLICY_SERIALIZATION_INVALID');
  }
  return policy.snapshot;
}

function assertExpectedPolicy(
  invocation: ProgrammeV5Invocation,
  review: TrustedProgrammeV6PolicyReview,
): void {
  if (invocation.expectedPolicy.controllerCommit !== invocation.controllerCommit
    || invocation.expectedPolicy.taskPath !== invocation.taskPath
    || invocation.expectedPolicy.fingerprint !== review.policyFingerprint) {
    throw new Error('HARNESS_PROGRAMME_V6_EXPECTED_POLICY_FINGERPRINT_MISMATCH');
  }
}

function assertBootstrapBinding(
  invocation: ProgrammeV5BaseInvocation,
  bootstrap: ProgrammeV5BootstrapEvidence,
): void {
  if (bootstrap.controllerCommit !== invocation.controllerCommit
    || bootstrap.taskPath !== invocation.taskPath) {
    throw new Error('HARNESS_PROGRAMME_V6_BOOTSTRAP_BINDING_MISMATCH');
  }
}

async function createOutcome(
  invocation: ProgrammeV5Invocation,
  result: ProgrammeV5DriverResult,
  review: TrustedProgrammeV6PolicyReview,
  receiptPath: string,
  executionClaimDigest: string,
): Promise<TrustedProgrammeV6Outcome> {
  const policy = verifySuppliedPolicy(review.policyBlob, review.policyFingerprint);
  if (result.policyFingerprint !== policy.basePolicyFingerprint
    || canonicalProgrammePolicyJson(result.policy) !== canonicalProgrammePolicyJson(policy.basePolicy)) {
    throw new Error('HARNESS_PROGRAMME_V6_DRIVER_POLICY_ANCHOR_MISMATCH');
  }
  const diagnosticBlob = await readProgrammeV5Diagnostics(
    invocation.controllerStore, invocation.controllerCommit,
  );
  const envelope = createProgrammeEnvelopeV6({
    policy,
    rufloEvidence: result.rufloEvidence,
    candidateTransactionEvidence: result.transaction.transactionEvidence,
    receipt: result.transaction.receipt,
    diagnosticBlob,
  }, review.policyFingerprint);
  const serialized = serializeProgrammeEnvelopeV6(envelope, review.policyFingerprint);
  const verified = parseProgrammeEnvelopeV6(serialized, review.policyFingerprint);
  if (verified.receiptChain.receipts.length !== 1
    || verified.receiptChain.receipts[0]?.digest !== result.transaction.receipt.digest) {
    throw new Error('HARNESS_PROGRAMME_V6_RECEIPT_CHAIN_INVALID');
  }
  const finalized = finalizeProgrammeOutcomeV6({
    expectedPolicyFingerprint: review.policyFingerprint,
    transactionStatus: result.transaction.status,
    transactionReason: result.transaction.reason,
    envelope: verified,
  });
  let sealed = false;
  return Object.freeze({
    ...finalized,
    async seal() {
      if (sealed) throw new Error('HARNESS_PROGRAMME_V6_OUTCOME_ALREADY_SEALED');
      assertAbsent(receiptPath, 'HARNESS_PROGRAMME_V6_RECEIPT_EXISTS');
      writeProgrammeV6PrivateArtifact(invocation.repositoryRoot, receiptPath, serialized);
      sealed = true;
      return Object.freeze({
        status: finalized.status,
        transactionStatus: result.transaction.status,
        receiptPath,
        receiptDigest: result.transaction.receipt.digest,
        candidateTransactionEvidenceDigest: verified.candidateTransactionEvidenceDigest,
        programmeAcceptanceDigest: verified.programmeAcceptanceDigest,
        envelopeDigest: verified.envelopeDigest,
        policyFingerprint: review.policyFingerprint,
        executionClaimDigest,
      });
    },
  });
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

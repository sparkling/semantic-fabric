// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  createProgrammeEnvelopeV6,
  finalizeProgrammeOutcomeV6,
  parseProgrammeEnvelopeV6,
  serializeProgrammeEnvelopeV6,
  type ProgrammeEnvelopeInputV6,
} from '../src/programme-envelope-v6.js';
import { digestValue, type Receipt } from '../src/receipts.js';
import { diagnosticBlob } from './candidate-fixtures.js';
import {
  programmeV6Fixture,
  rebindCandidateEvidence,
  rehashCandidateEvidence,
  rehashReceipt,
} from './programme-v6-fixtures.js';

describe('schema-v6 programme envelope', () => {
  it.each(['none', 'not-started', 'failed', 'passed'] as const)(
    'round-trips an accepted %s history with exact evidence ordering',
    (disposition) => {
      const fixture = programmeV6Fixture(disposition);
      const input = envelopeInput(fixture);
      const envelope = createProgrammeEnvelopeV6(input, fixture.policy.fingerprint);
      const serialized = serializeProgrammeEnvelopeV6(envelope, fixture.policy.fingerprint);
      const replayed = parseProgrammeEnvelopeV6(serialized, fixture.policy.fingerprint);

      expect(replayed).toEqual(envelope);
      expect(replayed.programmeAcceptance).toMatchObject({
        status: 'ACCEPTED', score: 100, hardGatesPassed: true, fitnessEligible: false,
      });
      expect(replayed.candidateTransactionEvidenceDigest)
        .toBe(fixture.candidateTransactionEvidence.evidenceDigest);
      expect(Object.keys(replayed)).toEqual([
        'schemaVersion', 'authority', 'policy', 'policyFingerprint', 'rufloEvidence',
        'rufloEvidenceDigest', 'receiptChain', 'candidateTransactionEvidence',
        'candidateTransactionEvidenceDigest', 'diagnosticBlob', 'diagnosticBlobDigest',
        'programmeAcceptance', 'programmeAcceptanceDigest', 'envelopeDigest',
      ]);
      expect(serialized.endsWith('\n')).toBe(true);
      expect(Object.isFrozen(replayed)).toBe(true);
    },
  );

  it('requires a non-null sidecar for pass and an exact null pair for non-pass', () => {
    const passing = programmeV6Fixture();
    expect(() => createProgrammeEnvelopeV6({
      ...envelopeInput(passing), candidateTransactionEvidence: null,
    }, passing.policy.fingerprint)).toThrow();

    const failed = programmeV6Fixture();
    const receipt = structuredClone(failed.receipt) as Receipt;
    receipt.status = 'fail';
    receipt.failureCode = 'HARNESS_TRANSACTION_FAILED';
    failed.receipt = rehashReceipt(receipt);
    const input = { ...envelopeInput(failed), candidateTransactionEvidence: null };
    const envelope = createProgrammeEnvelopeV6(input, failed.policy.fingerprint);

    expect(envelope.candidateTransactionEvidence).toBeNull();
    expect(envelope.candidateTransactionEvidenceDigest).toBeNull();
    expect(envelope.programmeAcceptance.status).toBe('REJECTED');
    expect(parseProgrammeEnvelopeV6(
      serializeProgrammeEnvelopeV6(envelope, failed.policy.fingerprint),
      failed.policy.fingerprint,
    )).toEqual(envelope);
    expect(finalizeProgrammeOutcomeV6({
      expectedPolicyFingerprint: failed.policy.fingerprint,
      transactionStatus: 'fail',
      transactionReason: 'HARNESS_TRANSACTION_FAILED: bounded detail',
      envelope,
    })).toEqual({ status: 'fail', reason: 'HARNESS_TRANSACTION_FAILED' });
    expect(() => createProgrammeEnvelopeV6({
      ...envelopeInput(failed), candidateTransactionEvidence: failed.candidateTransactionEvidence,
    }, failed.policy.fingerprint)).toThrow('HARNESS_PROGRAMME_V6_NONPASS_EVIDENCE_FORBIDDEN');
  });

  it('gates a structurally passing transaction when a final V1 gate fails', () => {
    const fixture = programmeV6Fixture('not-started');
    const receipt = structuredClone(fixture.receipt) as Receipt;
    receipt.admittedPaths = ['crates/sf-sparql/src/not-declared.rs'];
    fixture.receipt = rehashReceipt(receipt);
    rebindCandidateEvidence(fixture);
    const envelope = createProgrammeEnvelopeV6(envelopeInput(fixture), fixture.policy.fingerprint);

    expect(envelope.programmeAcceptance.status).toBe('REJECTED');
    expect(finalizeProgrammeOutcomeV6({
      expectedPolicyFingerprint: fixture.policy.fingerprint,
      transactionStatus: 'pass', transactionReason: null, envelope,
    })).toEqual({ status: 'gated', reason: 'HARNESS_PROGRAMME_ACCEPTANCE_REJECTED' });
  });

  it('fails closed on anchors, loose keys, duplicate keys, downgrade, and nested tampering', () => {
    const fixture = programmeV6Fixture('not-started');
    const envelope = createProgrammeEnvelopeV6(envelopeInput(fixture), fixture.policy.fingerprint);
    expect(() => createProgrammeEnvelopeV6(envelopeInput(fixture), 'e'.repeat(64)))
      .toThrow(/FINGERPRINT/);
    expect(() => (createProgrammeEnvelopeV6 as any)(envelopeInput(fixture))).toThrow(/ANCHOR/);
    expect(() => parseProgrammeEnvelopeV6(
      JSON.stringify({ ...envelope, extra: true }), fixture.policy.fingerprint,
    )).toThrow(/invalid keys/);
    expect(() => parseProgrammeEnvelopeV6(
      JSON.stringify({ ...envelope, schemaVersion: 5 }), fixture.policy.fingerprint,
    )).toThrow(/IDENTITY/);

    const duplicate = JSON.stringify(envelope).replace(
      '"schemaVersion":6', '"schemaVersion":6,"schemaVersion":6',
    );
    expect(() => parseProgrammeEnvelopeV6(duplicate, fixture.policy.fingerprint))
      .toThrow(/duplicate/);

    const tampered = structuredClone(envelope) as any;
    const repair = tampered.candidateTransactionEvidence.nativeRuntimeEvidence.invocations
      .find((invocation: any) => invocation.operation === 'repair');
    repair.patchPayloadSha256 = 'e'.repeat(64);
    rehashCandidateEvidence(tampered.candidateTransactionEvidence);
    const { envelopeDigest: _digest, ...body } = tampered;
    tampered.envelopeDigest = digestValue(body);
    expect(() => parseProgrammeEnvelopeV6(JSON.stringify(tampered), fixture.policy.fingerprint))
      .toThrow();
  });
});

function envelopeInput(fixture: ReturnType<typeof programmeV6Fixture>): ProgrammeEnvelopeInputV6 {
  return {
    policy: fixture.policy.snapshot,
    rufloEvidence: fixture.rufloEvidence,
    candidateTransactionEvidence: fixture.candidateTransactionEvidence,
    receipt: fixture.receipt,
    diagnosticBlob,
  };
}

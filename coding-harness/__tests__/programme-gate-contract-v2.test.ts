// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import { createProgrammeGateContractV1 } from '../src/programme-gate-contract-v1.js';
import {
  createProgrammeGateContractV2,
  parseProgrammeGateContractV2,
} from '../src/programme-gate-contract-v2.js';
import { digestValue } from '../src/receipts.js';

describe('programme gate contract V2', () => {
  it('wraps the frozen V1 law without changing it', () => {
    const base = createProgrammeGateContractV1(2);
    const baseDigest = digestValue(base);
    const contract = createProgrammeGateContractV2(base);

    expect(contract.baseGateContract).toEqual(base);
    expect(contract.baseGateContractDigest).toBe(baseDigest);
    expect(digestValue(base)).toBe(baseDigest);
    expect(contract.attempts).toMatchObject({
      maximumRepairs: 2,
      evidenceSchemaVersion: 1,
      evidenceKind: 'candidate-transaction-repair-evidence',
      dispositionRule: 'not-started-zero-builds-failed-prefix-passed-exact-successful-set',
      finalAttemptSemantics: 'programme-gate-contract-v1-unchanged',
    });
    expect(contract.evaluation).toEqual({
      baseDimensions: 'evaluateProgrammeGatesV5-over-frozen-base-policy',
      reliabilityRule: 'v1-reliability-base-and-transition-aware-prior-attempt-history',
      dimensionEvidenceRule:
        'schema-v2-binds-base-digest-v2-policy-candidate-evidence-and-result',
    });
    expect(contract.envelope.candidateEvidenceDigestBinding).toBe(
      'envelope.candidateTransactionEvidenceDigest-equals-candidateTransactionEvidence.evidenceDigest',
    );
    expect(parseProgrammeGateContractV2(structuredClone(contract))).toEqual(contract);
    for (const value of [contract, contract.baseGateContract, contract.attempts,
      contract.nativeEvidence, contract.evaluation, contract.envelope]) {
      expect(Object.isFrozen(value)).toBe(true);
    }
  });

  it.each([
    ['unknown key', (value: any) => { value.extra = true; }],
    ['missing key', (value: any) => { delete value.evaluation; }],
    ['base digest', (value: any) => { value.baseGateContractDigest = 'e'.repeat(64); }],
    ['attempt law', (value: any) => { value.attempts.dispositionRule = 'caller-asserted'; }],
    ['native law', (value: any) => { value.nativeEvidence.fullRuntimeEvidenceRequired = false; }],
    ['evaluation law', (value: any) => { value.evaluation.baseDimensions = 'replacement'; }],
    ['envelope law', (value: any) => {
      value.envelope.candidateEvidenceDigestBinding = 'digest-whole-self-digested-object';
    }],
  ])('rejects %s tampering', (_name, mutate) => {
    const value = structuredClone(
      createProgrammeGateContractV2(createProgrammeGateContractV1(2)),
    ) as any;
    mutate(value);
    expect(() => parseProgrammeGateContractV2(value)).toThrow();
  });

  it('rejects a V1 contract at the V2 boundary', () => {
    expect(() => parseProgrammeGateContractV2(createProgrammeGateContractV1(2))).toThrow();
  });
});

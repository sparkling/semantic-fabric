// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import { evaluateProgrammeGatesV5 } from '../src/programme-gates-v5.js';
import { evaluateProgrammeGatesV6 } from '../src/programme-gates-v6.js';
import { digestValue, type Receipt } from '../src/receipts.js';
import {
  declaredBuildCommands,
  programmeV6Fixture,
  rebindCandidateEvidence,
  rehashCandidateEvidence,
  rehashReceipt,
  type ProgrammeV6Fixture,
  type RepairDisposition,
} from './programme-v6-fixtures.js';

describe('schema-v6 transition-aware programme gates', () => {
  it.each([
    ['none', undefined],
    ['not-started', 'not-started'],
    ['failed', 'failed'],
    ['passed', 'passed'],
  ] as const)('accepts %s repair history with full replay evidence', (mode, expected) => {
    const fixture = programmeV6Fixture(mode);
    const result = evaluate(fixture);

    expect(result.dimensions.every(({ passed }) => passed)).toBe(true);
    expect(result.diagnostics.every(({ passed }) => passed)).toBe(true);
    expect(result.candidateTransactionEvidence.nativeRuntimeEvidence.schemaVersion).toBe(2);
    expect(result.candidateTransactionEvidence.repairTransitions[0]?.buildDisposition)
      .toBe(expected);
    expect(Object.isFrozen(result)).toBe(true);
  });

  it('preserves every V1/V5 outcome when there is no repair', () => {
    const fixture = programmeV6Fixture('none');
    const legacy = evaluateProgrammeGatesV5({
      policy: fixture.policy.base,
      receipt: fixture.receipt,
      diagnostics: fixture.diagnostics,
      rufloEvidence: fixture.rufloEvidence,
    });
    const current = evaluate(fixture);

    expect(current.dimensions.map(({ id, passed }) => [id, passed]))
      .toEqual(legacy.dimensions.map(({ id, passed }) => [id, passed]));
    expect(current.diagnostics).toEqual(legacy.diagnostics);
  });

  it('requires not-started attempts to contain no build commands', () => {
    const fixture = programmeV6Fixture('not-started');
    const tree = fixture.repairTransitions[0].sourceCandidate.tree;
    mutateReceipt(fixture, (receipt) => {
      receipt.commands.push(...declaredBuildCommands(fixture, 0, tree));
    });

    expect(reliability(evaluate(fixture))).toBe(false);
  });

  it.each([
    ['missing prefix', (receipt: Receipt) => {
      receipt.commands = receipt.commands.filter((command) =>
        command.stage !== 'build' || command.attempt !== 0);
    }],
    ['all-zero prefix', (receipt: Receipt) => {
      receipt.commands.find((command) =>
        command.stage === 'build' && command.attempt === 0)!.exitCode = 0;
    }],
    ['wrong source tree', (receipt: Receipt) => {
      receipt.commands.find((command) =>
        command.stage === 'build' && command.attempt === 0)!.candidateTree = 'e'.repeat(40);
    }],
    ['undeclared projection', (receipt: Receipt) => {
      receipt.commands.find((command) =>
        command.stage === 'build' && command.attempt === 0)!.argv.push('--tampered');
    }],
    ['abnormal completion', (receipt: Receipt) => {
      receipt.commands.find((command) =>
        command.stage === 'build' && command.attempt === 0)!.timedOut = true;
    }],
  ] as const)('rejects failed disposition with %s', (_name, mutate) => {
    const fixture = programmeV6Fixture('failed');
    mutateReceipt(fixture, mutate);
    expect(reliability(evaluate(fixture))).toBe(false);
  });

  it.each([
    ['missing declared build', (receipt: Receipt) => {
      receipt.commands = receipt.commands.filter((command) =>
        command.stage !== 'build' || command.attempt !== 0);
    }],
    ['nonzero build', (receipt: Receipt) => {
      receipt.commands.find((command) =>
        command.stage === 'build' && command.attempt === 0)!.exitCode = 1;
    }],
    ['extra declared build', (receipt: Receipt) => {
      const command = receipt.commands.find((entry) =>
        entry.stage === 'build' && entry.attempt === 0)!;
      receipt.commands.push(structuredClone(command));
    }],
    ['wrong source tree', (receipt: Receipt) => {
      receipt.commands.find((command) =>
        command.stage === 'build' && command.attempt === 0)!.candidateTree = 'e'.repeat(40);
    }],
  ] as const)('rejects passed disposition with %s', (_name, mutate) => {
    const fixture = programmeV6Fixture('passed');
    mutateReceipt(fixture, mutate);
    expect(reliability(evaluate(fixture))).toBe(false);
  });

  it('keeps all V1 final-attempt gates authoritative', () => {
    const verifier = programmeV6Fixture('not-started');
    mutateReceipt(verifier, (receipt) => {
      delete receipt.verifierDigests['attempt-1:public'];
    });
    expect(dimension(evaluate(verifier), 'patched-candidate-verification')).toBe(false);

    const build = programmeV6Fixture('not-started');
    mutateReceipt(build, (receipt) => {
      receipt.commands.find((command) =>
        command.stage === 'build' && command.attempt === 1)!.exitCode = 1;
    });
    const result = evaluate(build);
    expect(dimension(result, 'patched-candidate-verification')).toBe(false);
    // V1 assigns final command success to the patched-candidate dimension.
    expect(reliability(result)).toBe(true);
  });

  it('rejects receipt transplants and rehashed disposition claims', () => {
    const target = programmeV6Fixture('failed');
    const donor = programmeV6Fixture('not-started');
    target.candidateTransactionEvidence = donor.candidateTransactionEvidence;
    expect(() => evaluate(target)).toThrow('HARNESS_CANDIDATE_EVIDENCE_RECEIPT_BINDING_MISMATCH');

    const fixture = programmeV6Fixture('not-started');
    const evidence = structuredClone(fixture.candidateTransactionEvidence) as any;
    evidence.repairTransitions[0].buildDisposition = 'failed';
    rehashTransition(evidence.repairTransitions[0]);
    rehashCandidateEvidence(evidence);
    fixture.candidateTransactionEvidence = evidence;
    expect(() => evaluate(fixture)).toThrow('HARNESS_CANDIDATE_EVIDENCE_TRANSITION_BINDING_MISMATCH');
  });

  it('binds all V2 dimension digests to valid candidate evidence changes', () => {
    const fixture = programmeV6Fixture('failed');
    const original = evaluate(fixture);
    const evidence = structuredClone(fixture.candidateTransactionEvidence) as any;
    evidence.repairTransitions[0].reasonDigests[0] = digestValue('different reason');
    rehashTransition(evidence.repairTransitions[0]);
    rehashCandidateEvidence(evidence);
    fixture.candidateTransactionEvidence = evidence;
    const changed = evaluate(fixture);

    expect(changed.dimensions.map(({ passed }) => passed))
      .toEqual(original.dimensions.map(({ passed }) => passed));
    expect(changed.dimensions.map(({ evidenceDigest }) => evidenceDigest))
      .not.toEqual(original.dimensions.map(({ evidenceDigest }) => evidenceDigest));
  });

  it('rejects an over-policy repair count before parsing untrusted candidate evidence', () => {
    const fixture = programmeV6Fixture('none');
    const receipt = structuredClone(fixture.receipt) as Receipt;
    receipt.recovery.repairCount = 3;
    fixture.receipt = receipt;
    fixture.candidateTransactionEvidence = null as never;

    expect(() => evaluate(fixture)).toThrow('HARNESS_PROGRAMME_V6_REPAIR_COUNT_EXCEEDS_POLICY');
  });
});

function evaluate(fixture: ProgrammeV6Fixture) {
  return evaluateProgrammeGatesV6({
    policy: fixture.policy,
    receipt: fixture.receipt,
    diagnostics: fixture.diagnostics,
    rufloEvidence: fixture.rufloEvidence,
    candidateTransactionEvidence: fixture.candidateTransactionEvidence,
  });
}

function mutateReceipt(fixture: ProgrammeV6Fixture, mutate: (receipt: Receipt) => void): void {
  const receipt = structuredClone(fixture.receipt) as Receipt;
  mutate(receipt);
  fixture.receipt = rehashReceipt(receipt);
  rebindCandidateEvidence(fixture);
}

function reliability(result: ReturnType<typeof evaluate>): boolean {
  return dimension(result, 'reliability-and-receipts');
}

function dimension(
  result: ReturnType<typeof evaluate>,
  id: ReturnType<typeof evaluate>['dimensions'][number]['id'],
): boolean {
  return result.dimensions.find((entry) => entry.id === id)!.passed;
}

function rehashTransition(transition: Record<string, unknown>): void {
  const { digest: _digest, ...body } = transition;
  transition.digest = digestValue(body);
}

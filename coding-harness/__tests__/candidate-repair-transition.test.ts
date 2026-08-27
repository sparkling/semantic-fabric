// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  completeCandidateRepairTransitionReset,
  createCandidateRepairTransitionDraft,
  sealCandidateRepairTransitions,
  type CandidateRepairTransitionDraft,
} from '../src/candidate-repair-transition.js';
import type { NativeRuntimeEvidence } from '../src/evidence.js';
import { digestValue } from '../src/receipts.js';
import { digest, identity } from './candidate-fixtures.js';

describe('candidate repair transition evidence', () => {
  it('binds a contiguous repair to its replacement patch, clean reset, and native invocation', () => {
    const drafts = [draft()];
    completeCandidateRepairTransitionReset(drafts, 1, identity('2'), digest('b'));

    const [transition] = sealCandidateRepairTransitions(drafts, nativeEvidence());

    expect(transition).toMatchObject({
      schemaVersion: 1,
      fromAttempt: 0,
      toAttempt: 1,
      trigger: 'admission-validation',
      buildDisposition: 'not-started',
      sourcePatchDigest: digest('a'),
      replacementPatchDigest: digest('b'),
      sourceCandidate: identity('3'),
      repairResetIdentity: null,
      resetIdentity: identity('2'),
      reasonDigests: [digestValue('source mismatch')],
      nativeInvocation: {
        invocationId: 'repair-0001',
        operation: 'repair',
        candidateTree: identity('3').tree,
      },
    });
    const { digest: sealedDigest, ...body } = transition;
    expect(sealedDigest).toBe(digestValue(body));
  });

  it.each([
    ['missing reset', [draft()], nativeEvidence(), 'HARNESS_REPAIR_TRANSITION_RESET_MISSING'],
    ['wrong replacement', [draft()], nativeEvidence(), 'HARNESS_REPAIR_TRANSITION_REPLACEMENT_MISMATCH'],
  ] as const)('rejects %s evidence', (_label, drafts, runtime, expected) => {
    if (expected.endsWith('REPLACEMENT_MISMATCH')) {
      expect(() => completeCandidateRepairTransitionReset(
        drafts as CandidateRepairTransitionDraft[], 1, identity('2'), digest('c'),
      )).toThrow(expected);
      return;
    }
    expect(() => sealCandidateRepairTransitions(drafts, runtime)).toThrow(expected);
  });

  it('rejects attempt gaps, candidate mismatches, duplicate binding, and stray repair invocations', () => {
    const valid = completedDraft();
    const gap = [{ ...valid, fromAttempt: 1, toAttempt: 2 }];
    expect(() => sealCandidateRepairTransitions(gap, nativeEvidence()))
      .toThrow('HARNESS_REPAIR_TRANSITION_SEQUENCE_INVALID');

    expect(() => sealCandidateRepairTransitions([valid], nativeEvidence(identity('4').tree)))
      .toThrow('HARNESS_REPAIR_TRANSITION_CANDIDATE_MISMATCH');

    expect(() => sealCandidateRepairTransitions(
      [valid, { ...valid, fromAttempt: 1, toAttempt: 2 }], nativeEvidence(),
    )).toThrow('HARNESS_REPAIR_TRANSITION_NATIVE_INVOCATION_INVALID');

    expect(() => sealCandidateRepairTransitions(
      [valid], nativeEvidence(identity('3').tree, true),
    )).toThrow('HARNESS_REPAIR_TRANSITION_NATIVE_INVOCATION_UNBOUND');
  });

  it('requires exactly one pending predecessor for every non-initial reset', () => {
    expect(() => completeCandidateRepairTransitionReset([], 1, identity('2'), digest('b')))
      .toThrow('HARNESS_REPAIR_TRANSITION_PENDING_MISSING');
    expect(() => completeCandidateRepairTransitionReset([], 0, identity('2'), digest('a')))
      .not.toThrow();

    const drafts = [completedDraft()];
    expect(() => completeCandidateRepairTransitionReset(drafts, 1, identity('2'), digest('b')))
      .toThrow('HARNESS_REPAIR_TRANSITION_PENDING_MISSING');
  });

  it('requires the immediate clean reset only for failed patch admission', () => {
    const common = {
      fromAttempt: 0,
      sourcePatchDigest: digest('a'),
      replacementPatchDigest: digest('b'),
      sourceCandidate: identity('3'),
      reasons: ['failed'],
      repairInvocationId: 'repair-0001',
    };
    expect(() => createCandidateRepairTransitionDraft({
      ...common, phase: 'pre-admission', trigger: 'patch-admission',
    })).toThrow('HARNESS_REPAIR_TRANSITION_REPAIR_RESET_REQUIRED');
    expect(() => createCandidateRepairTransitionDraft({
      ...common,
      phase: 'post-admission',
      trigger: 'build',
      repairResetIdentity: identity('2'),
    })).toThrow('HARNESS_REPAIR_TRANSITION_REPAIR_RESET_UNEXPECTED');
  });

  it('derives phase and build disposition instead of trusting caller claims', () => {
    expect(() => createCandidateRepairTransitionDraft({
      fromAttempt: 0,
      phase: 'pre-admission',
      trigger: 'build',
      sourcePatchDigest: digest('a'),
      replacementPatchDigest: digest('b'),
      sourceCandidate: identity('3'),
      reasons: ['compiler failed'],
      repairInvocationId: 'repair-0001',
    })).toThrow('HARNESS_REPAIR_TRANSITION_PHASE_INVALID');

    expect(createCandidateRepairTransitionDraft({
      fromAttempt: 0,
      phase: 'post-admission',
      trigger: 'build',
      sourcePatchDigest: digest('a'),
      replacementPatchDigest: digest('b'),
      sourceCandidate: identity('3'),
      reasons: ['compiler failed'],
      repairInvocationId: 'repair-0001',
    }).buildDisposition).toBe('failed');
  });
});

function draft(): CandidateRepairTransitionDraft {
  return createCandidateRepairTransitionDraft({
    fromAttempt: 0,
    phase: 'post-admission',
    trigger: 'admission-validation',
    sourcePatchDigest: digest('a'),
    replacementPatchDigest: digest('b'),
    sourceCandidate: identity('3'),
    reasons: ['source mismatch'],
    repairInvocationId: 'repair-0001',
  });
}

function completedDraft(): CandidateRepairTransitionDraft {
  const drafts = [draft()];
  completeCandidateRepairTransitionReset(drafts, 1, identity('2'), digest('b'));
  return drafts[0];
}

function nativeEvidence(
  candidateTree = identity('3').tree,
  includeStray = false,
): NativeRuntimeEvidence {
  const invocation = (invocationId: string) => ({
    invocationId,
    host: 'claude-code' as const,
    model: 'claude-sonnet',
    operation: 'repair' as const,
    candidateTree,
    environmentDigest: digest('1'),
    outputDigest: digest('2'),
    exitCode: 0 as const,
    network: {
      enforcement: 'origin-pinned-process-boundary' as const,
      mechanism: 'test-firewall',
      pinnedOrigins: ['https://api.anthropic.com', 'https://claude.ai'],
      allowedConnections: 1,
      deniedConnections: 0,
      connectDigest: digest('3'),
    },
    filesystem: {
      enforcement: 'os-filesystem-namespace' as const,
      mechanism: 'test-namespace',
      workspaceRootDigest: digest('4'),
      mountManifestDigest: digest('5'),
      configurationMaskDigest: digest('6'),
      outputChannelDigest: digest('7'),
      hostFileConfidentiality: true as const,
      emptyPrivateHome: true as const,
      privateEphemeralHome: true as const,
      hostRootMounted: false as const,
      hostCredentialPathMounted: false as const,
      gitMetadataMasked: true as const,
    },
    resources: {
      enforcement: 'systemd-cgroup-v2' as const,
      mechanism: 'systemd-transient-service',
      limitsDigest: digest('8'),
    },
  });
  return {
    schemaVersion: 1,
    source: 'trusted-native-runtime',
    taskId: 'task',
    runId: 'run',
    hosts: [],
    invocations: includeStray
      ? [invocation('repair-0001'), invocation('repair-0002')]
      : [invocation('repair-0001')],
  };
}

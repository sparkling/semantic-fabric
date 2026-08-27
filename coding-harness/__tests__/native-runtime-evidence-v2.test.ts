// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import { bindNativeRuntimeEvidence } from '../src/evidence.js';
import { bindNativeRuntimeEvidenceV2 } from '../src/native-runtime-evidence-v2.js';
import { context, digest, identity } from './candidate-fixtures.js';

const expectations = [
  {
    invocationId: 'implementation-0001',
    host: 'codex' as const,
    operation: 'implementation' as const,
    candidateTree: identity('2').tree,
    patchPayloadSha256: digest('b'),
  },
  {
    invocationId: 'review-0001',
    host: 'claude-code' as const,
    operation: 'review' as const,
    candidateTree: identity('3').tree,
  },
] as const;

describe('native runtime evidence V2', () => {
  it('binds patch payload provenance while preserving exact V1 parsing', () => {
    const evidence = proof();
    const bound = bindNativeRuntimeEvidenceV2(binding(evidence));
    expect(bound.schemaVersion).toBe(2);
    expect(bound.invocations.map(({ patchPayloadSha256 }) => patchPayloadSha256))
      .toEqual([digest('b'), null]);
    expect(Object.isFrozen(bound)).toBe(true);

    const v1 = {
      ...evidence,
      schemaVersion: 1,
      invocations: evidence.invocations.map((invocation) => {
        const projected = { ...invocation };
        delete (projected as { patchPayloadSha256?: string | null }).patchPayloadSha256;
        return projected;
      }),
    };
    expect(bindNativeRuntimeEvidence(binding(v1)).schemaVersion).toBe(1);
  });

  it('does not downgrade schema-1 evidence into the V2 trust path', () => {
    expect(() => bindNativeRuntimeEvidenceV2(binding({ ...proof(), schemaVersion: 1 })))
      .toThrow('native runtime evidence V2 provenance is invalid');
  });

  it.each([
    ['missing patch digest', mutateInvocation(0, (entry) => {
      delete entry.patchPayloadSha256;
    }), /invalid keys/],
    ['extra invocation field', mutateInvocation(0, (entry) => {
      entry.untrusted = true;
    }), /invalid keys/],
    ['zero patch digest', mutateInvocation(0, (entry) => {
      entry.patchPayloadSha256 = '0'.repeat(64);
    }), /non-genesis SHA-256/],
    ['null patch digest', mutateInvocation(0, (entry) => {
      entry.patchPayloadSha256 = null;
    }), /non-genesis SHA-256/],
    ['non-patch digest', mutateInvocation(1, (entry) => {
      entry.patchPayloadSha256 = digest('c');
    }), /must be null/],
  ])('rejects %s', (_label, evidence, expected) => {
    expect(() => bindNativeRuntimeEvidenceV2(binding(evidence))).toThrow(expected);
  });

  it('rejects substitution against the transaction patch expectation', () => {
    const substituted = expectations.map((entry) => entry.operation === 'implementation'
      ? { ...entry, patchPayloadSha256: digest('c') }
      : entry);
    expect(() => bindNativeRuntimeEvidenceV2({
      ...binding(proof()), expectations: substituted,
    })).toThrow('HARNESS_NATIVE_PATCH_PAYLOAD_BINDING_MISMATCH');
  });
});

function binding(value: unknown) {
  return {
    value,
    taskId: context.taskId,
    runId: context.runId,
    hosts: context.hosts,
    expectations,
  };
}

function mutateInvocation(
  index: number,
  mutate: (entry: Record<string, unknown>) => void,
): unknown {
  const evidence = structuredClone(proof()) as unknown as {
    invocations: Record<string, unknown>[];
  };
  mutate(evidence.invocations[index]);
  return evidence;
}

function proof() {
  return {
    schemaVersion: 2,
    source: 'trusted-native-runtime',
    taskId: context.taskId,
    runId: context.runId,
    hosts: [
      host('codex', 'gpt-5', 'chatgpt-subscription', 'codex 1'),
      host('claude-code', 'claude-sonnet', 'claude-subscription', 'claude 1'),
    ],
    invocations: [
      invocation(
        'implementation-0001', 'codex', 'gpt-5', 'implementation',
        identity('2').tree, digest('b'),
      ),
      invocation(
        'review-0001', 'claude-code', 'claude-sonnet', 'review',
        identity('3').tree, null,
      ),
    ],
  };
}

function host(
  name: 'codex' | 'claude-code',
  model: string,
  authentication: 'chatgpt-subscription' | 'claude-subscription',
  clientVersion: string,
) {
  return {
    host: name, model, authentication, clientVersion,
    executablePath: `/tools/${name}`,
    executableDigest: digest(name === 'codex' ? '1' : '2'),
    preflightDigest: digest(name === 'codex' ? '3' : '4'),
    credentialCapability: 'invocation-private-copy',
    hostCredentialPathMounted: false,
  };
}

function invocation(
  invocationId: string,
  host: 'codex' | 'claude-code',
  model: string,
  operation: 'implementation' | 'review',
  candidateTree: string,
  patchPayloadSha256: string | null,
) {
  return {
    invocationId, host, model, operation, candidateTree,
    environmentDigest: digest('5'), outputDigest: digest('6'), patchPayloadSha256, exitCode: 0,
    network: {
      enforcement: 'origin-pinned-process-boundary', mechanism: 'test-firewall',
      pinnedOrigins: host === 'codex'
        ? ['https://api.openai.com', 'https://chatgpt.com']
        : ['https://api.anthropic.com', 'https://claude.ai'],
      allowedConnections: 1, deniedConnections: 0, connectDigest: digest('7'),
    },
    filesystem: {
      enforcement: 'os-filesystem-namespace', mechanism: 'test-namespace',
      workspaceRootDigest: digest('8'), mountManifestDigest: digest('9'),
      configurationMaskDigest: digest('a'), outputChannelDigest: digest('b'),
      hostFileConfidentiality: true, emptyPrivateHome: true, privateEphemeralHome: true,
      hostRootMounted: false, hostCredentialPathMounted: false, gitMetadataMasked: true,
    },
    resources: {
      enforcement: 'systemd-cgroup-v2', mechanism: 'systemd-transient-service',
      limitsDigest: digest('c'),
    },
  };
}

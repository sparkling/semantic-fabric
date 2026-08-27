// SPDX-License-Identifier: MIT

import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { NativeExecutionEvidence } from '../src/native-process.js';
import { createTrustedNativeRuntime, type TrustedNativeRuntime } from '../src/native-runtime.js';
import type { NativeAuthEvidence, NativeHost } from '../src/models/types.js';
import { digestValue } from '../src/receipts.js';
import { TEST_RESOURCE_LIMITS, createTestConfig } from './helpers.js';

const roots: string[] = [];
const runtimes: TrustedNativeRuntime[] = [];
const HOSTS = ['codex', 'claude-code'] as const;

afterEach(() => {
  vi.restoreAllMocks();
  for (const runtime of runtimes.splice(0)) runtime.cleanup();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('native runtime preflight executable identity', () => {
  it('requires both host preflights before exposing the snapshot', () => {
    const runtime = fixture();
    const preflights = mockPreflights(runtime);

    expect(() => runtime.ledger.preflightExecutableIdentitySnapshot())
      .toThrow('HARNESS_NATIVE_PREFLIGHT_IDENTITY_INCOMPLETE');
    runtime.ledger.recordPreflight(preflights.codex);
    expect(() => runtime.ledger.preflightExecutableIdentitySnapshot())
      .toThrow('HARNESS_NATIVE_PREFLIGHT_IDENTITY_INCOMPLETE');
  });

  it('exposes a deeply frozen, host-specific path and digest snapshot', () => {
    const runtime = fixture();
    const preflights = mockPreflights(runtime);
    runtime.ledger.recordPreflight(preflights.codex);
    runtime.ledger.recordPreflight(preflights['claude-code']);

    const snapshot = runtime.ledger.preflightExecutableIdentitySnapshot();
    expect(snapshot).toEqual(HOSTS.map((host) => ({
      host,
      path: runtime.runner.executableEvidence()[host].path,
      digest: runtime.runner.executableEvidence()[host].digest,
    })));
    expect(Object.isFrozen(snapshot)).toBe(true);
    expect(snapshot.every(Object.isFrozen)).toBe(true);
  });

  it('rejects an identity that is inconsistent with the trusted runner', () => {
    const runtime = fixture();
    const preflights = mockPreflights(runtime, 'codex');
    runtime.ledger.recordPreflight(preflights.codex);
    runtime.ledger.recordPreflight(preflights['claude-code']);

    expect(() => runtime.ledger.preflightExecutableIdentitySnapshot())
      .toThrow('HARNESS_NATIVE_PREFLIGHT_IDENTITY_INVALID');
  });

  it('does not expose the preflight snapshot after sealing', () => {
    const runtime = fixture();
    const preflights = mockPreflights(runtime);
    runtime.ledger.recordPreflight(preflights.codex);
    runtime.ledger.recordPreflight(preflights['claude-code']);
    const sealed = runtime.ledger.seal({
      taskId: 'task-1',
      runId: 'run-1',
      hosts: HOSTS.map((host) => ({
        host,
        model: model(host),
        role: `${host}-review`,
        clientVersion: `${host}-1.0.0`,
        authClass: host === 'codex'
          ? 'native-openai-subscription' as const
          : 'native-anthropic-subscription' as const,
        subscriptionCostUsd: 0 as const,
      })),
      expectations: [],
    });
    expect(sealed.hosts.every(({ hostCredentialPathMounted }) =>
      hostCredentialPathMounted === false)).toBe(true);

    expect(() => runtime.ledger.preflightExecutableIdentitySnapshot())
      .toThrow('HARNESS_NATIVE_RUNTIME_LEDGER_SEALED');
  });

  it('makes a failed seal terminal', () => {
    const runtime = fixture();
    const input = { taskId: 'task-1', runId: 'run-1', hosts: [], expectations: [] } as const;

    expect(() => runtime.ledger.seal(input))
      .toThrow('HARNESS_NATIVE_RUNTIME_HOST_COVERAGE_REQUIRED');
    expect(() => runtime.ledger.seal(input))
      .toThrow('HARNESS_NATIVE_RUNTIME_LEDGER_SEALED');
  });
});

describe('native runtime patch payload provenance', () => {
  it('emits schema V2 and enforces operation-compatible payload digests', () => {
    const runtime = fixture();
    const implementation = modelInvocationExecution(
      runtime, 'implementation-0001', 'codex', 'implementation',
    );
    const review = modelInvocationExecution(runtime, 'review-0001', 'claude-code', 'review');
    const preflights = mockPreflights(runtime, undefined, [implementation, review]);
    runtime.ledger.recordPreflight(preflights.codex);
    runtime.ledger.recordPreflight(preflights['claude-code']);

    expect(() => runtime.ledger.recordInvocation({
      invocationId: implementation.executionId,
      host: 'codex', model: model('codex'), operation: 'implementation',
      outputDigest: digestValue('implementation-output'), patchPayloadSha256: null,
    })).toThrow('HARNESS_NATIVE_PATCH_PAYLOAD_DIGEST_INVALID');
    expect(() => runtime.ledger.recordInvocation({
      invocationId: review.executionId,
      host: 'claude-code', model: model('claude-code'), operation: 'review',
      outputDigest: digestValue('review-output'), patchPayloadSha256: digestValue('unexpected'),
    })).toThrow('HARNESS_NATIVE_PATCH_PAYLOAD_DIGEST_UNEXPECTED');

    const patchPayloadSha256 = digestValue('exact-patch-payload');
    runtime.ledger.recordInvocation({
      invocationId: implementation.executionId,
      host: 'codex', model: model('codex'), operation: 'implementation',
      outputDigest: digestValue('implementation-output'), patchPayloadSha256,
    });
    runtime.ledger.recordInvocation({
      invocationId: review.executionId,
      host: 'claude-code', model: model('claude-code'), operation: 'review',
      outputDigest: digestValue('review-output'), patchPayloadSha256: null,
    });
    const sealed = runtime.ledger.seal({
      taskId: 'task-1', runId: 'run-1', hosts: hostEvidence(),
      expectations: [
        {
          invocationId: implementation.executionId, host: 'codex',
          operation: 'implementation', candidateTree: 'a'.repeat(40), patchPayloadSha256,
        },
        {
          invocationId: review.executionId, host: 'claude-code',
          operation: 'review', candidateTree: 'b'.repeat(40),
        },
      ],
    });
    expect(sealed.schemaVersion).toBe(2);
    expect(sealed.invocations.map(({ patchPayloadSha256: digest }) => digest))
      .toEqual([patchPayloadSha256, null]);
  });

  it('fails closed when the transaction expectation substitutes another patch digest', () => {
    const runtime = fixture();
    const implementation = modelInvocationExecution(
      runtime, 'implementation-0001', 'codex', 'implementation',
    );
    const review = modelInvocationExecution(runtime, 'review-0001', 'claude-code', 'review');
    const preflights = mockPreflights(runtime, undefined, [implementation, review]);
    runtime.ledger.recordPreflight(preflights.codex);
    runtime.ledger.recordPreflight(preflights['claude-code']);
    runtime.ledger.recordInvocation({
      invocationId: implementation.executionId,
      host: 'codex', model: model('codex'), operation: 'implementation',
      outputDigest: digestValue('implementation-output'),
      patchPayloadSha256: digestValue('exact-patch-payload'),
    });
    runtime.ledger.recordInvocation({
      invocationId: review.executionId,
      host: 'claude-code', model: model('claude-code'), operation: 'review',
      outputDigest: digestValue('review-output'), patchPayloadSha256: null,
    });

    expect(() => runtime.ledger.seal({
      taskId: 'task-1', runId: 'run-1', hosts: hostEvidence(),
      expectations: [
        {
          invocationId: implementation.executionId, host: 'codex',
          operation: 'implementation', candidateTree: 'a'.repeat(40),
          patchPayloadSha256: digestValue('substituted-patch-payload'),
        },
        {
          invocationId: review.executionId, host: 'claude-code',
          operation: 'review', candidateTree: 'b'.repeat(40),
        },
      ],
    })).toThrow('HARNESS_NATIVE_INVOCATION_BINDING_MISMATCH');
  });
});

function fixture(): TrustedNativeRuntime {
  const root = mkdtempSync(join(tmpdir(), 'native-runtime-ledger-'));
  roots.push(root);
  const runtimeParent = join(root, 'runtime');
  const workspace = join(root, 'workspace');
  const credentialsRoot = join(root, 'credentials');
  mkdirSync(runtimeParent, { mode: 0o700 });
  mkdirSync(workspace, { mode: 0o700 });
  mkdirSync(credentialsRoot, { mode: 0o700 });
  const codex = executable(root, 'codex');
  const claude = executable(root, 'claude');
  const proxyLauncher = join(root, 'proxy-launcher.mjs');
  const codexCredential = join(credentialsRoot, 'codex.json');
  const claudeCredential = join(credentialsRoot, 'claude.json');
  writeFileSync(proxyLauncher, 'export {};\n', { mode: 0o600 });
  writeFileSync(codexCredential, '{"auth":"codex"}\n', { mode: 0o600 });
  writeFileSync(claudeCredential, '{"auth":"claude"}\n', { mode: 0o600 });
  const trueExecutable = realpathSync('/bin/true');
  const runtime = createTrustedNativeRuntime({
    config: createTestConfig(),
    runtimeParent,
    allowedWorkspaceRoots: [workspace],
    workspaceRoot: workspace,
    executables: {
      codex,
      claude,
      node: realpathSync(process.execPath),
      bwrap: trueExecutable,
      systemdRun: trueExecutable,
      systemctl: trueExecutable,
      proxyLauncher,
    },
    credentials: { codex: codexCredential, 'claude-code': claudeCredential },
    resourceLimits: TEST_RESOURCE_LIMITS,
    controllerEnvironment: {
      DBUS_SESSION_BUS_ADDRESS: 'unix:path=/test/user-bus',
      XDG_RUNTIME_DIR: '/test/user-runtime',
    },
  });
  runtimes.push(runtime);
  return runtime;
}

function executable(root: string, name: string): string {
  const path = join(root, name);
  writeFileSync(path, '#!/bin/sh\nexit 0\n');
  chmodSync(path, 0o700);
  return path;
}

function mockPreflights(
  runtime: TrustedNativeRuntime,
  invalidHost?: NativeHost,
  additionalExecutions: readonly NativeExecutionEvidence[] = [],
): Readonly<Record<NativeHost, NativeAuthEvidence>> {
  const executions = new Map<string, NativeExecutionEvidence>();
  const evidence = Object.fromEntries(HOSTS.map((host) => {
    const ids = [`${host}:authentication`, `${host}:version`] as const;
    const identity = runtime.runner.executableEvidence()[host];
    const executableIdentity = host === invalidHost
      ? Object.freeze({ ...identity, digest: 'invalid' })
      : identity;
    executions.set(ids[0], execution(ids[0], host, 'authentication-preflight', executableIdentity));
    executions.set(ids[1], execution(ids[1], host, 'version-preflight', executableIdentity));
    return [host, Object.freeze({
      host,
      requestedModel: model(host),
      authentication: host === 'codex' ? 'chatgpt-subscription' : 'claude-subscription',
      clientVersion: `${host}-1.0.0`,
      fallback: 'none',
      subscriptionCostUsd: 0,
      preflightExecutionIds: ids,
    })];
  })) as unknown as Readonly<Record<NativeHost, NativeAuthEvidence>>;
  for (const item of additionalExecutions) executions.set(item.executionId, item);
  vi.spyOn(runtime.runner, 'executionEvidence').mockImplementation((id) => {
    const item = executions.get(id);
    if (item === undefined) throw new Error('unexpected execution id');
    return item;
  });
  return evidence;
}

function modelInvocationExecution(
  runtime: TrustedNativeRuntime,
  executionId: string,
  host: NativeHost,
  operation: 'implementation' | 'review',
): NativeExecutionEvidence {
  return Object.freeze({
    executionId, host, purpose: 'model-invocation', model: model(host), operation,
    executable: runtime.runner.executableEvidence()[host],
    environmentDigest: digestValue(`${executionId}:environment`), exitCode: 0,
    stdoutDigest: digestValue(`${executionId}:stdout`),
    stderrDigest: digestValue(`${executionId}:stderr`),
    network: Object.freeze({
      enforcement: 'origin-pinned-process-boundary', mechanism: 'test-origin-boundary',
      pinnedOrigins: host === 'codex'
        ? ['https://api.openai.com', 'https://chatgpt.com']
        : ['https://api.anthropic.com', 'https://claude.ai'],
      allowedConnections: 1, deniedConnections: 0,
      connectDigest: digestValue(`${executionId}:connect`),
    }),
    filesystem: Object.freeze({
      enforcement: 'os-filesystem-namespace', mechanism: 'test-filesystem-boundary',
      workspaceRootDigest: digestValue('workspace'),
      mountManifestDigest: digestValue(`${executionId}:mounts`),
      configurationMaskDigest: digestValue(`${executionId}:masks`),
      hostFileConfidentiality: true, emptyPrivateHome: true, privateEphemeralHome: true,
      hostRootMounted: false, hostCredentialPathMounted: false, gitMetadataMasked: true,
    }),
    resources: Object.freeze({
      enforcement: 'systemd-cgroup-v2', mechanism: 'systemd-transient-service',
      limitsDigest: digestValue(`${executionId}:limits`),
    }),
  });
}

function hostEvidence() {
  return HOSTS.map((host) => ({
    host, model: model(host), role: `${host}-review`, clientVersion: `${host}-1.0.0`,
    authClass: host === 'codex'
      ? 'native-openai-subscription' as const
      : 'native-anthropic-subscription' as const,
    subscriptionCostUsd: 0 as const,
  }));
}

function execution(
  executionId: string,
  host: NativeHost,
  purpose: 'authentication-preflight' | 'version-preflight',
  executableIdentity: Readonly<{ path: string; digest: string }>,
): NativeExecutionEvidence {
  return Object.freeze({
    executionId,
    host,
    purpose,
    model: model(host),
    operation: null,
    executable: executableIdentity,
    environmentDigest: digestValue(`${executionId}:environment`),
    exitCode: 0,
    stdoutDigest: digestValue(`${executionId}:stdout`),
    stderrDigest: digestValue(`${executionId}:stderr`),
    network: Object.freeze({
      enforcement: 'origin-pinned-process-boundary',
      mechanism: 'test-origin-boundary',
      pinnedOrigins: [],
      allowedConnections: 0,
      deniedConnections: 0,
      connectDigest: digestValue(`${executionId}:connect`),
    }),
    filesystem: Object.freeze({
      enforcement: 'os-filesystem-namespace',
      mechanism: 'test-filesystem-boundary',
      workspaceRootDigest: digestValue('workspace'),
      mountManifestDigest: digestValue(`${executionId}:mounts`),
      configurationMaskDigest: digestValue(`${executionId}:masks`),
      hostFileConfidentiality: true,
      emptyPrivateHome: true,
      privateEphemeralHome: true,
      hostRootMounted: false,
      hostCredentialPathMounted: false,
      gitMetadataMasked: false,
    }),
    resources: Object.freeze({
      enforcement: 'systemd-cgroup-v2',
      mechanism: 'systemd-transient-service',
      limitsDigest: digestValue(`${executionId}:limits`),
    }),
  });
}

function model(host: NativeHost): string {
  return host === 'codex' ? 'gpt-5.6' : 'claude-opus-4-1';
}

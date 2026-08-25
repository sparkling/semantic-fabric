// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { deepFreeze, SHA256_PATTERN } from './contracts.js';
import type {
  NativeInvocationExpectation,
  NativeModelOperation,
  NativeRuntimeEvidence,
} from './evidence.js';
import type { NativeAuthEvidence, NativeHost } from './models/types.js';
import { BoundedNativeProcessRunner, type NativeExecutionEvidence } from './native-process.js';
import type { HostEvidence } from './receipts.js';

export interface NativeInvocationRecord {
  readonly invocationId: string;
  readonly host: NativeHost;
  readonly model: string;
  readonly operation: NativeModelOperation;
  readonly outputDigest: string;
}

interface HostRecord {
  readonly evidence: NativeAuthEvidence;
  readonly executablePath: string;
  readonly executableDigest: string;
  readonly preflightDigest: string;
}

export class NativeRuntimeLedger {
  readonly #runner: BoundedNativeProcessRunner;
  readonly #hosts = new Map<NativeHost, HostRecord>();
  readonly #invocations = new Map<string, NativeInvocationRecord>();
  #sealed = false;

  constructor(runner: BoundedNativeProcessRunner) {
    if (!(runner instanceof BoundedNativeProcessRunner) || !runner.hasTrustedSystemBoundaries()) {
      throw new Error('HARNESS_NATIVE_TRUSTED_RUNNER_REQUIRED');
    }
    this.#runner = runner;
  }

  recordPreflight(evidence: NativeAuthEvidence): void {
    this.#assertOpen();
    if (this.#hosts.has(evidence.host)) throw new Error('HARNESS_NATIVE_PREFLIGHT_DUPLICATE');
    const executions = evidence.preflightExecutionIds.map((id) => this.#runner.executionEvidence(id));
    if (new Set(evidence.preflightExecutionIds).size !== 2
      || executions[0].purpose !== 'authentication-preflight'
      || executions[1].purpose !== 'version-preflight'
      || executions.some((entry) => entry.host !== evidence.host
        || entry.model !== evidence.requestedModel
        || entry.operation !== null
        || entry.exitCode !== 0
        || entry.network.deniedConnections !== 0)) {
      throw new Error('HARNESS_NATIVE_PREFLIGHT_EXECUTION_MISMATCH');
    }
    const executable = executions[0].executable;
    if (executions[1].executable.path !== executable.path
      || executions[1].executable.digest !== executable.digest) {
      throw new Error('HARNESS_NATIVE_PREFLIGHT_EXECUTABLE_MISMATCH');
    }
    this.#hosts.set(evidence.host, deepFreeze({
      evidence,
      executablePath: executable.path,
      executableDigest: executable.digest,
      preflightDigest: digest(executions),
    }));
  }

  recordInvocation(input: NativeInvocationRecord): void {
    this.#assertOpen();
    if (!SHA256_PATTERN.test(input.outputDigest) || input.outputDigest === '0'.repeat(64)) {
      throw new Error('HARNESS_NATIVE_OUTPUT_DIGEST_INVALID');
    }
    if (this.#invocations.has(input.invocationId)) {
      throw new Error('HARNESS_NATIVE_INVOCATION_DUPLICATE');
    }
    const execution = this.#runner.executionEvidence(input.invocationId);
    if (execution.purpose !== 'model-invocation'
      || execution.host !== input.host
      || execution.model !== input.model
      || execution.operation !== input.operation
      || execution.exitCode !== 0
      || execution.network.allowedConnections < 1
      || execution.network.deniedConnections !== 0
      || execution.filesystem.gitMetadataMasked !== true) {
      throw new Error('HARNESS_NATIVE_INVOCATION_EXECUTION_MISMATCH');
    }
    this.#invocations.set(input.invocationId, deepFreeze({ ...input }));
  }

  seal(input: Readonly<{
    taskId: string;
    runId: string;
    hosts: readonly HostEvidence[];
    expectations: readonly NativeInvocationExpectation[];
  }>): NativeRuntimeEvidence {
    this.#assertOpen();
    const expectedHosts = new Map(input.hosts.map((host) => [host.host, host]));
    if (input.hosts.length !== 2 || expectedHosts.size !== 2 || this.#hosts.size !== 2) {
      throw new Error('HARNESS_NATIVE_RUNTIME_HOST_COVERAGE_REQUIRED');
    }
    const hosts = (['codex', 'claude-code'] as const).map((host) => {
      const expected = expectedHosts.get(host);
      const actual = this.#hosts.get(host);
      if (expected === undefined || actual === undefined
        || expected.model !== actual.evidence.requestedModel
        || expected.clientVersion !== actual.evidence.clientVersion) {
        throw new Error('HARNESS_NATIVE_PREFLIGHT_BINDING_MISMATCH');
      }
      return deepFreeze({
        host,
        model: expected.model,
        authentication: actual.evidence.authentication,
        clientVersion: expected.clientVersion,
        executablePath: actual.executablePath,
        executableDigest: actual.executableDigest,
        preflightDigest: actual.preflightDigest,
        credentialCapability: 'invocation-private-copy' as const,
        hostCredentialPathMounted: false as const,
      });
    });
    const expectationIds = input.expectations.map(({ invocationId }) => invocationId);
    if (new Set(expectationIds).size !== expectationIds.length
      || expectationIds.length !== this.#invocations.size
      || expectationIds.some((id) => !this.#invocations.has(id))) {
      throw new Error('HARNESS_NATIVE_INVOCATION_SET_MISMATCH');
    }
    const invocations = input.expectations.map((expected) => {
      const recorded = this.#invocations.get(expected.invocationId) as NativeInvocationRecord;
      const execution = this.#runner.executionEvidence(expected.invocationId);
      if (recorded.operation !== expected.operation
        || (expected.host !== undefined && recorded.host !== expected.host)) {
        throw new Error('HARNESS_NATIVE_INVOCATION_BINDING_MISMATCH');
      }
      return invocationEvidence(recorded, execution, expected.candidateTree);
    });
    this.#sealed = true;
    return deepFreeze({
      schemaVersion: 1,
      source: 'trusted-native-runtime',
      taskId: nonEmpty(input.taskId, 'TASK_ID'),
      runId: nonEmpty(input.runId, 'RUN_ID'),
      hosts,
      invocations,
    });
  }

  #assertOpen(): void {
    if (this.#sealed) throw new Error('HARNESS_NATIVE_RUNTIME_LEDGER_SEALED');
  }
}

function invocationEvidence(
  recorded: NativeInvocationRecord,
  execution: NativeExecutionEvidence,
  candidateTree: string,
): NativeRuntimeEvidence['invocations'][number] {
  return deepFreeze({
    invocationId: recorded.invocationId,
    host: recorded.host,
    model: recorded.model,
    operation: recorded.operation,
    candidateTree,
    environmentDigest: execution.environmentDigest,
    outputDigest: recorded.outputDigest,
    exitCode: 0,
    network: {
      enforcement: execution.network.enforcement,
      mechanism: execution.network.mechanism,
      pinnedOrigins: [...execution.network.pinnedOrigins],
      allowedConnections: execution.network.allowedConnections,
      deniedConnections: 0,
      connectDigest: execution.network.connectDigest,
    },
    filesystem: {
      enforcement: execution.filesystem.enforcement,
      mechanism: execution.filesystem.mechanism,
      workspaceRootDigest: execution.filesystem.workspaceRootDigest,
      mountManifestDigest: execution.filesystem.mountManifestDigest,
      configurationMaskDigest: execution.filesystem.configurationMaskDigest,
      outputChannelDigest: recorded.outputDigest,
      hostFileConfidentiality: true,
      emptyPrivateHome: true,
      privateEphemeralHome: true,
      hostRootMounted: false,
      hostCredentialPathMounted: false,
      gitMetadataMasked: true,
    },
    resources: {
      enforcement: execution.resources.enforcement,
      mechanism: execution.resources.mechanism,
      limitsDigest: execution.resources.limitsDigest,
    },
  });
}

function digest(value: unknown): string {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex');
}

function nonEmpty(value: string, label: string): string {
  if (typeof value !== 'string' || value.trim() === '') throw new Error(`HARNESS_NATIVE_${label}_INVALID`);
  return value;
}

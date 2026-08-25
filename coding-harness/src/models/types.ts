// SPDX-License-Identifier: MIT

export type NativeHost = 'codex' | 'claude-code';
export type NativeProcessPurpose = 'authentication-preflight' | 'version-preflight' | 'model-invocation';
export type NativeProcessOperation = 'architecture' | 'implementation' | 'repair' | 'review';

export interface NativeProcessRequest {
  readonly host: NativeHost;
  readonly purpose: NativeProcessPurpose;
  readonly model: string;
  readonly operation?: NativeProcessOperation;
  readonly executable: string;
  readonly args: readonly string[];
  readonly cwd: string;
  readonly env: Readonly<Record<string, string>>;
  readonly timeoutMs: number;
  readonly stdin?: string;
  readonly readOnlyPaths?: readonly string[];
  readonly writablePaths?: readonly string[];
  readonly signal?: AbortSignal;
}

export interface NativeProcessResult {
  readonly executionId: string;
  readonly exitCode: number | null;
  readonly stdout: string;
  readonly stderr: string;
  readonly timedOut: boolean;
  readonly cancelled?: boolean;
  readonly outputLimitExceeded?: boolean;
  readonly spawnError?: string;
  readonly stdoutDigest: string;
  readonly stderrDigest: string;
}

/** Execution seam implemented by H-A's bounded process layer. */
export interface NativeProcessRunner {
  run(request: NativeProcessRequest): Promise<NativeProcessResult>;
}

export type NativeAuthentication =
  | 'chatgpt-subscription'
  | 'claude-subscription';

export interface NativeAuthEvidence {
  readonly host: NativeHost;
  readonly requestedModel: string;
  readonly authentication: NativeAuthentication;
  readonly clientVersion: string;
  readonly fallback: 'none';
  readonly subscriptionCostUsd: 0;
  readonly preflightExecutionIds: readonly [string, string];
}

export interface NativePreflightRequest {
  readonly cwd: string;
  readonly requestedModel: string;
  readonly signal?: AbortSignal;
}

export type WorkspaceAccess = 'read' | 'write';

interface NativeInvocationBase {
  readonly cwd: string;
  readonly model: string;
  readonly prompt: string;
  readonly schema: Readonly<Record<string, unknown>>;
  readonly workspaceAccess: WorkspaceAccess;
  readonly timeoutMs: number;
  readonly signal?: AbortSignal;
  readonly operation: NativeProcessOperation;
}

export interface CodexInvocationRequest extends NativeInvocationBase {
  readonly schemaPath: string;
  readonly outputPath: string;
}

export type ClaudeInvocationRequest = NativeInvocationBase;

export interface NativeSubscriptionAdapter {
  readonly host: NativeHost;
  preflight(request: NativePreflightRequest): Promise<NativeAuthEvidence>;
}

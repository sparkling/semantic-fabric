// SPDX-License-Identifier: MIT
import { randomUUID, createHash } from 'node:crypto';
import { closeSync, mkdirSync, openSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { hash } from '@metaharness/harness';
import { createDeliveryApi, parseDeliveryChanges, parseDeliveryPlan, renderDeliveryPrompt, type DeliveryPlan, type DeliverySourceFile } from './delivery-api.js';
import { ClaudeCodeSubscriptionAdapter, CodexSubscriptionAdapter } from './models/native-adapters.js';
import type { NativeProcessRunner } from './models/types.js';
import { ordinaryRustEnvironment, runCommand } from './delivery-process.js';
import { nativeCommandProgress } from './delivery-native-progress.js';
import { NativeStderrTail } from './delivery-native-stderr.js';
import { withOperationLock } from './delivery-workspace.js';
import { parseStageResponse, type NativeStageRequest, type NativeStageResponse } from './delivery-workflow-contracts.js';
import type { DeliveryHarness } from './delivery-runtime.js';

export interface DeliveryExecution { response: NativeStageResponse; changes: { path: string; content: string }[];
  plan?: DeliveryPlan; evidence?: unknown; evidencePath?: string }
export type DeliveryExecutor = (request: NativeStageRequest, files: DeliverySourceFile[], checks: unknown[], signal?: AbortSignal) => Promise<DeliveryExecution>;

export function claudeStructuredError(stdout: string, environment: Readonly<Record<string, string | undefined>>): string | undefined {
  let value: unknown;
  try { value = JSON.parse(stdout); } catch { return undefined; }
  if (!value || typeof value !== 'object' || !('is_error' in value) || value.is_error !== true
    || !('result' in value) || typeof value.result !== 'string') return undefined;
  const tail = new NativeStderrTail(environment);
  tail.push(Buffer.from(value.result));
  return tail.finish().replace(/[\x00-\x1f\x7f]/g, ' ').slice(0, 2000);
}

/** Reuse native argument/environment adapters and existing process-group custody. */
export function createDeliveryExecutor(harness: DeliveryHarness, taskDigest: string): DeliveryExecutor {
  const api = createDeliveryApi({ directory: harness.context.apiDirectory ?? join(harness.directory, 'api') });
  return async (request, files, checks, signal) => {
    if (request.route.host === 'openrouter') return api(request, files, checks, taskDigest, signal);
    const invocation = join(harness.directory, `native-${randomUUID()}`); mkdirSync(invocation, { mode: 0o700 });
    let lastError = '';
    let outputExhausted = false;
    let stalled = false;
    const runner: NativeProcessRunner = { run: async input => {
      const id = randomUUID(), processRoot = join(invocation, id); mkdirSync(processRoot, { mode: 0o700 });
      return withOperationLock(processRoot, async () => {
      const outPath = join(processRoot, 'stdout'), errPath = join(processRoot, 'stderr');
      const out = openSync(outPath, 'wx', 0o600), err = openSync(errPath, 'wx', 0o600);
      try {
        const args = input.host === 'codex' && input.purpose === 'model-invocation'
          ? [...input.args.slice(0, -1), ...Object.entries(ordinaryRustEnvironment({})).flatMap(([name, value]) =>
            ['-c', `shell_environment_policy.set.${name}=${JSON.stringify(value)}`]), input.args.at(-1)!] : input.args;
        const result = await runCommand([input.executable, ...args], input.cwd, ordinaryRustEnvironment(input.env), out, err,
          processRoot, { timeoutMs: input.timeoutMs }, input.signal, input.stdin,
          input.purpose === 'model-invocation' ? nativeCommandProgress({ directory: processRoot,
            taskId: request.taskId, stage: request.repair ? 'repair' : request.stage, host: input.host }) : undefined);
        stalled = result.error === 'native-inactivity';
        const stdout = readFileSync(outPath, 'utf8'), stderr = readFileSync(errPath, 'utf8');
        const structured = !result.error && result.signal === null && request.route.host === 'claude-code'
          ? claudeStructuredError(stdout, process.env) : undefined;
        if (structured !== undefined) {
          lastError = structured;
          outputExhausted = /(?:Claude's response exceeded the \d+ output token maximum|stop_reason["'\s:]+max_tokens|finish_reason["'\s:]+max_tokens)/i.test(structured);
        } else if (result.error || result.exitCode !== 0) lastError = [result.error, stderr.trim()].filter(Boolean).join(': ');
        const digest = (text: string) => createHash('sha256').update(text).digest('hex');
        return { executionId: `native-run:${id}`, exitCode: structured !== undefined && result.exitCode === 0 ? 1 : result.exitCode, stdout, stderr,
          timedOut: result.error === 'check-timeout', cancelled: result.error === 'cancelled',
          outputLimitExceeded: result.error === 'check-output-limit', ...(result.error ? { spawnError: result.error } : {}), stdoutDigest: digest(stdout), stderrDigest: digest(stderr) };
      } finally { closeSync(out); closeSync(err); }
    }); } };
    const adapter = request.route.host === 'codex'
      ? new CodexSubscriptionAdapter({ executable: 'codex', evidenceRoot: invocation, runner, sourceEnvironment: process.env })
      : new ClaudeCodeSubscriptionAdapter({ executable: 'claude', runner, sourceEnvironment: process.env });
    const properties = { outcome: { type: 'string', enum: ['completed', 'changes-requested'] }, summary: { type: 'string' },
      issues: { type: 'array', items: { type: 'string' } }, changes: { type: 'array', items: { type: 'object', additionalProperties: false,
        properties: { path: { type: 'string' }, content: { type: 'string' } }, required: ['path', 'content'] } },
      ...(request.stage === 'architecture' ? { plan: { type: 'object', additionalProperties: false,
        properties: { summary: { type: 'string' }, files: { type: 'array', items: { type: 'string' } }, tests: { type: 'array', items: { type: 'string' } } }, required: ['summary', 'files', 'tests'] } } : {}) };
    const schema = { type: 'object', additionalProperties: false, properties, required: Object.keys(properties) };
    const schemaPath = join(invocation, 'schema.json'), outputPath = join(invocation, 'response.json');
    writeFileSync(schemaPath, JSON.stringify(schema), { flag: 'wx', mode: 0o600 });
    const started = performance.now();
    try {
      await adapter.preflight({ cwd: harness.root, requestedModel: request.route.model, signal });
      const common = { cwd: harness.root, model: request.route.model, prompt: renderDeliveryPrompt(request, files, checks), schema,
        workspaceAccess: 'read' as const, timeoutMs: 1_800_000, signal,
        operation: request.stage === 'implementation' && request.repair ? 'repair' as const : request.stage,
        ...(request.route.effort === 'default' ? {} : { reasoningEffort: request.route.effort }) };
      const result = adapter.host === 'codex' ? await adapter.invoke({ ...common, schemaPath, outputPath }) : await adapter.invoke({ ...common, streamJson: true });
      const raw = adapter.host === 'codex' ? readFileSync(outputPath, 'utf8') : result.stdout;
      const envelope = JSON.parse(raw), value = adapter.host === 'claude-code' ? envelope.structured_output ?? JSON.parse(envelope.result) : envelope;
      const response = parseStageResponse({ schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
        native: { ...request.route, executorId: request.executorId ?? result.executionId, authentication: 'native-subscription',
          observation: `${adapter.host} ${request.route.model}; ${result.executionId}` },
        outcome: value.outcome, summary: value.summary, issues: value.issues,
        metering: { costUsd: 0, latencyMs: Math.round(performance.now() - started), evidenceDigest: hash(result) } });
      return { response, changes: parseDeliveryChanges(value.changes, request.scope, request.stage !== 'implementation'),
        ...(request.stage === 'architecture' ? { plan: parseDeliveryPlan(value.plan, request.scope) } : {}) };
    } catch (error) {
      const kind = stalled ? 'DELIVERY_NATIVE_STALLED' : outputExhausted ? 'DELIVERY_NATIVE_OUTPUT_EXHAUSTED' : 'DELIVERY_NATIVE_STOP';
      throw new Error(`${kind}:${adapter.host}:${request.route.model}:${error instanceof Error ? error.message : String(error)}${lastError ? `: ${lastError}` : ''}`, { cause: error });
    }
  };
}

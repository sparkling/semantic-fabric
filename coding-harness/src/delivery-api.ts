// SPDX-License-Identifier: MIT
import { createHash, randomUUID } from 'node:crypto';
import { existsSync, linkSync, mkdirSync, readFileSync, readdirSync, unlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { asRecord, assertExactKeys } from './contracts.js';
import { atomicJson, processIdentity, readJson } from './delivery-workspace.js';
import { parseStageResponse, type NativeStageRequest, type NativeStageResponse } from './delivery-workflow-contracts.js';

export const DELIVERY_API_DEFAULTS = Object.freeze({ model: 'deepseek/deepseek-v4.1-flash',
  maxRequestUsd: 1, maxTotalUsd: null, maxOutputTokens: 131072, timeoutMs: 1800000,
  maxResponseBytes: 2000000, promptPrice: 0.5, completionPrice: 2 });
const digest = (value: unknown): string => createHash('sha256').update(JSON.stringify(value)).digest('hex');
export const deliveryApiDigest = digest;
const activeRequests = new Set<string>();
export interface DeliverySourceFile { path: string; content: string | null }
export interface DeliveryApiEvidence {
  requestId: string; stageRequestId: string; taskDigest: string; packet: string; status: string;
  requestedModel: string; resolvedModel?: string; providerRequestId?: string;
  actualUsd: number | null; maximumUsd: number; inputTokens?: number; outputTokens?: number;
  owner?: { pid: number; start: string; boot: string };
  proposalDigest?: string;
}
export class DeliveryApiFailure extends Error {
  constructor(readonly code: string, readonly evidence: DeliveryApiEvidence) { super(code); }
}
function exclusiveJson(path: string, value: unknown): void {
  const temporary = `${path}.${randomUUID()}.tmp`;
  writeFileSync(temporary, JSON.stringify(value), { flag: 'wx', mode: 0o600 });
  try { linkSync(temporary, path); } finally { unlinkSync(temporary); }
}
export function renderDeliveryPrompt(request: NativeStageRequest, files: DeliverySourceFile[], checks: unknown[]): string {
  return JSON.stringify({ mode: request.stage, executionMode: 'packet-only', transport: request.route.host, toolsAvailable: false, taskId: request.taskId,
    requirement: request.requirement, sourceDigest: request.sourceDigest, scope: request.scope, files,
    ...(request.failedApi ? { priorApiFailure: { status: request.failedApi.status, actualUsd: request.failedApi.actualUsd,
      requestedModel: request.failedApi.requestedModel, resolvedModel: request.failedApi.resolvedModel } } : {}),
    ...(request.stage === 'review' ? { checks } : { feedback: request.feedback }),
    instruction: request.stage === 'review'
      ? 'Fresh independent review of current source and sanitized deterministic checks. No author rationale. Return actual JSON verdict; changes must be empty.'
      : 'Packet-only execution: do not use tools or edit source. Propose full UTF-8 file contents only for admitted paths. Return actual JSON instance, not schema.',
    response: { outcome: 'completed|changes-requested', summary: 'string', issues: ['string'], changes: [{ path: 'admitted path', content: 'full source' }] } });
}
export function parseDeliveryChanges(value: unknown, scope: string[], review = false): { path: string; content: string }[] {
  if (!Array.isArray(value) || value.length > scope.length || (review && value.length)) throw new Error('Invalid changes');
  const seen = new Set<string>();
  return value.map((item: unknown) => {
    const change = asRecord(item, 'API change'); assertExactKeys(change, ['path', 'content'], 'API change');
    if (typeof change.path !== 'string' || !scope.includes(change.path) || seen.has(change.path)
      || typeof change.content !== 'string' || Buffer.byteLength(change.content) > 512 * 1024) throw new Error('Change outside scope');
    seen.add(change.path); return { path: change.path, content: change.content };
  });
}
interface Envelope { id?: unknown; model?: unknown; error?: { code?: unknown };
  usage?: { cost?: unknown; prompt_tokens?: unknown; completion_tokens?: unknown };
  choices?: { finish_reason?: unknown; message?: { content?: unknown } }[] }
async function readEnvelope(response: Response): Promise<Envelope> {
  if (!response.body) throw new Error('Missing API response');
  const reader = response.body.getReader(); const chunks: Buffer[] = []; let bytes = 0;
  try {
    for (;;) {
      const next = await reader.read(); if (next.done) break;
      bytes += next.value.byteLength;
      if (bytes > DELIVERY_API_DEFAULTS.maxResponseBytes) throw new Error('API response overflow');
      chunks.push(Buffer.from(next.value));
    }
    return JSON.parse(Buffer.concat(chunks).toString('utf8')) as Envelope;
  } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
}

export function createDeliveryApi(options: { directory: string; fetch?: typeof fetch; apiKey?: () => string | undefined;
  observation?: (event: { phase: string; requestId: string }) => void }) {
  mkdirSync(options.directory, { recursive: true, mode: 0o700 });
  const boot = readFileSync('/proc/sys/kernel/random/boot_id', 'utf8').trim();
  return async (request: NativeStageRequest, files: DeliverySourceFile[], checks: unknown[], taskDigest: string,
    signal?: AbortSignal): Promise<{ response: NativeStageResponse; changes: { path: string; content: string }[];
      evidence: DeliveryApiEvidence; evidencePath: string }> => {
    const started = performance.now();
    if (request.route.host !== 'openrouter' || request.route.model !== DELIVERY_API_DEFAULTS.model || request.route.effort !== 'high') throw new Error('DELIVERY_API_ROUTE_REQUIRED');
    if (request.stage === 'implementation' && request.repair) throw new Error('DELIVERY_CAPABLE_NATIVE_REPAIR_REQUIRED');
    const prompt = renderDeliveryPrompt(request, files, checks);
    const maximumUsd = ((Buffer.byteLength(prompt) + 4096) * 0.5 + 131072 * 2) / 1e6;
    const evidence: DeliveryApiEvidence = { requestId: randomUUID(), stageRequestId: request.id, taskDigest, packet: request.stage,
      status: 'not-dispatched', requestedModel: request.route.model, actualUsd: null, maximumUsd };
    const fail = (code: string): never => { throw new DeliveryApiFailure(code, evidence); };
    if (signal?.aborted) fail('cancelled-before-dispatch');
    if (!Number.isFinite(maximumUsd) || maximumUsd > 1) fail('request-cost-bound');
    const apiKey = (options.apiKey ?? (() => process.env.OPENROUTER_API_KEY))();
    if (!apiKey) fail('api-key-unavailable');
    const unknown = join(options.directory, 'unknown-charge.json');
    if (existsSync(unknown)) fail('completion-unknown-held');
    let replay: DeliveryApiEvidence | undefined;
    for (const name of readdirSync(options.directory).filter(name => /^request-.*\.json$/.test(name))) {
      let previous: DeliveryApiEvidence;
      try { previous = readJson(join(options.directory, name)) as DeliveryApiEvidence; }
      catch { return fail('completion-unknown-held'); }
      if (previous.taskDigest === taskDigest && previous.stageRequestId === request.id) replay = previous;
      if (!['reserved', 'completion-unknown', 'completed-awaiting-validation'].includes(previous.status)) continue;
      let live = false;
      try { live = previous.status !== 'completion-unknown' && !!previous.owner?.start
        && previous.owner.boot === boot
        && processIdentity(previous.owner.pid) === previous.owner.start
        && (previous.owner.pid !== process.pid || activeRequests.has(join(options.directory, name))); } catch {}
      if (!live && previous.status === 'completed-awaiting-validation') {
        if (previous.taskDigest === taskDigest && previous.packet === request.stage) throw new DeliveryApiFailure('completed-validation-interrupted', previous);
      } else if (!live) fail('completion-unknown-held');
    }
    const holdPath = join(options.directory, `hold-${digest({ taskDigest, policy: DELIVERY_API_DEFAULTS, packet: request.stage })}.json`);
    if (existsSync(holdPath)) {
      const previous = readJson(holdPath) as DeliveryApiEvidence;
      throw new DeliveryApiFailure(previous.status === 'completed-invalid-output' ? 'task-output-held' : previous.status, previous);
    }
    if (replay) throw new DeliveryApiFailure('request-replay-refused', replay);
    const evidencePath = join(options.directory, `request-${digest({ taskDigest, requestId: request.id })}.json`);
    const start = processIdentity(process.pid);
    if (!start) fail('process-custody-unavailable');
    evidence.owner = { pid: process.pid, start: start!, boot };
    evidence.status = 'reserved';
    try { exclusiveJson(evidencePath, evidence); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'EEXIST') fail('request-replay-refused'); throw error; }
    const save = () => atomicJson(evidencePath, evidence);
    const hold = (path: string) => {
      try { exclusiveJson(path, evidence); }
      catch (error) { if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error; }
    };
    const emit = (phase: string) => { try { options.observation?.({ phase, requestId: evidence.requestId }); } catch {} };
    const controller = new AbortController();
    const abort = () => controller.abort(signal?.reason);
    signal?.addEventListener('abort', abort, { once: true });
    const timer = setTimeout(() => controller.abort(), DELIVERY_API_DEFAULTS.timeoutMs);
    activeRequests.add(evidencePath);
    try {
      emit('api-start');
      if (signal?.aborted) { evidence.status = 'cancelled-before-dispatch'; evidence.actualUsd = 0; save(); fail(evidence.status); }
      const response = await (options.fetch ?? fetch)('https://openrouter.ai/api/v1/chat/completions', {
        method: 'POST', signal: controller.signal, headers: { Authorization: `Bearer ${apiKey}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ model: request.route.model, messages: [{ role: 'user', content: prompt }], max_tokens: 131072,
          reasoning: { effort: 'high', exclude: true }, provider: { require_parameters: true, max_price: { prompt: 0.5, completion: 2 } },
          response_format: { type: 'json_object' }, stream: false }),
      });
      const body = await readEnvelope(response);
      const refused = body.error?.code === response.status && !body.id && !body.choices && !body.usage;
      if ([401, 403, 402].includes(response.status) && refused) {
        evidence.status = response.status === 402 ? 'confirmed-credit-rejection' : 'authentication-rejected';
        evidence.actualUsd = 0; save(); fail(evidence.status);
      }
      if (typeof body.id === 'string' && body.id) evidence.providerRequestId = body.id;
      if (typeof body.usage?.cost === 'number' && Number.isFinite(body.usage.cost) && body.usage.cost >= 0) evidence.actualUsd = body.usage.cost;
      if (typeof body.usage?.prompt_tokens === 'number') evidence.inputTokens = body.usage.prompt_tokens;
      if (typeof body.usage?.completion_tokens === 'number') evidence.outputTokens = body.usage.completion_tokens;
      if (!response.ok || !evidence.providerRequestId || evidence.actualUsd === null) throw new Error('Unconfirmed completion');
      evidence.resolvedModel = String(body.model); evidence.status = 'completed-awaiting-validation'; save();
      if (!['deepseek/deepseek-v4.1-flash', 'deepseek/deepseek-v4.1-flash-20260910'].includes(evidence.resolvedModel)) {
        evidence.status = 'completed-model-mismatch'; save(); hold(holdPath); fail(evidence.status);
      }
      let proposal: { response: NativeStageResponse; changes: { path: string; content: string }[] };
      try {
        if (body.choices?.[0]?.finish_reason !== 'stop' || typeof body.choices[0]?.message?.content !== 'string') throw new Error('Incomplete output');
        const value = asRecord(JSON.parse(body.choices[0].message.content), 'API proposal');
        assertExactKeys(value, ['outcome', 'summary', 'issues', 'changes'], 'API proposal');
        if (value.outcome !== 'completed' && value.outcome !== 'changes-requested') throw new Error('Invalid API outcome');
        const changes = parseDeliveryChanges(value.changes, request.scope, request.stage === 'review');
        const stageResponse = parseStageResponse({ schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
          native: { ...request.route, executorId: request.stage === 'implementation' ? request.executorId : `api-${evidence.requestId}`,
            authentication: 'openrouter-api', observation: `OpenRouter generation ${evidence.providerRequestId}; resolved ${evidence.resolvedModel}` },
          outcome: value.outcome, summary: value.summary, issues: value.issues,
          metering: { costUsd: evidence.actualUsd, latencyMs: performance.now() - started, evidenceDigest: digest(evidence) } });
        evidence.proposalDigest = digest({ outcome: stageResponse.outcome, summary: stageResponse.summary, issues: stageResponse.issues, changes });
        proposal = { response: stageResponse, changes };
      } catch { evidence.status = 'completed-invalid-output'; save(); hold(holdPath); return fail(evidence.status); }
      evidence.status = 'completed-valid-output'; save();
      proposal.response.metering!.evidenceDigest = digest(evidence);
      return { ...proposal, evidence, evidencePath };
    } catch (error) {
      if (error instanceof DeliveryApiFailure) throw error;
      evidence.status = 'completion-unknown'; save(); hold(unknown); return fail(evidence.status);
    } finally { activeRequests.delete(evidencePath); clearTimeout(timer); signal?.removeEventListener('abort', abort); emit('api-settled'); }
  };
}

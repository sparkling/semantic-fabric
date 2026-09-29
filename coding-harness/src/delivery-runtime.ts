// SPDX-License-Identifier: MIT
import { closeSync, existsSync, openSync, unlinkSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { hash, VerifierRegistry, type Verdict } from '@metaharness/harness';
import { parseDeliveryTask, selectDeliveryRoute, route, nonempty, identifier, executorIdentity, assertHostEnabled, parseDeliveryHandoff,
  type DeliveryTask, type DeliveryRoute, type NativeHandoff } from './delivery-contracts.js';
import { atomicJson, git, outsideDigest, readJson,
  withOperationLock, recoverOperation } from './delivery-workspace.js';
import { mainDeliveryContext, type DeliveryContext } from './delivery-context.js';
import { activeReservations, assertIntegrationReservations } from './delivery-cohort-custody.js';
import { resolveWorkspacePath } from './workspace.js';
import { buildCheckEnvironment, checkEnvironmentEvidence, logDigest, runCommand } from './delivery-process.js';
import { parseStageResponse, type DeliveryAction, type DeliveryWorkflow, type NativeStageRequest } from './delivery-workflow-contracts.js';
import { checkDigests, nextWorkflowAction, stageEvidenceDigest, workflowReady } from './delivery-workflow.js';
import { verifyNativeStage } from './delivery-stage.js';
import type { DeliveryApiEvidence } from './delivery-api.js';
import { applyIntegration, parseIntegrationInput, prepareIntegration, validateIntegrationEvidence, type CandidateIntegration } from './delivery-integration.js';

export interface CheckResult {
  id: string; attempt: number; argv: string[]; cwd: string; startedAt: string; durationMs: number;
  sourceBefore: string; sourceAfter: string; exitCode: number | null; signal: string | null;
  stdout: string; stderr: string; stdoutDigest: string; stderrDigest: string; error?: string;
  passed: boolean;
  environmentDigest: string;
}
export interface DeliveryRun {
  schemaVersion: 1; task: DeliveryTask; route: DeliveryRoute; baseCommit: string;
  outsideDigest: string; startedAt: string; updatedAt: string;
  status: 'awaiting-native' | 'active' | 'paused' | 'complete' | 'superseded';
  handoffs: (NativeHandoff & { at: string })[]; checks: CheckResult[];
  events: { at: string; kind: string; reason: string }[];
  adoptedSource: Record<string, string | null>;
  commit?: string; verdict?: Verdict; digest?: string;
  resolution?: { successorTask: string; successorCommit: string; reason: string; at: string };
  workflow?: DeliveryWorkflow;
  integration?: CandidateIntegration;
}

/** Main-only development harness. The native host still owns editing and spawning.
 * Operator-observed native identity is not cryptographic provider attestation.
 * Local records detect drift; they are neither managed Ruflo memory nor release proof. */
export class DeliveryHarness {
  readonly root: string;
  readonly directory: string;
  constructor(root: string, readonly context: DeliveryContext = mainDeliveryContext(root)) {
    context.assert();
    this.root = context.root;
    this.directory = context.directory;
  }
  snapshot() { this.context.assert(); return this.context.snapshot(); }
  private file(id: string): string { return join(this.directory, `${identifier(id)}.json`); }
  private get activeFile(): string { return join(this.directory, 'active.json'); }
  inspect(): { active: unknown; operation: unknown } {
    const operation = join(this.directory, 'operation.lock');
    return { active: existsSync(this.activeFile) ? readJson(this.activeFile) : null,
      operation: existsSync(operation) ? readJson(operation) : null };
  }
  read(id: string): DeliveryRun {
    const run = readJson(this.file(id)) as DeliveryRun;
    const { digest, ...body } = run;
    if (hash(body) !== digest || run.task.id !== id) throw new Error('DELIVERY_RECORD_DIGEST_MISMATCH');
    parseDeliveryTask(run.task);
    if (!['awaiting-native', 'active', 'paused', 'complete', 'superseded'].includes(run.status)) {
      throw new Error('DELIVERY_INVALID_STATUS');
    }
    if (run.status === 'superseded') {
      if (!run.resolution || identifier(run.resolution.successorTask) !== run.resolution.successorTask
        || !/^[a-f0-9]{40,64}$/.test(run.resolution.successorCommit)) {
        throw new Error('DELIVERY_INVALID_RESOLUTION');
      }
      nonempty(run.resolution.reason, 'resolution reason'); nonempty(run.resolution.at, 'resolution timestamp');
    } else if (run.resolution !== undefined) throw new Error('DELIVERY_INVALID_RESOLUTION');
    return run;
  }
  private save(run: DeliveryRun): DeliveryRun {
    run.updatedAt = new Date().toISOString();
    delete run.digest;
    run.digest = hash(run);
    atomicJson(this.file(run.task.id), run);
    return run;
  }
  private claim(id: string): void {
    if (existsSync(this.activeFile)) throw new Error('DELIVERY_WRITER_ALREADY_CLAIMED');
    atomicJson(this.activeFile, { id });
  }
  private own(run: DeliveryRun, owner: string, requireHandoff = true): void {
    this.context.assert();
    if (owner !== run.task.owner || !existsSync(this.activeFile)
      || (readJson(this.activeFile) as { id: string }).id !== run.task.id) throw new Error('DELIVERY_WRITER_MISMATCH');
    if (run.status !== 'active' && run.status !== 'awaiting-native') throw new Error('DELIVERY_RUN_NOT_ACTIVE');
    if (requireHandoff && run.status !== 'active') throw new Error('DELIVERY_NATIVE_HANDOFF_REQUIRED');
  }
  private source(run: DeliveryRun): string {
    if (this.context.head() !== run.baseCommit) throw new Error('DELIVERY_BASE_MOVED');
    const snapshot = this.snapshot();
    if (outsideDigest(snapshot, run.task.scope) !== run.outsideDigest) throw new Error('DELIVERY_OUT_OF_SCOPE_CHANGE');
    return snapshot.digest;
  }
  async begin(input: unknown): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      this.context.assert();
      const task = parseDeliveryTask(input);
      if (this.context.kind === 'main' && activeReservations(this).length) throw new Error('DELIVERY_ACTIVE_CANDIDATES_REQUIRE_INTEGRATION');
      assertHostEnabled(task.host);
      if (task.reviewer) assertHostEnabled(task.reviewer.host);
      if (existsSync(this.file(task.id))) throw new Error('DELIVERY_RUN_ALREADY_EXISTS');
      const snapshot = this.snapshot();
      const dirty = new Set(this.context.dirty(task.scope));
      if (hash([...dirty].sort()) !== hash([...(task.adoptExistingChanges ?? [])].sort())) {
        throw new Error('DELIVERY_EXISTING_SCOPE_REQUIRES_EXPLICIT_ADOPTION');
      }
      for (const path of task.scope) {
        resolveWorkspacePath(this.root, path, { allowMissingLeaf: true, requireRegularFile: true });
      }
      const now = new Date().toISOString();
      const run: DeliveryRun = { schemaVersion: 1, task, route: selectDeliveryRoute(task),
        baseCommit: this.context.head(), outsideDigest: outsideDigest(snapshot, task.scope),
        startedAt: now, updatedAt: now, status: 'awaiting-native', handoffs: [], checks: [], events: [],
        workflow: { requests: [], results: [], invalidated: [] },
        adoptedSource: Object.fromEntries([...dirty].map(p => [p, snapshot.files[p] ?? null])) };
      if (existsSync(this.activeFile)) throw new Error('DELIVERY_WRITER_ALREADY_CLAIMED');
      this.save(run); this.claim(task.id); return run;
    });
  }
  /** Explicit sole-integrator adoption; original model attestations remain unchanged. */
  async integrate(value: unknown): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      this.context.assert();
      const input = parseIntegrationInput(value);
      if (this.context.kind !== 'main') throw new Error('DELIVERY_CANONICAL_SOURCE_REQUIRED');
      const existing = existsSync(this.file(input.id)) ? this.read(input.id) : undefined;
      if (existing && !existing.integration) throw new Error('DELIVERY_RUN_ALREADY_EXISTS');
      const active = this.inspect().active as { id: string } | null;
      if (active && active.id !== input.id) throw new Error('DELIVERY_WRITER_ALREADY_CLAIMED');
      const run = existing ?? await prepareIntegration(this, input);
      if (run.task.owner !== input.owner) throw new Error('DELIVERY_INTEGRATION_OWNER_MISMATCH');
      if (run.status !== 'active') throw new Error('DELIVERY_RUN_NOT_ACTIVE');
      if (!existing) this.save(run);
      if (!active) this.claim(input.id);
      applyIntegration(this, run, input);
      return this.save(run);
    });
  }
  async bind(id: string, owner: string, handoff: NativeHandoff): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.own(run, owner, false); this.source(run);
      if (run.status !== 'awaiting-native') throw new Error('DELIVERY_HANDOFF_ALREADY_BOUND');
      const parsed = parseDeliveryHandoff(handoff);
      const selected = route({ host: parsed.host, model: parsed.model, effort: parsed.effort });
      assertHostEnabled(selected.host);
      if (hash(selected) !== hash(run.route)) {
        throw new Error('DELIVERY_NATIVE_ROUTE_MISMATCH');
      }
      executorIdentity(handoff.executorId); nonempty(handoff.observation, 'native observation');
      run.handoffs.push({ ...selected, executorId: handoff.executorId, authentication: handoff.authentication,
        observation: handoff.observation, at: new Date().toISOString() });
      run.status = 'active';
      return this.save(run);
    });
  }
  async fallback(id: string, owner: string, requestId: string, evidencePath: string, handoff: NativeHandoff): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.own(run, owner);
      const source = this.source(run), workflow = run.workflow;
      const previous = workflow?.requests.find(request => request.id === requestId);
      if (!workflow || !previous || previous.route.host !== 'openrouter'
        || workflow.invalidated.includes(requestId) || workflow.results.some(result => result.request.id === requestId)
        || previous.sourceDigest !== source || previous.evidenceDigest !== stageEvidenceDigest(run)) throw new Error('DELIVERY_PENDING_API_REQUEST_REQUIRED');
      await this.assertPrerequisites(run, previous);
      const path = resolve(evidencePath);
      if (dirname(path) !== (this.context.apiDirectory ?? join(this.directory, 'api')) || !/^request-[a-f0-9]{64}\.json$/.test(basename(path))) throw new Error('DELIVERY_API_EVIDENCE_PATH');
      const evidence = readJson(path) as DeliveryApiEvidence;
      if (evidence.stageRequestId !== requestId || evidence.taskDigest !== hash(run.task)
        || evidence.packet !== previous.stage || evidence.requestedModel !== previous.route.model
        || typeof evidence.actualUsd !== 'number' || !Number.isFinite(evidence.actualUsd) || evidence.actualUsd < 0) throw new Error('DELIVERY_API_EVIDENCE_MISMATCH');
      const native = parseDeliveryHandoff(handoff);
      const credit = evidence.status === 'confirmed-credit-rejection' && evidence.actualUsd === 0;
      const repair = evidence.status === 'completed-invalid-output';
      const eligible = repair
        ? native.host === 'claude-code' && native.model === 'cc/claude-opus-5-5[1m]' && native.effort === 'high'
        : credit && native.effort === 'medium' && ((native.host === 'codex' && native.model === 'gpt-5.6-sol')
          || (native.host === 'claude-code' && native.model === 'cc/claude-sonnet-5[1m]'));
      if (!eligible) throw new Error('DELIVERY_API_FALLBACK_NOT_AUTHORIZED');
      assertHostEnabled(native.host);
      if (run.handoffs.some(author => author.executorId === native.executorId)) throw new Error('DELIVERY_FRESH_FALLBACK_EXECUTOR_REQUIRED');
      this.reroute(run, previous, native, evidence);
      run.events.push({ at: new Date().toISOString(), kind: 'api-fallback', reason: `${requestId}:${evidence.status}:${native.host}:${native.model}` });
      delete run.verdict;
      return this.save(run);
    });
  }
  private reroute(run: DeliveryRun, previous: NativeStageRequest, native: NativeHandoff, failedApi?: DeliveryApiEvidence): void {
    const workflow = run.workflow!, selected = route({ host: native.host, model: native.model, effort: native.effort });
    workflow.invalidated.push(previous.id);
    const { id: _id, ...body } = previous;
    const nextBody = { ...body, route: selected, attempt: workflow.requests.length + 1,
      ...(previous.stage === 'implementation' ? { executorId: native.executorId } : {}), ...(failedApi ? { failedApi } : {}) };
    workflow.requests.push({ ...nextBody, id: hash(nextBody) });
    if (previous.stage === 'implementation') { run.route = selected; run.handoffs.push({ ...native, at: new Date().toISOString() }); }
  }
  private async assertPrerequisites(run: DeliveryRun, request: NativeStageRequest): Promise<void> {
    const references = new Set(request.prerequisiteDigests);
    const results = new Map((run.workflow?.results ?? []).map(result => [hash(result), result]));
    // A rejected repair can reference another repair; retain every original check.
    for (const reference of references) {
      for (const prerequisite of results.get(reference)?.request.prerequisiteDigests ?? []) references.add(prerequisite);
    }
    for (const check of run.checks.filter(check => references.has(hash(check)))) {
      if (check.environmentDigest !== hash(checkEnvironmentEvidence(buildCheckEnvironment()))) throw new Error('DELIVERY_STALE_PREREQUISITES');
      for (const [path, digest] of [[check.stdout, check.stdoutDigest], [check.stderr, check.stderrDigest]]) {
        if (!existsSync(path) || await logDigest(path) !== digest) throw new Error('DELIVERY_STALE_PREREQUISITES');
      }
    }
  }
  async repair(id: string, owner: string, handoff: NativeHandoff): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.own(run, owner); const source = this.source(run), workflow = run.workflow;
      const pending = workflow?.requests.find(request => !workflow.invalidated.includes(request.id)
        && !workflow.results.some(result => result.request.id === request.id));
      if (!pending?.repair || pending.stage !== 'implementation'
        || pending.sourceDigest !== source || pending.evidenceDigest !== stageEvidenceDigest(run)) throw new Error('DELIVERY_CAPABILITY_REPAIR_REQUIRED');
      await this.assertPrerequisites(run, pending);
      const native = parseDeliveryHandoff(handoff);
      if (native.host !== 'claude-code' || native.model !== 'cc/claude-opus-5-5[1m]' || native.effort !== 'high'
        || run.handoffs.some(author => author.executorId === native.executorId)) throw new Error('DELIVERY_CAPABLE_FRESH_NATIVE_REQUIRED');
      assertHostEnabled(native.host); this.reroute(run, pending, native);
      run.events.push({ at: new Date().toISOString(), kind: 'capability-repair', reason: pending.id });
      delete run.verdict; return this.save(run);
    });
  }
  async check(id: string, owner: string, checkId: string, signal?: AbortSignal): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.own(run, owner);
      const sourceBefore = this.source(run);
      const check = run.task.checks.find(c => c.id === checkId);
      if (!check) throw new Error('DELIVERY_UNKNOWN_CHECK');
      const cwd = resolveWorkspacePath(this.root, check.cwd, { allowRoot: true, requireDirectory: true });
      if (check.argv[0] === 'node') resolveWorkspacePath(cwd, check.argv[1], { requireRegularFile: true });
      const env = buildCheckEnvironment();
      const attempt = run.events.filter(e => e.kind === 'check-start' && e.reason.startsWith(`${check.id}:`)).length + 1;
      const logName = `${id}-${check.id}-${attempt}`;
      const stdout = join(this.directory, `${logName}.stdout`);
      const stderr = join(this.directory, `${logName}.stderr`);
      const startedAt = new Date().toISOString();
      run.events.push({ at: startedAt, kind: 'check-start', reason: `${check.id}:${attempt}` });
      this.save(run); // A crash leaves a start without a successful result, never a fabricated pass.
      const out = openSync(stdout, 'wx', 0o600), err = openSync(stderr, 'wx', 0o600);
      const start = performance.now();
      let result: { exitCode: number | null; signal: string | null; error?: string };
      try {
        result = await runCommand(check.argv, cwd, env, out, err, this.directory, check, signal);
      } finally { closeSync(out); closeSync(err); }
      let sourceAfter: string;
      try { this.context.assert(); sourceAfter = this.source(run); }
      catch (e) { sourceAfter = 'invalid'; result.error = String(e); }
      run.checks.push({ ...check, attempt, startedAt, durationMs: Math.round(performance.now() - start),
        sourceBefore, sourceAfter, environmentDigest: hash(checkEnvironmentEvidence(env)), ...result, stdout, stderr,
        stdoutDigest: await logDigest(stdout), stderrDigest: await logDigest(stderr),
        passed: result.exitCode === 0 && result.signal === null && !result.error && sourceBefore === sourceAfter });
      delete run.verdict;
      return this.save(run);
    });
  }
  private async validChecks(run: DeliveryRun, digest: string): Promise<Set<string>> {
    const valid = new Set<string>();
    for (const check of run.task.checks) {
      const latest = run.checks.filter(c => c.id === check.id).at(-1);
      const start = run.events.filter(e => e.kind === 'check-start' && e.reason.startsWith(`${check.id}:`)).at(-1);
      if (!latest?.passed || start?.reason !== `${check.id}:${latest.attempt}`
        || latest.sourceBefore !== digest || latest.sourceAfter !== digest
        || latest.environmentDigest !== hash(checkEnvironmentEvidence(buildCheckEnvironment()))) continue;
      let intact = true;
      for (const [path, expected] of [[latest.stdout, latest.stdoutDigest], [latest.stderr, latest.stderrDigest]]) {
        if (!existsSync(path) || await logDigest(path) !== expected) intact = false;
      }
      if (intact) valid.add(check.id);
    }
    return valid;
  }
  private async verdict(run: DeliveryRun, digest: string): Promise<Verdict> {
    await validateIntegrationEvidence(run);
    const valid = await this.validChecks(run, digest);
    const registry = new VerifierRegistry();
    for (const check of run.task.checks) registry.register({ id: check.id, kind: 'delivery', check: async () => {
      const failed = { pass: false, score: 0, reasons: [`${check.id}: missing, failed, stale, or changed evidence`] };
      return valid.has(check.id) ? { pass: true, score: 1, reasons: [] } : failed;
    } });
    // Historical records remain readable; new begins always require the workflow.
    if (run.workflow) registry.register({ id: 'native-workflow', kind: 'delivery', check: async () => ({
      pass: workflowReady(run, digest), score: workflowReady(run, digest) ? 1 : 0,
      reasons: workflowReady(run, digest) ? [] : ['missing or stale implementation/independent review'],
    }) });
    if (!registry.forKinds().length) throw new Error('DELIVERY_EMPTY_VERIFIER_SET');
    return registry.run(run, undefined, ['delivery']);
  }
  async next(id: string, owner: string): Promise<DeliveryAction> {
    return withOperationLock(this.directory, async () => {
      const run = this.read(id);
      if (run.task.owner !== owner) throw new Error('DELIVERY_WRITER_MISMATCH');
      if (run.status === 'paused') return { kind: 'paused', reason: run.events.at(-1)?.reason ?? 'paused' };
      this.own(run, owner);
      const source = this.source(run);
      const action = nextWorkflowAction(run, source, await this.validChecks(run, source));
      if (action.kind === 'native') await this.assertPrerequisites(run, action.request);
      this.save(run); return action;
    });
  }
  async advance(id: string, owner: string, signal?: AbortSignal): Promise<DeliveryAction> {
    for (;;) {
      const action = await this.next(id, owner);
      if (action.kind !== 'check') return action;
      const run = await this.check(id, owner, action.checkId, signal);
      if (signal?.aborted || run.checks.at(-1)?.error === 'cancelled') {
        await this.pause(id, owner, 'deterministic check cancelled; explicit resume required');
        return { kind: 'paused', reason: 'deterministic check cancelled' };
      }
    }
  }
  async submit(id: string, owner: string, input: unknown): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.own(run, owner);
      const source = this.source(run), response = parseStageResponse(input);
      const workflow = run.workflow;
      const request = workflow?.requests.find(r => r.id === response.requestId);
      if (!workflow || !request || workflow.invalidated.includes(request.id)
        || workflow.results.some(r => r.request.id === request.id)) throw new Error('DELIVERY_PENDING_REQUEST_REQUIRED');
      if (request.evidenceDigest !== stageEvidenceDigest(run)) throw new Error('DELIVERY_STALE_PREREQUISITES');
      await this.assertPrerequisites(run, request);
      if (response.sourceDigest !== source) throw new Error('DELIVERY_RESPONSE_SOURCE_MISMATCH');
      if (response.rootApplication && (response.rootApplication.sourceBefore !== request.sourceDigest
        || response.rootApplication.rootChangedPaths.some(path => !request.scope.includes(path)))) throw new Error('DELIVERY_INVALID_ROOT_APPLICATION');
      const native = response.native;
      assertHostEnabled(native.host);
      if (hash(route({ host: native.host, model: native.model, effort: native.effort })) !== hash(request.route)) {
        throw new Error('DELIVERY_NATIVE_ROUTE_MISMATCH');
      }
      if (request.stage === 'implementation' && (native.executorId !== request.executorId
        || native.executorId !== run.handoffs.at(-1)?.executorId)) throw new Error('DELIVERY_IMPLEMENTATION_EXECUTOR_MISMATCH');
      if (request.stage === 'review') {
        if (run.handoffs.some(h => h.executorId === native.executorId)) throw new Error('DELIVERY_INDEPENDENT_REVIEW_REQUIRED');
        const impl = workflow.results.filter(r => r.request.stage === 'implementation'
          && r.response.outcome !== 'unavailable' && r.response.outcome !== 'cancelled').at(-1);
        if (!impl?.accepted || impl.response.sourceDigest !== source || request.sourceDigest !== source
          || (await this.validChecks(run, source)).size !== run.task.checks.length
          || hash(request.prerequisiteDigests) !== hash([hash(impl), ...checkDigests(run)])) {
          throw new Error('DELIVERY_STALE_REVIEW');
        }
      }
      const noProgress = request.stage === 'implementation' && request.repair
        && request.sourceDigest === source && response.outcome !== 'unavailable' && response.outcome !== 'cancelled';
      const reasons = noProgress ? ['DELIVERY_REPAIR_NO_PROGRESS: change the scoped source before resubmitting a repair'] : [];
      if (request.stage === 'implementation' && response.outcome === 'changes-requested') reasons.push('implementation did not complete');
      workflow.results.push(await verifyNativeStage(request, response, reasons));
      delete run.verdict;
      if (noProgress || response.outcome === 'unavailable' || response.outcome === 'cancelled') {
        run.status = 'paused';
        run.events.push({ at: new Date().toISOString(), kind: 'pause', reason:
          `${native.host} ${native.model}: ${reasons.join('; ') || response.summary}` });
        this.save(run); unlinkSync(this.activeFile); return run;
      }
      return this.save(run);
    });
  }
  async verify(id: string, owner: string): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.own(run, owner);
      run.verdict = await this.verdict(run, this.source(run));
      return this.save(run);
    });
  }
  async finish(id: string, owner: string, commit: string): Promise<DeliveryRun> {
    if (this.context.kind !== 'main') throw new Error('DELIVERY_CANDIDATE_CANNOT_COMMIT');
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.own(run, owner);
      if (!/^[a-f0-9]{40,64}$/.test(commit) || git(this.root, 'rev-parse', 'HEAD') !== commit
        || git(this.root, 'show', '-s', '--format=%P', commit) !== run.baseCommit) throw new Error('DELIVERY_EXACT_COMMIT_REQUIRED');
      const changed = git(this.root, 'diff-tree', '--no-commit-id', '--name-only', '-r', commit).split('\n').filter(Boolean);
      if (!changed.length || changed.some(p => !run.task.scope.includes(p))) throw new Error('DELIVERY_COMMIT_SCOPE_MISMATCH');
      if (git(this.root, 'diff', commit, '--', ...run.task.scope) !== '') throw new Error('DELIVERY_UNCOMMITTED_SCOPE');
      const snapshot = this.snapshot();
      const untracked = git(this.root, 'ls-files', '--others', '--exclude-standard', '-z').split('\0');
      if (run.task.scope.some(p => untracked.includes(p))) throw new Error('DELIVERY_UNCOMMITTED_SCOPE');
      if (outsideDigest(snapshot, run.task.scope) !== run.outsideDigest) throw new Error('DELIVERY_OUT_OF_SCOPE_CHANGE');
      run.verdict = await this.verdict(run, snapshot.digest);
      if (!run.verdict.pass) { this.save(run); throw new Error('DELIVERY_VERIFICATION_FAILED'); }
      run.commit = commit; run.status = 'complete'; this.save(run); unlinkSync(this.activeFile);
      return run;
    });
  }
  async pause(id: string, owner: string, reason: string): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.own(run, owner, false);
      run.events.push({ at: new Date().toISOString(), kind: 'pause', reason: nonempty(reason, 'reason') });
      run.status = 'paused'; this.save(run); unlinkSync(this.activeFile); return run;
    });
  }
  async resume(id: string, owner: string): Promise<DeliveryRun> {
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.context.assert();
      if (run.status !== 'paused' || run.task.owner !== owner) throw new Error('DELIVERY_PAUSED_OWNER_REQUIRED');
      if (this.context.kind === 'main' && !run.integration && activeReservations(this).length) throw new Error('DELIVERY_ACTIVE_CANDIDATES_REQUIRE_INTEGRATION');
      if (this.context.kind === 'main' && run.integration) assertIntegrationReservations(this, run.task.scope, run.integration.resources);
      this.source(run);
      if (existsSync(this.activeFile)) throw new Error('DELIVERY_WRITER_ALREADY_CLAIMED');
      run.status = run.integration ? 'active' : 'awaiting-native';
      this.invalidatePending(run);
      run.events.push({ at: new Date().toISOString(), kind: 'resume', reason: 'new native handoff required' });
      this.save(run); this.claim(id); return run;
    });
  }
  async supersede(id: string, owner: string, successorId: string, reason: string): Promise<DeliveryRun> {
    if (this.context.kind !== 'main') throw new Error('DELIVERY_CANDIDATE_CANNOT_COMMIT');
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.context.assert();
      if (run.task.owner !== owner) throw new Error('DELIVERY_WRITER_MISMATCH');
      if (run.status !== 'paused') throw new Error('DELIVERY_PAUSED_RUN_REQUIRED');
      if (existsSync(this.activeFile)) throw new Error('DELIVERY_WRITER_ALREADY_CLAIMED');
      const successor = this.read(identifier(successorId));
      if (successor.status !== 'complete' || successor.verdict?.pass !== true
        || !successor.commit || !/^[a-f0-9]{40,64}$/.test(successor.commit)) {
        throw new Error('DELIVERY_COMPLETE_SUCCESSOR_REQUIRED');
      }
      try {
        if (git(this.root, 'rev-parse', '--verify', `${successor.commit}^{commit}`) !== successor.commit) {
          throw new Error('commit identity mismatch');
        }
      } catch (error) {
        throw new Error('DELIVERY_SUCCESSOR_COMMIT_REQUIRED', { cause: error });
      }
      try {
        if (run.baseCommit === successor.commit) throw new Error('successor must advance past base');
        git(this.root, 'merge-base', '--is-ancestor', run.baseCommit, successor.commit);
      } catch (error) {
        throw new Error('DELIVERY_SUCCESSOR_DESCENDANT_REQUIRED', { cause: error });
      }
      const at = new Date().toISOString(), resolutionReason = nonempty(reason, 'resolution reason');
      run.resolution = { successorTask: successor.task.id, successorCommit: successor.commit,
        reason: resolutionReason, at };
      run.status = 'superseded';
      run.events.push({ at, kind: 'supersede',
        reason: `${successor.task.id}@${successor.commit}: ${resolutionReason}` });
      return this.save(run);
    });
  }
  async reconcile(id: string, owner: string, nonce: string, reason: string): Promise<DeliveryRun> {
    const current = this.read(id);
    if (current.task.owner !== owner) throw new Error('DELIVERY_WRITER_MISMATCH');
    nonempty(reason, 'recovery reason');
    recoverOperation(this.directory, nonce);
    return withOperationLock(this.directory, async () => {
      const run = this.read(id); this.context.assert();
      const active = existsSync(this.activeFile) ? (readJson(this.activeFile) as { id: string }).id : undefined;
      if (active && active !== id) throw new Error('DELIVERY_WRITER_ALREADY_CLAIMED');
      if (run.status === 'paused' || run.status === 'complete' || run.status === 'superseded') {
        if (active === id) unlinkSync(this.activeFile);
      } else {
        if (this.context.kind === 'main' && !run.integration && activeReservations(this).length) throw new Error('DELIVERY_ACTIVE_CANDIDATES_REQUIRE_INTEGRATION');
        if (this.context.kind === 'main' && run.integration) assertIntegrationReservations(this, run.task.scope, run.integration.resources);
        this.source(run);
        if (!active) this.claim(id);
        run.status = run.integration ? 'active' : 'awaiting-native';
        this.invalidatePending(run);
      }
      run.events.push({ at: new Date().toISOString(), kind: 'reconcile', reason: `${nonce}: ${reason}` });
      return this.save(run);
    });
  }
  private invalidatePending(run: DeliveryRun): void {
    for (const request of run.workflow?.requests ?? []) if (!run.workflow!.results.some(r => r.request.id === request.id)
      && !run.workflow!.invalidated.includes(request.id)) run.workflow!.invalidated.push(request.id);
  }
}

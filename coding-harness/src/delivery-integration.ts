// SPDX-License-Identifier: MIT
import { chmodSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import { createHash, randomUUID } from 'node:crypto';
import { basename, dirname, join } from 'node:path';
import { hash, ReceiptLog, type RunResult } from '@metaharness/harness';
import { asRecord, assertExactKeys, normalizeWorkspacePath } from './contracts.js';
import { identifier, nonempty } from './delivery-contracts.js';
import { candidateContext } from './delivery-candidate.js';
import { DeliveryHarness, type DeliveryRun } from './delivery-runtime.js';
import { atomicJson, git, isSnapshotSourcePath, outsideDigest, readJson, type SourceSnapshot } from './delivery-workspace.js';
import { buildCheckEnvironment, checkEnvironmentEvidence, logDigest } from './delivery-process.js';
import { workflowReady } from './delivery-workflow.js';
import { parseStageResponse } from './delivery-workflow-contracts.js';
import { resolveMutablePath, resolveWorkspacePath } from './workspace.js';
import { requiredDeliveryInputs } from './delivery-lineage.js';
import { assertIntegrationReservations } from './delivery-cohort-custody.js';
import { integrationReviewReady, integrationReviewPaths, integrationWorkflowIntact } from './delivery-integration-review.js';
import type { NativeStageResult } from './delivery-workflow-contracts.js';

interface OwnerRevalidation { commit: string; sourceDigest: string }
export interface IntegrationInput { candidateRoot: string; id: string; owner: string; expectedDigest: string; revalidateAgainst?: OwnerRevalidation; recover?: true }
export interface CandidateIntegration {
  candidateRoot: string; candidateDigest: string; original: DeliveryRun;
  sourceBefore: SourceSnapshot; sourceAfter: SourceSnapshot; phase: 'applying' | 'prepared';
  candidateSource: SourceSnapshot; readPaths: string[]; reviewReadPaths?: string[]; custodyDigest: string; resources: string[];
  checkLogs: Record<string, string>; outcomeReceipts: unknown[]; outcomeHistory: unknown[];
  ownerRevalidation?: OwnerRevalidation;
}
interface Custody { root: string; sourceBefore: SourceSnapshot; baseCommit: string; scope: string[];
  readPaths?: string[]; declaredReadPaths?: string[] | null; cleanBase?: boolean; resources?: string[]; acceptedSource?: unknown }

export function parseIntegrationInput(input: unknown): IntegrationInput {
  const value = asRecord(input, 'integration');
  assertExactKeys({ revalidateAgainst: undefined, recover: undefined, ...value }, ['candidateRoot', 'id', 'owner', 'expectedDigest', 'revalidateAgainst', 'recover'], 'integration');
  if (value.recover !== undefined && value.recover !== true) throw new Error('DELIVERY_INTEGRATION_RECOVERY_FLAG');
  if (typeof value.expectedDigest !== 'string' || !/^[a-f0-9]{64}$/.test(value.expectedDigest)) throw new Error('DELIVERY_INTEGRATION_DIGEST_REQUIRED');
  let revalidateAgainst: OwnerRevalidation | undefined;
  if (value.revalidateAgainst !== undefined) {
    const pin = asRecord(value.revalidateAgainst, 'integration revalidation');
    assertExactKeys(pin, ['commit', 'sourceDigest'], 'integration revalidation');
    if (typeof pin.commit !== 'string' || !/^[a-f0-9]{40}$/.test(pin.commit)
      || typeof pin.sourceDigest !== 'string' || !/^[a-f0-9]{64}$/.test(pin.sourceDigest)) throw new Error('DELIVERY_INTEGRATION_REVALIDATION_PIN_REQUIRED');
    revalidateAgainst = { commit: pin.commit, sourceDigest: pin.sourceDigest };
  }
  return { candidateRoot: realpathSync(nonempty(value.candidateRoot, 'candidate root')), id: identifier(value.id),
    owner: nonempty(value.owner, 'integration owner'), expectedDigest: value.expectedDigest,
    ...(revalidateAgainst ? { revalidateAgainst } : {}), ...(value.recover ? { recover: true } : {}) };
}
// Explicit read closure may narrow data dependencies, never evaluator/runtime/package inputs.
function requiredInputs(files: Record<string, string>, run: DeliveryRun): string[] {
  return requiredDeliveryInputs(files, run.task.checks);
}
export async function validateCandidateRun(run: DeliveryRun, source: SourceSnapshot, logs?: Record<string, string>, historicalEnvironment = false): Promise<void> {
  const { digest, ...body } = run;
  if (run.integration || hash(body) !== digest || !run.verdict?.pass || run.status !== 'active'
    || !workflowReady(run, source.digest)) throw new Error('DELIVERY_CANDIDATE_NOT_REVIEWED');
  validateStageResults(run.workflow!.results);
  const review = run.workflow!.results.at(-1)!;
  if (run.handoffs.some(author => author.executorId === review.response.native.executorId)) throw new Error('DELIVERY_INDEPENDENT_REVIEW_REQUIRED');
  for (const declared of run.task.checks) {
    const check = run.checks.filter(c => c.id === declared.id).at(-1);
    const start = run.events.filter(e => e.kind === 'check-start' && e.reason.startsWith(`${declared.id}:`)).at(-1);
    if (!check?.passed || check.sourceBefore !== source.digest || check.sourceAfter !== source.digest
      || hash(check.argv) !== hash(declared.argv) || check.cwd !== declared.cwd || check.exitCode !== 0 || check.signal !== null
      || start?.reason !== `${check.id}:${check.attempt}`
      || typeof check.environmentDigest !== 'string' || !/^[a-f0-9]{64}$/.test(check.environmentDigest)
      || (!historicalEnvironment && check.environmentDigest !== hash(checkEnvironmentEvidence(buildCheckEnvironment())))
      || await checkedLog(check.stdout, logs) !== check.stdoutDigest || await checkedLog(check.stderr, logs) !== check.stderrDigest) {
      throw new Error('DELIVERY_CANDIDATE_CHECK_INVALID');
    }
  }
}
function validateStageResults(results: NativeStageResult[]): void {
  for (const stage of results) {
    const { id, ...request } = stage.request;
    const response = parseStageResponse(stage.response);
    if (hash(request) !== id || response.requestId !== id || !stage.kernel.receipts.length
      || !ReceiptLog.fromJSON({ receipts: stage.kernel.receipts }).verify().ok
      || (stage.accepted && hash(stage.kernel.result) !== hash({ request: stage.request, response }))
      || stage.kernel.receipts.some(receipt => receipt.outputHash !== hash({ request: stage.request, response })
        || receipt.agent !== response.native.executorId || receipt.runId !== id)
      || (stage.accepted && (!stage.kernel.success || response.outcome !== 'completed' || stage.reasons.length))) {
      throw new Error('DELIVERY_CANDIDATE_RECEIPT_INVALID');
    }
  }
}
async function checkedLog(path: string, logs?: Record<string, string>): Promise<string> {
  if (!logs) return logDigest(path);
  if (!(path in logs)) throw new Error('DELIVERY_INTEGRATION_LOG_MISSING');
  return createHash('sha256').update(Buffer.from(logs[path], 'base64')).digest('hex');
}
export async function validateIntegrationEvidence(run: DeliveryRun): Promise<void> {
  if (!run.integration) return;
  // Archived candidate evidence proves its own execution, never current canonical check fitness.
  await validateCandidateRun(run.integration.original, run.integration.candidateSource, run.integration.checkLogs, true);
  validateStageResults(run.workflow?.results.slice(run.integration.original.workflow!.results.length) ?? []);
  for (const receipt of run.integration.outcomeReceipts) validateOutcomeReceipt(receipt, run.integration.original, run.integration.candidateSource);
  for (const receipt of run.integration.outcomeHistory) validateOutcomeReceipt(receipt, run.integration.original);
}
function validateOutcomeReceipt(input: unknown, original: DeliveryRun, source?: SourceSnapshot): void {
  const { digest, ...body } = asRecord(input, 'candidate outcome');
  const kernel = body.kernel as RunResult;
  if (digest !== hash(body) || body.taskDigest !== hash(original.task)
    || (source && (body.success !== true || body.sourceAfter !== source.digest || !kernel.receipts.length))
    || !ReceiptLog.fromJSON({ receipts: kernel.receipts }).verify().ok) throw new Error('DELIVERY_CANDIDATE_RECEIPT_INVALID');
}
function mutableIntegrationPath(root: string, path: string): string {
  normalizeWorkspacePath(path, 'integration path');
  const parts = path.split('/');
  for (let i = 1; i < parts.length; i++) {
    const parent = parts.slice(0, i).join('/');
    try { lstatSync(join(root, parent)); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return join(root, path); throw error; }
    resolveWorkspacePath(root, parent, { requireDirectory: true });
  }
  return resolveMutablePath(root, path);
}
export async function prepareIntegration(harness: DeliveryHarness, input: IntegrationInput, previous?: DeliveryRun): Promise<DeliveryRun> {
  if (harness.context.kind !== 'main') throw new Error('DELIVERY_CANONICAL_SOURCE_REQUIRED');
  if (!previous && git(harness.root, 'status', '--porcelain') !== '') throw new Error('DELIVERY_INTEGRATION_DIRTY_CANONICAL');
  const custody = readJson(join(harness.directory, `candidate-${hash(input.candidateRoot)}.json`)) as Custody;
  if (custody.root !== input.candidateRoot || hash(custody.sourceBefore.files) !== custody.sourceBefore.digest || !custody.cleanBase) {
    throw new Error('DELIVERY_CANDIDATE_CUSTODY_INVALID');
  }
  const priorCustody = previous ? readJson(join(harness.directory, `candidate-${hash(previous.integration!.candidateRoot)}.json`)) as Custody : undefined;
  if (priorCustody) {
    const contract = (value: Custody) => hash([value.readPaths?.slice().sort() ?? null,
      value.declaredReadPaths?.slice().sort() ?? null, value.resources?.slice().sort() ?? [], value.acceptedSource ?? null]);
    if (hash(priorCustody) !== previous!.integration!.custodyDigest || contract(custody) !== contract(priorCustody)) {
      throw new Error('DELIVERY_INTEGRATION_RECOVERY_CUSTODY_CHANGED');
    }
  }
  const context = candidateContext(input.candidateRoot, harness, custody.sourceBefore, custody.baseCommit, custody.scope, true);
  const candidate = new DeliveryHarness(input.candidateRoot, context), original = candidate.read(input.id), candidateSource = candidate.snapshot();
  assertIntegrationReservations(harness, original.task.scope, custody.resources);
  if (original.task.owner !== input.owner) throw new Error('DELIVERY_INTEGRATION_OWNER_MISMATCH');
  if (original.digest !== input.expectedDigest || hash(original.task.scope) !== hash(custody.scope)
    || original.baseCommit !== custody.baseCommit || hash(original.task.readPaths ?? null) !== hash(custody.declaredReadPaths ?? null)) {
    throw new Error('DELIVERY_CANDIDATE_IDENTITY');
  }
  if (existsSync(join(candidate.directory, 'operation.lock')) || existsSync(join(candidate.directory, 'runner/operation.lock'))) throw new Error('DELIVERY_CANDIDATE_BUSY');
  if (previous && hash(original.task) !== hash(previous.task)) throw new Error('DELIVERY_INTEGRATION_RECOVERY_TASK_CHANGED');
  const before = harness.snapshot(), readPaths = [...new Set([...(custody.readPaths ?? Object.keys(custody.sourceBefore.files)),
    ...requiredInputs({ ...custody.sourceBefore.files, ...before.files }, original)])].filter(path => !original.task.scope.includes(path));
  const revalidation = input.revalidateAgainst;
  if (revalidation && (revalidation.commit !== harness.context.head() || revalidation.sourceDigest !== before.digest)) {
    throw new Error('DELIVERY_INTEGRATION_REVALIDATION_STALE');
  }
  await validateCandidateRun(original, candidateSource, undefined, !!revalidation);
  // Explicit owner revalidation retains every read; it never permits overwriting changed scope.
  for (const path of revalidation ? [] : readPaths) if (before.files[path] !== custody.sourceBefore.files[path]) throw new Error('DELIVERY_INTEGRATION_INPUT_CHANGED');
  for (const path of original.task.scope) if ((priorCustody ? priorCustody.sourceBefore.files[path] : before.files[path]) !== custody.sourceBefore.files[path]) throw new Error('DELIVERY_INTEGRATION_INPUT_CHANGED');
  if (!revalidation && custody.declaredReadPaths == null
    && outsideDigest(before, original.task.scope) !== outsideDigest(custody.sourceBefore, original.task.scope)) {
    throw new Error('DELIVERY_INTEGRATION_INPUT_CHANGED');
  }
  git(harness.root, 'merge-base', '--is-ancestor', custody.baseCommit, harness.context.head());
  const commits = git(harness.root, 'rev-list', `${custody.baseCommit}..HEAD`).split('\n').filter(Boolean);
  // Only exact, completed sibling integrations can account for intervening changes.
  for (const commit of revalidation ? [] : commits) {
    const paths = git(harness.root, 'diff-tree', '--no-commit-id', '--name-only', '-r', commit).split('\n').filter(Boolean);
    const sibling = findIntegratedRun(harness, commit);
    if (!sibling || !paths.length || paths.some(path => !sibling.task.scope.includes(path) || readPaths.includes(path) || original.task.scope.includes(path))) {
      throw new Error('DELIVERY_INTEGRATION_UNACCEPTED_DRIFT');
    }
  }
  const afterFiles = { ...before.files };
  for (const path of original.task.scope) {
    if (candidateSource.files[path]) afterFiles[path] = candidateSource.files[path]; else delete afterFiles[path];
    mutableIntegrationPath(harness.root, path);
  }
  const after = { files: afterFiles, digest: hash(afterFiles) };
  if (after.digest === before.digest) throw new Error('DELIVERY_INTEGRATION_NO_CHANGE');
  const checkLogs = Object.fromEntries(original.checks.flatMap(check => [check.stdout, check.stderr]).map(path => [path, readFileSync(path).toString('base64')]));
  const runnerDirectory = join(candidate.directory, 'runner');
  const outcomeHistory = existsSync(runnerDirectory) ? readdirSync(runnerDirectory).filter(name => /^outcome-[a-f0-9-]{36}\.json$/.test(name))
    .map(name => readJson(join(runnerDirectory, name))).filter(value => (value as { taskDigest?: string }).taskDigest === hash(original.task)) : [];
  const outcomeReceipts = outcomeHistory.filter(value => (value as { success?: boolean; sourceAfter?: string }).success === true
    && (value as { sourceAfter?: string }).sourceAfter === candidateSource.digest);
  for (const receipt of outcomeHistory) validateOutcomeReceipt(receipt, original);
  for (const receipt of outcomeReceipts) validateOutcomeReceipt(receipt, original, candidateSource);
  return { ...structuredClone(original), baseCommit: harness.context.head(), outsideDigest: outsideDigest(before, original.task.scope),
    checks: [], events: [{ at: new Date().toISOString(), kind: 'candidate-integration', reason: input.expectedDigest }],
    verdict: undefined, digest: undefined, integration: { candidateRoot: input.candidateRoot, candidateDigest: input.expectedDigest,
      original, sourceBefore: before, sourceAfter: after, candidateSource, readPaths, reviewReadPaths: custody.readPaths,
      custodyDigest: hash(custody), resources: custody.resources ?? [], checkLogs, outcomeReceipts, outcomeHistory, phase: 'applying',
      ...(revalidation ? { ownerRevalidation: revalidation } : {}) } };
}

export function findIntegratedRun(harness: DeliveryHarness, commit: string): DeliveryRun | undefined {
  for (const file of readdirSync(harness.directory)) {
    if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]{0,100}\.json$/.test(file)) continue;
    const value = readJson(join(harness.directory, file)) as Partial<DeliveryRun>;
    if (value.commit !== commit || !value.integration || value.status !== 'complete' || !value.verdict?.pass) continue;
    const run = harness.read(value.task!.id);
    if (integrationReady(run, run.integration!.sourceAfter.digest)) return run;
  }
  return undefined;
}
export function integrationReady(run: DeliveryRun, source: string): boolean {
  return integrationSourceReady(run, source) && (!(run.integration?.ownerRevalidation || run.events.some(event => event.kind === 'integration-recovery') || integrationReviewPaths(run).length) || integrationReviewReady(run, source));
}

/** Explicit same-task recovery preserves the rejected run and truthfully archives entry bytes. */
export async function recoverIntegration(harness: DeliveryHarness, input: IntegrationInput, previous?: DeliveryRun): Promise<DeliveryRun> {
  if (!previous?.integration || previous.status !== 'paused' || previous.integration.phase !== 'prepared' || previous.commit
    || previous.task.owner !== input.owner) throw new Error('DELIVERY_INTEGRATION_RECOVERY_REQUIRED');
  const before = harness.snapshot(), evidence = previous.integration;
  const revalidation = input.revalidateAgainst;
  if (revalidation && (revalidation.commit !== harness.context.head() || revalidation.sourceDigest !== before.digest)) {
    throw new Error('DELIVERY_INTEGRATION_REVALIDATION_STALE');
  }
  const rejectedScope = previous.task.scope.every(path => before.files[path] === evidence.sourceAfter.files[path]);
  const originalScope = previous.task.scope.every(path => before.files[path] === evidence.sourceBefore.files[path]);
  if ((!revalidation && (harness.context.head() !== previous.baseCommit || before.digest !== evidence.sourceAfter.digest))
    || (!rejectedScope && !(revalidation && originalScope))
    || git(harness.root, 'diff', '--cached', '--name-only') !== '') throw new Error('DELIVERY_INTEGRATION_RECOVERY_SOURCE_CHANGED');
  if (revalidation) {
    git(harness.root, 'merge-base', '--is-ancestor', previous.baseCommit, harness.context.head());
    const dirtyPaths = [...git(harness.root, 'diff', 'HEAD', '--name-only', '-z').split('\0'),
      ...git(harness.root, 'ls-files', '--others', '--exclude-standard', '-z').split('\0')].filter(Boolean);
    if (dirtyPaths.some(path => isSnapshotSourcePath(path) && !previous.task.scope.includes(path))) {
      throw new Error('DELIVERY_INTEGRATION_RECOVERY_SOURCE_CHANGED');
    }
  }
  const reviews = previous.workflow?.results.slice(evidence.original.workflow!.results.length) ?? [];
  if (!previous.checks.some(check => !check.passed && check.sourceBefore === evidence.sourceAfter.digest)
    && !reviews.some(result => !result.accepted && result.response.sourceDigest === evidence.sourceAfter.digest)) {
    throw new Error('DELIVERY_INTEGRATION_RECOVERY_REJECTION_REQUIRED');
  }
  await validateIntegrationEvidence(previous);
  const run = await prepareIntegration(harness, input, previous);
  if (run.integration!.sourceAfter.digest === before.digest) throw new Error('DELIVERY_INTEGRATION_RECOVERY_NO_CHANGE');
  const archive = join(harness.directory, `integration-recovery-${previous.digest}`);
  mkdirSync(archive, { recursive: true, mode: 0o700 });
  resolveWorkspacePath(harness.directory, basename(archive), { requireDirectory: true });
  for (const path of previous.task.scope) {
    if (!before.files[path]) continue;
    const target = mutableIntegrationPath(archive, `source/${path}`), bytes = readFileSync(resolveWorkspacePath(harness.root, path, { requireRegularFile: true }));
    mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
    if (!existsSync(target)) writeFileSync(target, bytes, { flag: 'wx', mode: 0o400 });
    else if (!readFileSync(target).equals(bytes)) throw new Error('DELIVERY_INTEGRATION_RECOVERY_ARCHIVE_CHANGED');
  }
  const record = join(archive, 'run.json');
  if (!existsSync(record)) atomicJson(record, previous);
  else if (hash(readJson(record)) !== hash(previous)) throw new Error('DELIVERY_INTEGRATION_RECOVERY_ARCHIVE_CHANGED');
  // source/ contains actual recovery-entry bytes, not reconstructed rejected bytes.
  // The exact negative run retains its original candidate and rejected source binding.
  const entry = { source: before, scopeState: rejectedScope ? 'rejected' : 'original', rejectedSourceDigest: evidence.sourceAfter.digest };
  const entryRecord = join(archive, 'entry-source.json');
  if (!existsSync(entryRecord)) atomicJson(entryRecord, entry);
  else if (hash(readJson(entryRecord)) !== hash(entry)) throw new Error('DELIVERY_INTEGRATION_RECOVERY_ARCHIVE_CHANGED');
  run.checks = structuredClone(previous.checks);
  run.events = [...structuredClone(previous.events), ...run.events,
    { at: new Date().toISOString(), kind: 'integration-recovery', reason: archive }];
  return run;
}
export function integrationSourceReady(run: DeliveryRun, source: string): boolean {
  const integration = run.integration;
  if (!integration || integration.phase !== 'prepared' || source !== integration.sourceAfter.digest) return false;
  if (integration.ownerRevalidation && (integration.ownerRevalidation.commit !== run.baseCommit
    || integration.ownerRevalidation.sourceDigest !== integration.sourceBefore.digest
    || hash(integration.sourceBefore.files) !== integration.sourceBefore.digest)) return false;
  const { digest, ...original } = integration.original;
  return digest === integration.candidateDigest && hash(original) === digest && !original.integration
    && hash(run.task) === hash(original.task) && integrationWorkflowIntact(run)
    && hash(run.handoffs) === hash(original.handoffs) && hash(integration.sourceAfter.files) === source
    && workflowReady(integration.original, integration.candidateSource.digest);
}
export function applyIntegration(harness: DeliveryHarness, run: DeliveryRun, input: IntegrationInput): void {
  assertIntegrationReservations(harness, run.task.scope,
    (readJson(join(harness.directory, `candidate-${hash(input.candidateRoot)}.json`)) as Custody).resources);
  const evidence = run.integration;
  if (!evidence || input.expectedDigest !== evidence.candidateDigest || input.candidateRoot !== evidence.candidateRoot
    || run.task.owner !== input.owner || hash(input.revalidateAgainst ?? null) !== hash(evidence.ownerRevalidation ?? null)) throw new Error('DELIVERY_INTEGRATION_OWNER_OR_IDENTITY');
  if (harness.context.head() !== run.baseCommit) throw new Error('DELIVERY_BASE_MOVED');
  if (git(harness.root, 'diff', '--cached', '--name-only') !== '') throw new Error('DELIVERY_INTEGRATION_DIRTY_INDEX');
  if (hash(readJson(join(harness.directory, `candidate-${hash(input.candidateRoot)}.json`))) !== evidence.custodyDigest
    || (readJson(join(evidence.candidateRoot, '.metaharness/delivery', `${run.task.id}.json`)) as DeliveryRun).digest !== evidence.candidateDigest) {
    throw new Error('DELIVERY_CANDIDATE_IDENTITY');
  }
  const current = harness.snapshot();
  if (outsideDigest(current, run.task.scope) !== run.outsideDigest) throw new Error('DELIVERY_OUT_OF_SCOPE_CHANGE');
  for (const path of run.task.scope) {
    if (current.files[path] !== evidence.sourceBefore.files[path] && current.files[path] !== evidence.sourceAfter.files[path]) throw new Error('DELIVERY_INTEGRATION_INPUT_CHANGED');
  }
  const context = candidateContext(evidence.candidateRoot, harness, evidence.sourceBefore, run.baseCommit, run.task.scope, true);
  // The current canonical baseline may include accepted siblings; compare original candidate directly.
  if (context.snapshot().digest !== evidence.candidateSource.digest) throw new Error('DELIVERY_CANDIDATE_SOURCE_CHANGED');
  for (const path of run.task.scope) {
    const target = mutableIntegrationPath(harness.root, path), entry = evidence.sourceAfter.files[path];
    if (!entry) { if (existsSync(target)) unlinkSync(target); }
    else {
      const temporary = join(harness.directory, `integration-${randomUUID()}.tmp`);
      writeFileSync(temporary, readFileSync(join(evidence.candidateRoot, path)), { flag: 'wx', mode: 0o600 });
      chmodSync(temporary, entry.startsWith('100755:') ? 0o755 : 0o644);
      mkdirSync(dirname(target), { recursive: true }); renameSync(temporary, target);
    }
  }
  if (harness.snapshot().digest !== evidence.sourceAfter.digest) throw new Error('DELIVERY_INTEGRATION_SOURCE_MISMATCH');
  evidence.phase = 'prepared';
}

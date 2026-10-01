// SPDX-License-Identifier: MIT
import { mkdirSync, readFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { hash } from '@metaharness/harness';
import { selectDeliveryRoute } from './delivery-contracts.js';
import { checkDigests, stageEvidenceDigest } from './delivery-workflow.js';
import { integrationSourceReady } from './delivery-integration.js';
import { createDeliveryExecutor, type DeliveryExecutor } from './delivery-executor.js';
import { atomicJson, withOperationLock } from './delivery-workspace.js';
import { resolveWorkspacePath } from './workspace.js';
import { buildCheckEnvironment } from './delivery-process.js';
import { requiredDeliveryInputs } from './delivery-lineage.js';
import type { DeliveryHarness, DeliveryRun } from './delivery-runtime.js';
import type { DeliveryAction, NativeStageRequest } from './delivery-workflow-contracts.js';

/** All changed implicit Cargo inputs remain review inputs; no guessed crate dependency graph. */
export function integrationReviewPaths(run: DeliveryRun): string[] {
  const evidence = run.integration;
  if (!evidence || (!evidence.ownerRevalidation && !run.task.checks.some(check => check.argv[0] === 'cargo'))) return [];
  return [...new Set([...Object.keys(evidence.candidateSource.files), ...Object.keys(evidence.sourceAfter.files)])]
    .filter(path => !run.task.scope.includes(path) && evidence.candidateSource.files[path] !== evidence.sourceAfter.files[path]);
}

/** Cargo identifies package boundaries; unrelated new packages need no duplicate source review. */
export async function integrationReviewInputPaths(root: string, run: DeliveryRun, signal?: AbortSignal): Promise<string[]> {
  const declared = [...run.task.scope, ...(run.integration!.reviewReadPaths ?? run.task.readPaths ?? [])];
  const changed = integrationReviewPaths(run);
  const all = () => [...new Set([...declared, ...changed])];
  if (!changed.length || run.task.checks.some(check => check.argv[0] !== 'cargo')) return all();
  try {
    const roots = new Set<string>(), commands = new Map<string, { cwd: string; manifest?: string }>();
    for (const check of run.task.checks) {
      if (check.argv.some(arg => arg === '--config' || arg.startsWith('--config='))) return all();
      const index = check.argv.indexOf('--manifest-path');
      const manifest = index >= 0 ? check.argv[index + 1] : check.argv.find(arg => arg.startsWith('--manifest-path='))?.slice(16);
      const cwd = resolve(root, check.cwd);
      commands.set(JSON.stringify([cwd, manifest]), { cwd, manifest });
    }
    for (const { cwd, manifest } of commands.values()) {
      const { stdout } = await promisify(execFile)('cargo', ['metadata', '--format-version', '1', '--no-deps', '--offline', '--locked',
        ...(manifest ? ['--manifest-path', manifest] : [])], { cwd, env: buildCheckEnvironment(), signal, maxBuffer: 10_000_000 });
      const metadata = JSON.parse(stdout) as { packages: { manifest_path: string; dependencies: { path?: string }[];
        targets: { src_path: string }[] }[] };
      if (!metadata.packages.length) return all();
      const packages = new Set(metadata.packages.map(pkg => dirname(pkg.manifest_path)));
      // --no-deps cannot describe transitive external path packages; retain full context in that case.
      if (metadata.packages.some(pkg => pkg.dependencies.some(dep => dep.path && !packages.has(dep.path)))) return all();
      if (metadata.packages.some(pkg => pkg.targets.some(target => {
        const local = relative(dirname(pkg.manifest_path), target.src_path);
        return local === '..' || local.startsWith('../') || isAbsolute(local);
      }))) return all();
      for (const path of packages) {
        const local = relative(root, path);
        if (!local || local === '..' || local.startsWith('../') || isAbsolute(local)) return all();
        roots.add(local);
      }
    }
    const hardInputs = new Set(requiredDeliveryInputs({ ...run.integration!.candidateSource.files, ...run.integration!.sourceAfter.files }, run.task.checks));
    // Existing implicit inputs retain their review, including deletions and nonstandard Rust includes.
    return [...new Set([...declared, ...changed.filter(path => run.integration!.candidateSource.files[path] || hardInputs.has(path)
      || [...roots].some(prefix => path.startsWith(`${prefix}/`)))])];
  } catch (error) {
    if (signal?.aborted) throw error;
    // Metadata is an optimisation, never a new acceptance or availability gate.
    return all();
  }
}
export function integrationReviewPrerequisites(run: DeliveryRun): string[] {
  return [hash(run.integration!.original), ...checkDigests(run)];
}
export function integrationWorkflowIntact(run: DeliveryRun): boolean {
  const original = run.integration!.original.workflow!, current = run.workflow;
  if (!current) return false;
  if (!run.integration!.ownerRevalidation && !run.events.some(event => event.kind === 'integration-recovery') && !integrationReviewPaths(run).length) return hash(current) === hash(original);
  return hash(current.requests.slice(0, original.requests.length)) === hash(original.requests)
    && hash(current.results.slice(0, original.results.length)) === hash(original.results)
    && hash(current.invalidated.slice(0, original.invalidated.length)) === hash(original.invalidated)
    && current.requests.slice(original.requests.length).every(request => request.stage === 'review')
    && current.results.slice(original.results.length).every(result => result.request.stage === 'review');
}
export function integrationReviewReady(run: DeliveryRun, source: string): boolean {
  const workflow = run.workflow!, original = run.integration!.original.workflow!;
  const review = workflow.results.slice(original.results.length).at(-1);
  return !!review?.accepted && review.request.stage === 'review'
    && review.request.sourceDigest === source && review.response.sourceDigest === source
    && !run.handoffs.some(h => h.executorId === review.response.native.executorId)
    && !original.results.some(result => result.response.native.executorId === review.response.native.executorId)
    && !workflow.invalidated.includes(review.request.id)
    && !workflow.requests.some(request => !workflow.invalidated.includes(request.id)
      && !workflow.results.some(result => result.request.id === request.id))
    && hash(review.request.prerequisiteDigests) === hash(integrationReviewPrerequisites(run));
}

/** Revalidate preserved bytes, never replan or reauthor because an independent sibling finished. */
export function nextIntegrationAction(run: DeliveryRun, source: string, validChecks: ReadonlySet<string>): DeliveryAction {
  if (!integrationSourceReady(run, source)) throw new Error('DELIVERY_INTEGRATION_SOURCE_MISMATCH');
  const missing = run.task.checks.find(check => !validChecks.has(check.id));
  if (missing) {
    const latest = run.checks.filter(check => check.id === missing.id).at(-1);
    if (latest && !latest.passed && latest.sourceBefore === source) return { kind: 'paused', reason: 'DELIVERY_INTEGRATION_CHECK_FAILED' };
    return { kind: 'check', checkId: missing.id };
  }
  const changed = integrationReviewPaths(run);
  if ((!run.integration!.ownerRevalidation && !run.events.some(event => event.kind === 'integration-recovery') && !changed.length) || integrationReviewReady(run, source)) return { kind: 'ready-to-commit', sourceDigest: source };
  const workflow = run.workflow!;
  const pending = workflow.requests.find(request => !workflow.invalidated.includes(request.id)
    && !workflow.results.some(result => result.request.id === request.id));
  if (pending) {
    if (pending.sourceDigest === source && pending.evidenceDigest === stageEvidenceDigest(run)
      && hash(pending.prerequisiteDigests) === hash(integrationReviewPrerequisites(run))) return { kind: 'native', request: pending };
    workflow.invalidated.push(pending.id);
  }
  const latest = workflow.results.slice(run.integration!.original.workflow!.results.length).at(-1);
  if (latest && !latest.accepted) return { kind: 'paused', reason: 'DELIVERY_INTEGRATION_REVIEW_FAILED' };
  const body: Omit<NativeStageRequest, 'id'> = { schemaVersion: 1, taskId: run.task.id, thread: run.task.thread,
    baseCommit: run.baseCommit, stage: 'review', attempt: workflow.requests.length + 1, sourceDigest: source,
    evidenceDigest: stageEvidenceDigest(run), repair: false, prerequisiteDigests: integrationReviewPrerequisites(run),
    route: run.task.reviewer ?? selectDeliveryRoute({ ...run.task, taskClass: 'implementation', requested: undefined, preserveMainModel: false }),
    requirement: run.task.requirement, scope: run.task.scope,
    feedback: ['Review preserved candidate patch against current accepted source and fresh deterministic checks. No authoring or planning.',
      `${run.integration!.ownerRevalidation ? 'Owner-pinned canonical revalidation; previous candidate acceptance does not qualify changed evaluators or dependencies' : 'Accepted sibling inputs changed'}: ${changed.join(', ')}`] };
  const request = { ...body, id: hash(body) }; workflow.requests.push(request);
  return { kind: 'native', request };
}

export async function runIntegrationReview(harness: DeliveryHarness, id: string, owner: string,
  options: { execute?: DeliveryExecutor; signal?: AbortSignal } = {}) {
  const initial = harness.read(id);
  if (harness.context.kind !== 'main' || !initial.integration) throw new Error('DELIVERY_INTEGRATION_REQUIRED');
  const directory = join(harness.directory, `integration-review-${id}`);
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  return withOperationLock(directory, async () => {
    const action = await harness.advance(id, owner, options.signal);
    if (action.kind === 'paused') {
      if (harness.read(id).status !== 'paused') await harness.pause(id, owner, action.reason);
      return { success: false, failure: action.reason };
    }
    if (action.kind === 'native') {
      if (action.request.stage !== 'review') throw new Error('DELIVERY_INTEGRATION_REVIEW_ONLY');
      const source = harness.snapshot().digest;
      if (source !== action.request.sourceDigest) throw new Error('DELIVERY_EXECUTION_SOURCE_CHANGED');
      const paths = await integrationReviewInputPaths(harness.root, initial, options.signal);
      const files = paths.map(path => {
        // A snapshot-bound deletion remains review evidence even when its parent directory is gone.
        if (initial.integration!.candidateSource.files[path] && !initial.integration!.sourceAfter.files[path]) return { path, content: null };
        const absolute = resolveWorkspacePath(harness.root, path, { allowMissingLeaf: true, requireRegularFile: true });
        try { return { path, content: readFileSync(absolute, 'utf8') }; }
        catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return { path, content: null }; throw error; }
      });
      if (harness.snapshot().digest !== source) throw new Error('DELIVERY_EXECUTION_SOURCE_CHANGED');
      const execute = options.execute ?? createDeliveryExecutor(harness, hash(initial.task));
      const result = await execute(action.request, files, [
        { preservedCandidate: initial.integration!.candidateDigest, candidateSource: initial.integration!.candidateSource.digest },
        ...(initial.integration!.ownerRevalidation ? [{ ownerRevalidation: initial.integration!.ownerRevalidation }] : []),
        ...harness.read(id).checks,
      ], options.signal);
      atomicJson(join(directory, `${action.request.id}-${randomUUID()}.json`), { request: action.request, ...result });
      if (result.changes.length || harness.snapshot().digest !== source) throw new Error('DELIVERY_REVIEW_MUTATION_REFUSED');
      await harness.submit(id, owner, result.response);
      if (harness.read(id).status === 'paused') return { success: false, failure: result.response.summary };
    }
    const verified = await harness.verify(id, owner);
    if (!verified.verdict?.pass) await harness.pause(id, owner, 'DELIVERY_INTEGRATION_REVIEW_FAILED');
    return { success: verified.verdict?.pass === true, failure: verified.verdict?.pass ? null : 'DELIVERY_INTEGRATION_REVIEW_FAILED' };
  });
}

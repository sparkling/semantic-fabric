// SPDX-License-Identifier: MIT
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { hash, ReceiptLog } from '@metaharness/harness';
import { gateFingerprint, verifyReceipt, verifyReplayBundle, type Policy, type PromotionReceipt, type PromotionRule, type ReplayBundle } from '@metaharness/flywheel';
import { asRecord, assertExactKeys } from './contracts.js';
import type { DeliveryRun } from './delivery-runtime.js';
import { atomicJson, readJson, withOperationLock } from './delivery-workspace.js';
import { mkdirSync } from 'node:fs';
import { parseDeliveryTask } from './delivery-contracts.js';
import { resolveWorkspacePath } from './workspace.js';

export const DELIVERY_ROOT_POLICY: Policy = Object.freeze({
  planner: 'Choose the smallest admitted file-level plan and name the required checks.',
  implementation: 'Preserve passing behavior; repair concrete verifier findings using exact admitted source.',
  reviewer: 'Report concrete correctness, security and regression blockers against current source and deterministic evidence.',
});

/** Subscription cost and usage are deliberately absent from the promotion decision. */
export const deliveryPromotionRule: PromotionRule = evidence => {
  const reasons: string[] = [];
  for (const value of [evidence.baseline.primary, evidence.candidate.primary, evidence.baseline.noopRate, evidence.candidate.noopRate]) {
    if (!Number.isFinite(value) || value < 0 || value > 1) reasons.push('invalid-quality-evidence');
  }
  if (typeof evidence.baseline.regressed !== 'boolean' || typeof evidence.candidate.regressed !== 'boolean') reasons.push('invalid-regression-evidence');
  if (evidence.anchor && [evidence.anchor.baseline, evidence.anchor.candidate].some(value => !Number.isFinite(value) || value < 0 || value > 1)) reasons.push('invalid-anchor-evidence');
  if (evidence.candidate.primary <= evidence.baseline.primary) reasons.push('no-quality-improvement');
  if (evidence.candidate.noopRate > evidence.baseline.noopRate || evidence.candidate.regressed) reasons.push('regression');
  if (evidence.anchor && evidence.anchor.candidate < evidence.anchor.baseline) reasons.push('anchor-regressed');
  return { promote: reasons.length === 0, reasons };
};

export function deliveryPolicyInputDigest(run: DeliveryRun): string {
  const directory = dirname(fileURLToPath(import.meta.url));
  const collect = (root: string, prefix = ''): [string, string][] => readdirSync(root, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).flatMap(entry => {
    const path = join(root, entry.name), name = `${prefix}${entry.name}`;
    return entry.isDirectory() ? collect(path, `${name}/`) : /\.(js|ts)$/.test(name) && !name.endsWith('.d.ts') ? [[name, hash(readFileSync(path, 'utf8'))]] : [];
  });
  const runtime = collect(directory);
  const packageRoot = dirname(directory);
  runtime.push(['package-lock.json', hash(readFileSync(join(packageRoot, 'package-lock.json'), 'utf8'))]);
  return hash({ runtime, node: process.versions.node, baseCommit: run.baseCommit, outsideDigest: run.outsideDigest, adoptedSource: run.adoptedSource,
    task: { requirement: run.task.requirement, scope: run.task.scope, checks: run.task.checks, route: run.route, reviewer: run.task.reviewer ?? null } });
}

export function captureDeliveryPolicy(value: unknown): Policy {
  const policy = asRecord(value, 'delivery policy'); assertExactKeys(policy, Object.keys(DELIVERY_ROOT_POLICY), 'delivery policy');
  for (const text of Object.values(policy)) if (typeof text !== 'string' || text.length < 8 || text.length > 20000) throw new Error('DELIVERY_POLICY_INVALID');
  return Object.freeze({ ...policy }) as Policy;
}

export function verifyDeliveryPolicyActivation(input: {
  policy: unknown; bundle: ReplayBundle; binding: PromotionReceipt; evidence: unknown; publicKey: string; rootId: string; inputDigest: string;
}): Policy {
  const policy = captureDeliveryPolicy(input.policy), head = input.bundle.chain[0];
  const evidence = asRecord(input.evidence, 'policy evaluation evidence');
  if (evidence.inputDigest !== input.inputDigest || !Array.isArray(evidence.evaluations) || !evidence.evaluations.length) throw new Error('DELIVERY_POLICY_EVIDENCE_INVALID');
  const suites = asRecord(evidence.suites, 'policy suites');
  const tasks = new Map<string, string[]>(), allIds = new Set<string>(), seenRuns = new Set<string>();
  for (const name of ['holdout', 'anchor']) {
    const rows = suites[name];
    if (!Array.isArray(rows) || rows.length < 5) throw new Error('DELIVERY_POLICY_SUITE_TOO_SMALL');
    tasks.set(name, rows.map(row => {
      const task = parseDeliveryTask(asRecord(row, 'policy case').task);
      if (task.host === 'openrouter' || allIds.has(task.id)) throw new Error('DELIVERY_POLICY_SUITE_INVALID');
      allIds.add(task.id); return hash(task);
    }));
  }
  const baseline = new Map<string, boolean>(), scores = new Map<string, unknown>();
  for (const value of evidence.evaluations) {
    const evaluation = asRecord(value, 'policy evaluation');
    const expected = tasks.get(String(evaluation.suite).replace(/^replay-/, ''));
    if (!expected || !Array.isArray(evaluation.receipts) || evaluation.receipts.length !== expected.length
      || hash(captureDeliveryPolicy(evaluation.policy)) !== evaluation.policyDigest) throw new Error('DELIVERY_POLICY_EVIDENCE_INVALID');
    const seen = new Set<string>(), receipts: Record<string, unknown>[] = [];
    for (const raw of evaluation.receipts) {
      const { digest, ...receipt } = asRecord(raw, 'policy evaluation receipt');
      const kernel = asRecord(receipt.kernel, 'policy evaluation kernel');
      const log = ReceiptLog.fromJSON({ receipts: kernel.receipts });
      if (hash(receipt) !== digest || receipt.policyDigest !== evaluation.policyDigest || receipt.policyEvaluation !== true
        || typeof receipt.success !== 'boolean' || kernel.success !== receipt.success || !expected.includes(String(receipt.taskDigest))
        || seen.has(String(receipt.taskDigest)) || seenRuns.has(String(receipt.runId)) || log.isEmpty || !log.verify().ok) throw new Error('DELIVERY_POLICY_EVIDENCE_INVALID');
      seen.add(String(receipt.taskDigest)); seenRuns.add(String(receipt.runId)); receipts.push(receipt);
      if (evaluation.policyDigest === hash(DELIVERY_ROOT_POLICY)) baseline.set(String(receipt.taskDigest), receipt.success);
    }
    const score = { primary: receipts.filter(row => row.success).length / receipts.length,
      noopRate: receipts.filter(row => row.sourceBefore === row.sourceAfter).length / receipts.length, costPerWin: 0,
      regressed: receipts.some(row => baseline.get(String(row.taskDigest)) === true && !row.success) };
    if (hash(score) !== hash(evaluation.score)) throw new Error('DELIVERY_POLICY_SCORE_INVALID');
    scores.set(`${evaluation.policyDigest}:${evaluation.suite}`, score);
  }
  const held = scores.get(`${hash(policy)}:holdout`), anchor = scores.get(`${hash(policy)}:anchor`);
  if (baseline.size !== allIds.size || !held || !anchor || hash(held) !== hash(head?.candidateScore)
    || (anchor as { primary: number }).primary !== head?.anchorScore || (anchor as { regressed: boolean }).regressed
    || hash(held) !== hash(scores.get(`${hash(policy)}:replay-holdout`))
    || hash(anchor) !== hash(scores.get(`${hash(policy)}:replay-anchor`))) throw new Error('DELIVERY_POLICY_REPLAY_INVALID');
  let prior = scores.get(`${hash(DELIVERY_ROOT_POLICY)}:holdout`);
  const rootAnchor = scores.get(`${hash(DELIVERY_ROOT_POLICY)}:anchor`) as { primary: number } | undefined;
  for (const commit of [...input.bundle.chain].reverse()) {
    if (commit.verdict === 'ROOT') {
      if (!rootAnchor || rootAnchor.primary !== commit.anchorScore) throw new Error('DELIVERY_POLICY_BASELINE_INVALID');
    } else {
      if (hash(commit.baselineScore) !== hash(prior)) throw new Error('DELIVERY_POLICY_BASELINE_INVALID');
      prior = commit.candidateScore;
    }
  }
  if (!head || head.verdict !== 'PROMOTED' || head.anchorScore === null || input.bundle.root_id !== input.rootId
    || input.bundle.all_commits.some(commit => commit.receipt.publicKey !== input.publicKey)
    || input.binding.publicKey !== input.publicKey || !verifyReceipt(input.binding)
    || !verifyReplayBundle(input.bundle, { pinnedGateFingerprint: gateFingerprint(deliveryPromotionRule), promotionRule: deliveryPromotionRule }).pass
    || hash(input.binding.payload) !== hash({ kind: 'fabric-evaluated-policy-v1', rootId: input.rootId, headId: head.id,
      policyDigest: hash(policy), bundleDigest: hash(input.bundle), inputDigest: input.inputDigest, evidenceDigest: hash(evidence) })) throw new Error('DELIVERY_POLICY_PROMOTION_INVALID');
  return policy;
}

/** Local compare-and-swap also supports rollback to previously verified, still-current evidence. */
export async function activateDeliveryPolicy(canonicalRoot: string, run: DeliveryRun, active: unknown, expectedDigest: string | null) {
  const directory = join(canonicalRoot, '.metaharness/delivery/policy'); mkdirSync(directory, { recursive: true, mode: 0o700 });
  return withOperationLock(directory, async () => {
    const trust = asRecord(readJson(resolveWorkspacePath(canonicalRoot, 'config/delivery-policy.json', { requireRegularFile: true })), 'policy trust');
    const next = asRecord(active, 'policy activation');
    assertExactKeys(next, ['policy', 'bundle', 'binding', 'evidence'], 'policy activation');
    if (trust.schemaVersion !== 1 || typeof trust.publicKey !== 'string' || typeof trust.rootId !== 'string') throw new Error('DELIVERY_POLICY_TRUST_INVALID');
    verifyDeliveryPolicyActivation({ policy: next.policy, bundle: next.bundle as unknown as ReplayBundle,
      binding: next.binding as unknown as PromotionReceipt, evidence: next.evidence, publicKey: trust.publicKey, rootId: trust.rootId, inputDigest: deliveryPolicyInputDigest(run) });
    const path = join(directory, 'active.json'), prior = existsSync(path) ? readJson(resolveWorkspacePath(canonicalRoot, '.metaharness/delivery/policy/active.json', { requireRegularFile: true })) : null;
    if ((prior === null ? null : hash(prior)) !== expectedDigest) throw new Error('DELIVERY_POLICY_CAS_MISMATCH');
    if (prior !== null) atomicJson(join(directory, `previous-${hash(prior)}.json`), prior);
    atomicJson(path, next); return { digest: hash(next), previous: prior === null ? null : hash(prior) };
  });
}

/** Trust pins stay in canonical config; the candidate cannot nominate its own signer. */
export function loadDeliveryPolicy(canonicalRoot: string, run: DeliveryRun): { policy: Policy; digest: string; activation: string | null; nonapplicability?: string } {
  const configPath = join(canonicalRoot, 'config/delivery-policy.json');
  if (!existsSync(configPath)) return { policy: DELIVERY_ROOT_POLICY, digest: hash(DELIVERY_ROOT_POLICY), activation: null };
  resolveWorkspacePath(canonicalRoot, 'config/delivery-policy.json', { requireRegularFile: true });
  const trust = asRecord(readJson(configPath), 'delivery policy trust'); assertExactKeys(trust, ['schemaVersion', 'publicKey', 'rootId'], 'delivery policy trust');
  if (trust.schemaVersion !== 1 || typeof trust.publicKey !== 'string' || typeof trust.rootId !== 'string') throw new Error('DELIVERY_POLICY_TRUST_INVALID');
  const activation = resolveWorkspacePath(canonicalRoot, '.metaharness/delivery/policy/active.json', { requireRegularFile: true });
  const active = asRecord(readJson(activation), 'delivery policy activation'); assertExactKeys(active, ['policy', 'bundle', 'binding', 'evidence'], 'delivery policy activation');
  const binding = active.binding as unknown as PromotionReceipt, evidence = asRecord(active.evidence, 'policy evidence');
  if (binding.publicKey !== trust.publicKey || !verifyReceipt(binding) || binding.payload.evidenceDigest !== hash(evidence)) throw new Error('DELIVERY_POLICY_PROMOTION_INVALID');
  const policy = verifyDeliveryPolicyActivation({ policy: active.policy, bundle: active.bundle as unknown as ReplayBundle,
    binding: active.binding as unknown as PromotionReceipt, evidence: active.evidence, publicKey: trust.publicKey, rootId: trust.rootId,
    inputDigest: String(evidence.inputDigest) });
  if (evidence.targetTaskDigest !== hash(run.task)) return { policy: DELIVERY_ROOT_POLICY, digest: hash(DELIVERY_ROOT_POLICY), activation: null, nonapplicability: 'different-evaluated-task' };
  if (evidence.inputDigest !== deliveryPolicyInputDigest(run)) throw new Error('DELIVERY_POLICY_INPUT_CHANGED');
  return { policy, digest: hash(policy), activation: hash(active) };
}

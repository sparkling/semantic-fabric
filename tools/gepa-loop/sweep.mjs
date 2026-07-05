#!/usr/bin/env node
// sweep.mjs — ADR-0026 B7/B8. Runs gepaOptimize over a train slice, persists the run, then gates
// promotion by re-evaluating seed vs the sweep's best candidate on the UNSEEN holdout slice via
// darwin's own strict evaluatePromotion() predicate (Gate 3). Human-launched: prints a plan and
// requires --confirm before spending anything (mirrors ruflo-metaharness's mint/evolve safety
// convention, and ADR-0026's own "sweeps are budgeted and human-launched" rule).
//
// Usage:
//   node sweep.mjs --train-size 8 [--max-candidates 12] [--max-cost 100] [--max-stall 3]
//                   [--concurrency 4] [--holdout-size 6] [--confirm]

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { gepaOptimize, evaluatePromotion, summarizeEval } from '@metaharness/darwin/gepa';
import { makeEvaluator } from './evaluate.mjs';
import { reflect } from './reflect.mjs';

function flag(name, fallback) {
  const i = process.argv.indexOf(name);
  return i === -1 ? fallback : process.argv[i + 1];
}
function num(name, fallback) {
  const v = Number(flag(name, String(fallback)));
  return Number.isFinite(v) ? v : fallback;
}

const TRAIN_SIZE = num('--train-size', 8);
const HOLDOUT_SIZE = num('--holdout-size', 6);
const MAX_CANDIDATES = num('--max-candidates', 12);
const MAX_COST = num('--max-cost', 100);
const MAX_STALL = num('--max-stall', 3);
const CONCURRENCY = num('--concurrency', 4);
const CONFIRM = process.argv.includes('--confirm');

const DIR = import.meta.dirname;
const manifest = JSON.parse(readFileSync(join(DIR, 'corpus', 'manifest.json'), 'utf-8'));
const seed = JSON.parse(readFileSync(join(DIR, 'genome.seed.json'), 'utf-8'));

// Deterministic slice: first N ids of the manifest's own seeded-shuffle train/holdout split
// (mine-corpus.mjs already shuffled with a fixed seed=42; taking a prefix keeps it reproducible
// across runs without introducing a second RNG here).
const trainIds = manifest.train.slice(0, TRAIN_SIZE);
const holdoutIds = manifest.holdout.slice(0, HOLDOUT_SIZE);
const trainTasks = manifest.tasks.filter((t) => trainIds.includes(t.id));
const holdoutTasks = manifest.tasks.filter((t) => holdoutIds.includes(t.id));

const mutable = Object.keys(seed.components);

const plan = {
  trainSize: trainTasks.length, holdoutSize: holdoutTasks.length,
  maxCandidates: MAX_CANDIDATES, maxCost: MAX_COST, maxStall: MAX_STALL, concurrency: CONCURRENCY,
  mutable,
  estimatedMaxSpendUsd: MAX_CANDIDATES * trainTasks.length * 1.0 + holdoutTasks.length * 2 * 1.0,
  note: 'estimate uses the $1/task evaluator cap; actual spend is usually lower (many tasks resolve or budget-cap well under $1, and gepaOptimize can stop early on maxStall).',
};

if (!CONFIRM) {
  console.log('[sweep] DRY RUN -- pass --confirm to actually run. Plan:');
  console.log(JSON.stringify(plan, null, 2));
  process.exit(0);
}

console.log('[sweep] CONFIRMED -- starting real sweep. Plan:');
console.log(JSON.stringify(plan, null, 2));

const evaluate = makeEvaluator(trainTasks, { concurrency: CONCURRENCY });

const runId = new Date().toISOString().replace(/[:.]/g, '-');
const runDir = join(DIR, 'runs', runId);
mkdirSync(runDir, { recursive: true });
const eventsLog = [];
function onEvent(event, data) {
  const entry = { t: Date.now(), event, ...data };
  eventsLog.push(entry);
  process.stderr.write(`[sweep] ${event} ${JSON.stringify(data)}\n`);
  writeFileSync(join(runDir, 'events.jsonl'), eventsLog.map((e) => JSON.stringify(e)).join('\n') + '\n');
}

const result = await gepaOptimize({
  seed, evaluate, reflect, mutable,
  maxCandidates: MAX_CANDIDATES, maxCost: MAX_COST, maxStall: MAX_STALL,
  onEvent,
});

writeFileSync(join(runDir, 'result.json'), JSON.stringify(result, null, 2));
console.log(`[sweep] gepaOptimize done: best=${result.best} bestMean=${result.bestMean} pool=${result.pool.length} budget=${JSON.stringify(result.budget)}`);

if (!result.best || result.best === seed.meta.id) {
  console.log('[sweep] GATE 3: no candidate beat the seed on train -- explicit no-improvement, nothing to promote.');
  writeFileSync(join(runDir, 'promotion.json'), JSON.stringify({ promote: false, reason: 'no-candidate-beat-seed-on-train' }, null, 2));
  process.exit(0);
}

const bestEntry = result.pool.find((p) => p.id === result.best);

console.log('[sweep] GATE 3: re-evaluating seed vs best candidate on the UNSEEN holdout slice...');
const holdoutEvaluate = makeEvaluator(holdoutTasks, { concurrency: CONCURRENCY });
const [seedHoldout, candHoldout] = await Promise.all([
  holdoutEvaluate(seed),
  holdoutEvaluate(bestEntry.genome),
]);

const seedSummary = summarizeEval({
  details: seedHoldout.details, n: holdoutTasks.length,
  goldResolved: Object.values(seedHoldout.details).filter((d) => d.gold).length,
  cost: seedHoldout.cost,
});
const candSummary = summarizeEval({
  details: candHoldout.details, n: holdoutTasks.length,
  goldResolved: Object.values(candHoldout.details).filter((d) => d.gold).length,
  cost: candHoldout.cost,
});

const verdict = evaluatePromotion({ seed: seedSummary, cand: candSummary });
console.log(`[sweep] GATE 3 VERDICT: ${verdict.promote ? 'PROMOTE' : 'REJECT'} -- ${verdict.reason}`);

const promotionRecord = {
  runId, bestCandidateId: result.best, mutatedComponent: bestEntry.genome.meta?.mutated,
  train: { seedTrainScores: result.pool.find((p) => p.id === seed.meta.id)?.scores, candTrainScores: bestEntry.scores },
  holdout: { seed: seedSummary, cand: candSummary },
  verdict,
};
writeFileSync(join(runDir, 'promotion.json'), JSON.stringify(promotionRecord, null, 2));
console.log(`[sweep] wrote ${join(runDir, 'promotion.json')}`);

if (verdict.promote) {
  writeFileSync(join(runDir, 'promoted-genome.json'), JSON.stringify(bestEntry.genome, null, 2));
  console.log(`[sweep] PROMOTED. Candidate genome written to ${join(runDir, 'promoted-genome.json')}.`);
  console.log('[sweep] B8: this is NOT auto-applied to CLAUDE.md -- review the diff and land it as a normal reviewable commit.');
}

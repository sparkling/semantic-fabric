// SPDX-License-Identifier: MIT
import { execFileSync, spawnSync } from 'node:child_process';
import { realpathSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

export function coordinatorLaunch(root, sessionId) {
  if (!/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(sessionId ?? '')) {
    throw new Error('Set existing coordinator resume UUID in SEMANTIC_FABRIC_COORDINATOR_SESSION_ID');
  }
  const cwd = realpathSync(root);
  const git = (...args) => execFileSync('git', ['-C', cwd, ...args], { encoding: 'utf8' }).trim();
  if (git('rev-parse', '--show-toplevel') !== cwd || git('symbolic-ref', '--short', 'HEAD') !== 'main') {
    throw new Error('DELIVERY_MAIN_ONLY');
  }
  const prompt = [
    'Act as the existing sole Fabric native programme coordinator on local main. Read AGENTS.md, coding-harness/README.md,',
    'docs/plans/sota-application-completion-programme.md and docs/adr/ADR-0058-upstream-first-parallel-hybrid-delivery.md.',
    'Only execute application work currently authorized by the owner; a launch never resumes a paused goal by itself.',
    'Own decomposition into useful outcomes, dependency admission, file/read/resource ownership, independent review and serial integration.',
    'Use existing delivery ready manifests and runDeliveryOutcome; do not build another scheduler or use one monolithic programme worker.',
    'For each ready cohort record task/owner/source/read closure/checks, actual handoff, named resources and recipient.',
    'Choose maxConcurrency from independently ready disjoint work and observed client/host capacity, not a fixed native-session cap.',
    'Sample effective CPU, interval utilization, memory and I/O pressure before heavy work, every 30 seconds and at refill;',
    'control local Cargo/test jobs separately from remote model concurrency. Keep useful independent lanes in flight.',
    'Run npm --prefix coding-harness run delivery -- /absolute/repository ready /absolute/manifest.json with mode run.',
    'Observe delivery-pool-progress JSON on stderr and its durable evidencePath; verify actual task starts and candidate results.',
    'Upstream refills queued independent outcomes on completion. A failed lane does not block independent queued work.',
    'Keep canonical source unchanged until the cohort returns and releases its lock; per-outcome events never authorize early integration.',
    'Then serially integrate verified candidates with existing integrate/check/verify/commit/finish. Revalidate read dependencies.',
    'Only accepted integrated parents release children; immediately construct the next useful ready cohort. Report blocked/resources explicitly.',
    'Ordinary Sonnet 5.5 native planning/author/fresh review, existing Opus repair and learning stay configured; preserve explicit pins.',
    'No provider substitution, cloud launch, publication or push. Preserve WIP as unaccepted until its own checks and review pass.',
  ].join(' ');
  return { executable: 'codex', cwd, args: ['resume', sessionId, '--yolo', '--model', 'gpt-6-astra',
    '--config', 'model_reasoning_effort="medium"', '--config', 'plan_mode_reasoning_effort="medium"', prompt] };
}

export function runCoordinator(args = process.argv.slice(2), environment = process.env, launch = spawnSync) {
  if (args.length > 1 || (args.length === 1 && args[0] !== '--dry-run')) throw new Error('usage: coordinator [--dry-run]');
  const plan = coordinatorLaunch(repositoryRoot, environment.SEMANTIC_FABRIC_COORDINATOR_SESSION_ID);
  if (args[0] === '--dry-run') { console.log(JSON.stringify({ ...plan, executionStarted: false }, null, 2)); return 0; }
  const result = launch(plan.executable, plan.args, { cwd: plan.cwd, stdio: 'inherit' });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { process.exitCode = runCoordinator(); }
  catch (error) { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; }
}

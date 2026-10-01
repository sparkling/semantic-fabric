// SPDX-License-Identifier: MIT
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, realpathSync } from 'node:fs';
import { basename, dirname, isAbsolute, relative, resolve } from 'node:path';
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
    'A status or handoff answer does not pause authorized programme work; afterward service active workers/results and refill ready work. An explicit owner pause always wins.',
    'Own decomposition into useful outcomes, dependency admission, file/read/resource ownership, independent review and serial integration.',
    'Use existing delivery ready manifests and runDeliveryOutcome; do not build another scheduler or use one monolithic programme worker.',
    'Use inherited TMPDIR for new temporary candidate parents; preserve explicit parentDirectory in existing manifests and running commands.',
    'For each ready cohort record task/owner/source/read closure/checks, actual handoff, named resources and recipient.',
    'Choose maxConcurrency from independently ready disjoint work and observed client/host capacity, not a fixed native-session cap.',
    'At each refill examine every unfinished authorized outcome: assign useful independent work or name its actual prerequisite, read/write conflict, resource or authority blocker.',
    'Do not treat the previous two/three-task manifest as the full frontier. Decompose independent work inside an outcome where shared interfaces are stable.',
    'While candidates run, prepare useful next packets and read-only dependency analysis; preparation never accepts a child or bypasses source revalidation.',
    'Sample effective CPU, interval utilization, memory and I/O pressure before heavy work, every 30 seconds and at refill;',
    'control local Cargo/test jobs separately from remote model concurrency. Keep useful independent lanes in flight.',
    'Run npm --prefix coding-harness run delivery -- /absolute/repository ready /absolute/manifest.json with mode run.',
    'Observe delivery-pool-progress JSON on stderr and its durable evidencePath; verify actual task starts and candidate results.',
    'Upstream refills queued independent outcomes on completion. A failed lane does not block independent queued work.',
    'At each settled outcome, use existing integrate/check/verify/commit/finish serially without waiting for unrelated siblings; candidate success is not acceptance.',
    'Respect active read/write/resource reservations. If integration reports an active dependency, retain the candidate and retry after that dependency settles.',
    'For accepted sibling drift in Cargo inputs, preserve candidate bytes and run canonical delivery run for current-source checks and fresh review; never replay planner/author merely to refresh evidence.',
    'Only accepted integrated parents release children; dispatch their ready work while independent siblings continue. Revalidate read dependencies and report real blockers.',
    'Ordinary native Codex gpt-6.1-sol/high planning/author/fresh review use configured 9router; existing Opus repair and learning stay configured; preserve explicit pins.',
    'No provider substitution, cloud launch, publication or push. Preserve WIP as unaccepted until its own checks and review pass.',
  ].join(' ');
  return { executable: 'codex', cwd, args: ['resume', sessionId, '--yolo', '--model', 'gpt-6-astra',
    '--config', 'model_reasoning_effort="medium"', '--config', 'plan_mode_reasoning_effort="medium"', prompt] };
}

export function coordinatorEnvironment(root, environment) {
  if (environment.TMPDIR || environment.TMP || environment.TEMP) return { ...environment };
  const canonicalRoot = realpathSync(root);
  // Retained candidates belong beside the checkout on its storage volume, never inside source.
  const scratch = resolve(dirname(canonicalRoot), `.${basename(canonicalRoot)}-delivery-tmp`);
  mkdirSync(scratch, { recursive: true, mode: 0o700 });
  const physicalScratch = realpathSync(scratch);
  const relativeScratch = relative(canonicalRoot, physicalScratch);
  if (physicalScratch !== scratch || !relativeScratch
    || (!isAbsolute(relativeScratch) && relativeScratch !== '..' && !relativeScratch.startsWith('../'))) {
    throw new Error('DELIVERY_COORDINATOR_SCRATCH_NOT_ISOLATED');
  }
  return { ...environment, TMPDIR: physicalScratch };
}

export function runCoordinator(args = process.argv.slice(2), environment = process.env, launch = spawnSync) {
  if (args.length > 1 || (args.length === 1 && args[0] !== '--dry-run')) throw new Error('usage: coordinator [--dry-run]');
  const plan = coordinatorLaunch(repositoryRoot, environment.SEMANTIC_FABRIC_COORDINATOR_SESSION_ID);
  if (args[0] === '--dry-run') { console.log(JSON.stringify({ ...plan, executionStarted: false }, null, 2)); return 0; }
  const result = launch(plan.executable, plan.args, { cwd: plan.cwd, stdio: 'inherit',
    env: coordinatorEnvironment(plan.cwd, environment) });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { process.exitCode = runCoordinator(); }
  catch (error) { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; }
}

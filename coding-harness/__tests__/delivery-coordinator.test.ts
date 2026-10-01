import { describe, expect, it, vi } from 'vitest';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { coordinatorEnvironment, coordinatorLaunch, runCoordinator } from '../scripts/run-delivery-coordinator.mjs';

const root = new URL('../../', import.meta.url).pathname;
const sessionId = '01a0dd65-d69a-7841-9e52-c1c8617c333a';

describe('native programme coordinator entrypoint', () => {
  it('resumes the exact existing conversation on main with explicit model and effort', () => {
    const plan = coordinatorLaunch(root, sessionId);
    expect(plan.executable).toBe('codex');
    expect(plan.args.slice(0, 9)).toEqual(['resume', sessionId, '--yolo', '--model', 'gpt-6-astra',
      '--config', 'model_reasoning_effort="medium"', '--config', 'plan_mode_reasoning_effort="medium"']);
    expect(plan.args.at(-1)).toContain('ready /absolute/manifest.json with mode run');
    expect(plan.args.at(-1)).toContain('without waiting for unrelated siblings');
    expect(plan.args.at(-1)).toContain('exact approved replacement candidateRoot and expectedDigest');
    expect(plan.args.at(-1)).toContain('paused rejected prepared integration');
    expect(plan.args.at(-1)).toContain('explicit recover:true once');
    expect(plan.args.at(-1)).toContain('interrupted apply resumes through ordinary integrate without recover:true');
    expect(plan.args.at(-1)).toContain('revalidateAgainst:{commit,sourceDigest} from current canonical HEAD/snapshot');
    expect(plan.args.at(-1)).toContain('DELIVERY_INTEGRATION_RECOVERY_SOURCE_CHANGED refusal');
    expect(plan.args.at(-1)).toContain('Rerun every canonical check and fresh independent review before verify/commit/finish');
    expect(plan.args.at(-1)).toContain('active read/write/resource reservations');
    expect(plan.args.at(-1)).toContain('Only accepted integrated parents release children');
    expect(plan.args.at(-1)).toContain('examine every unfinished authorized outcome');
    expect(plan.args.at(-1)).toContain('previous two/three-task manifest');
    expect(plan.args.at(-1)).toContain('A status or handoff answer does not pause authorized programme work');
    expect(plan.args.at(-1)).toContain('An explicit owner pause always wins');
    expect(plan.args.at(-1)).toContain('never replay planner/author merely to refresh evidence');
    expect(plan.args.at(-1)).toContain('native Codex gpt-6.1-sol/high planning/author/fresh review use configured 9router');
    expect(plan.args.at(-1)).not.toContain('Ordinary Sonnet');
  });

  it('previews without launching and forwards native exit status without fallback', () => {
    const launch = vi.fn(() => ({ status: 7 }));
    const output = vi.spyOn(console, 'log').mockImplementation(() => {});
    try {
      const environment = { SEMANTIC_FABRIC_COORDINATOR_SESSION_ID: sessionId,
        TMPDIR: '/explicit/operator/scratch', ANTHROPIC_BASE_URL: 'http://gateway.invalid',
        ANTHROPIC_AUTH_TOKEN: 'fixture-only', CODEX_HOME: '/configured/codex' };
      expect(runCoordinator(['--dry-run'], environment, launch)).toBe(0);
      expect(launch).not.toHaveBeenCalled();
      expect(JSON.parse(output.mock.calls[0][0])).toMatchObject({ executionStarted: false, executable: 'codex' });
      expect(runCoordinator([], environment, launch)).toBe(7);
      expect(launch).toHaveBeenCalledTimes(1);
      expect(launch.mock.calls[0]).toEqual(['codex', coordinatorLaunch(root, sessionId).args,
        { cwd: coordinatorLaunch(root, sessionId).cwd, stdio: 'inherit', env: environment }]);
    } finally { output.mockRestore(); }
  });

  it('creates private sibling scratch on the checkout volume without mutating inherited environment', () => {
    const directory = mkdtempSync(join(tmpdir(), 'fabric-coordinator-storage-'));
    try {
      const canonical = join(directory, 'semantic-fabric'); mkdirSync(canonical);
      const environment = { SEMANTIC_FABRIC_COORDINATOR_SESSION_ID: sessionId, NATIVE_ROUTE: 'unchanged' };
      const scoped = coordinatorEnvironment(canonical, environment);
      expect(scoped).toEqual({ ...environment, TMPDIR: join(realpathSync(directory), '.semantic-fabric-delivery-tmp') });
      expect(dirname(scoped.TMPDIR)).toBe(dirname(realpathSync(canonical)));
      expect(statSync(scoped.TMPDIR).mode & 0o777).toBe(0o700);
      expect(environment).not.toHaveProperty('TMPDIR');
      expect(coordinatorEnvironment(canonical, environment)).toEqual(scoped);
    } finally { rmSync(directory, { recursive: true, force: true }); }
  });

  it('does not prepare scratch during dry-run', () => {
    const launch = vi.fn(), scratchRead = vi.fn(() => { throw new Error('scratch preparation forbidden'); });
    const output = vi.spyOn(console, 'log').mockImplementation(() => {});
    try {
      const environment = { SEMANTIC_FABRIC_COORDINATOR_SESSION_ID: sessionId, get TMPDIR() { return scratchRead(); } };
      expect(runCoordinator(['--dry-run'], environment, launch)).toBe(0);
      expect(scratchRead).not.toHaveBeenCalled();
      expect(launch).not.toHaveBeenCalled();
    } finally { output.mockRestore(); }
  });

  it.each(['TMPDIR', 'TMP', 'TEMP'])('preserves explicit %s without touching operator scratch', key => {
    const environment = { [key]: '/explicit/operator/scratch', NATIVE_ROUTE: 'unchanged' };
    expect(coordinatorEnvironment('/does-not-exist', environment)).toEqual(environment);
  });

  it('refuses default scratch symlinked into canonical source', () => {
    const directory = mkdtempSync(join(tmpdir(), 'fabric-coordinator-storage-'));
    try {
      const canonical = join(directory, 'semantic-fabric'); mkdirSync(canonical);
      symlinkSync(canonical, join(directory, `.${basename(canonical)}-delivery-tmp`), 'dir');
      expect(() => coordinatorEnvironment(canonical, {})).toThrow('DELIVERY_COORDINATOR_SCRATCH_NOT_ISOLATED');
    } finally { rmSync(directory, { recursive: true, force: true }); }
  });

  it('refuses default scratch redirected to external host tmp', () => {
    const directory = mkdtempSync(join(tmpdir(), 'fabric-coordinator-storage-'));
    try {
      const canonical = join(directory, 'semantic-fabric'); mkdirSync(canonical);
      symlinkSync(tmpdir(), join(directory, `.${basename(canonical)}-delivery-tmp`), 'dir');
      expect(() => coordinatorEnvironment(canonical, {})).toThrow('DELIVERY_COORDINATOR_SCRATCH_NOT_ISOLATED');
    } finally { rmSync(directory, { recursive: true, force: true }); }
  });

  it('refuses missing session and unsupported launch options before execution', () => {
    const launch = vi.fn();
    expect(() => runCoordinator([], {}, launch)).toThrow('existing coordinator resume UUID');
    expect(() => runCoordinator(['--new'], {}, launch)).toThrow('usage');
    expect(launch).not.toHaveBeenCalled();
  });

  it('preflights the native whole-outcome fixture with pinned routes and no model execution', () => {
    const script = join(root, 'coding-harness/scripts/live-delivery-pool-proof.mjs');
    const output = execFileSync(process.execPath, [script, '--whole-outcome', '--native', '--preflight'], { cwd: root, encoding: 'utf8' });
    const final = JSON.parse(output.trim().split('\n').at(-1)!);
    expect(final.phase).toBe('preflight-complete');
    const manifest = JSON.parse(readFileSync(join(final.proofRoot, 'manifest.json'), 'utf8'));
    expect(manifest.maxConcurrency).toBe(2);
    expect(manifest.outcomes[0].task.readPaths).toContain(manifest.outcomes[1].task.scope[0]);
    for (const outcome of manifest.outcomes) {
      expect(outcome.task.requested).toEqual({ host: 'claude-code', model: 'cc/claude-sonnet-5-5[1m]', effort: 'high' });
      expect(outcome.task.reviewer).toEqual(outcome.task.requested);
      expect(outcome.handoff.authentication).toBe('native-subscription');
    }
    const prerequisite = JSON.parse(readFileSync(join(final.proofRoot, 'prerequisites.json'), 'utf8'));
    expect(prerequisite.preflight).toBe(true);
    expect(prerequisite.dependentProof.reason).toContain('without waiting for the independent cohort');
    expect(prerequisite.excluded).toEqual(['path', 'resource']);
    expect(prerequisite.baseline.filter(row => row.checkId === 'acceptance').every(row => !row.passed && row.exitCode === 1)).toBe(true);
    const invalid = spawnSync(process.execPath, [script, '--whole-outcome', '--native', '--native'], { encoding: 'utf8' });
    expect(invalid.status).not.toBe(0);
    expect(invalid.stderr).toContain('usage:');
  });
});

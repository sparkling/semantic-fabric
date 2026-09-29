import { describe, expect, it, vi } from 'vitest';
import { coordinatorLaunch, runCoordinator } from '../scripts/run-delivery-coordinator.mjs';

const root = new URL('../../', import.meta.url).pathname;
const sessionId = '01a0dd65-d69a-7841-9e52-c1c8617c333a';

describe('native programme coordinator entrypoint', () => {
  it('resumes the exact existing conversation on main with explicit model and effort', () => {
    const plan = coordinatorLaunch(root, sessionId);
    expect(plan.executable).toBe('codex');
    expect(plan.args.slice(0, 9)).toEqual(['resume', sessionId, '--yolo', '--model', 'gpt-6-astra',
      '--config', 'model_reasoning_effort="medium"', '--config', 'plan_mode_reasoning_effort="medium"']);
    expect(plan.args.at(-1)).toContain('ready /absolute/manifest.json with mode run');
    expect(plan.args.at(-1)).toContain('unchanged until the cohort returns');
    expect(plan.args.at(-1)).toContain('Only accepted integrated parents release children');
  });

  it('previews without launching and forwards native exit status without fallback', () => {
    const launch = vi.fn(() => ({ status: 7 }));
    const output = vi.spyOn(console, 'log').mockImplementation(() => {});
    try {
      const environment = { SEMANTIC_FABRIC_COORDINATOR_SESSION_ID: sessionId };
      expect(runCoordinator(['--dry-run'], environment, launch)).toBe(0);
      expect(launch).not.toHaveBeenCalled();
      expect(JSON.parse(output.mock.calls[0][0])).toMatchObject({ executionStarted: false, executable: 'codex' });
      expect(runCoordinator([], environment, launch)).toBe(7);
      expect(launch).toHaveBeenCalledTimes(1);
      expect(launch.mock.calls[0]).toEqual(['codex', coordinatorLaunch(root, sessionId).args,
        { cwd: coordinatorLaunch(root, sessionId).cwd, stdio: 'inherit' }]);
    } finally { output.mockRestore(); }
  });

  it('refuses missing session and unsupported launch options before execution', () => {
    const launch = vi.fn();
    expect(() => runCoordinator([], {}, launch)).toThrow('existing coordinator resume UUID');
    expect(() => runCoordinator(['--new'], {}, launch)).toThrow('usage');
    expect(launch).not.toHaveBeenCalled();
  });
});

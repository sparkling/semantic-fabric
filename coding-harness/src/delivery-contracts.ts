// SPDX-License-Identifier: MIT
import { codexReasoningArguments } from './models/native-adapter-contracts.js';
import type { CodexReasoningEffort, NativeHost } from './models/types.js';
import { asRecord, assertExactKeys, normalizeWorkspacePath } from './contracts.js';

export interface DeliveryRoute {
  host: NativeHost;
  model: string;
  effort: CodexReasoningEffort | 'default';
}
export type TaskClass = 'mechanical' | 'pattern' | 'implementation' | 'correctness' | 'difficult';
export interface DeliveryTask {
  schemaVersion: 1;
  id: string;
  requirement: string;
  owner: string;
  thread: string;
  taskClass: TaskClass;
  host: NativeHost;
  scope: string[];
  checks: { id: string; kind: 'acceptance' | 'build'; argv: string[]; cwd: string;
    timeoutMs?: number; maxOutputBytes?: number }[];
  requested?: DeliveryRoute;
  selectionReason?: string;
  preserveMainModel?: boolean;
  explicitUltra?: boolean;
  rufloTaskId?: string;
  adoptExistingChanges?: string[];
}
export interface NativeHandoff extends DeliveryRoute {
  executorId: string;
  authentication: 'native-subscription';
  observation: string;
}

export function nonempty(value: unknown, label: string): string {
  if (typeof value !== 'string' || !value.trim() || value.includes('\0')) {
    throw new Error(`DELIVERY_INVALID:${label}`);
  }
  return value;
}
export function identifier(value: unknown): string {
  const id = nonempty(value, 'id');
  if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]{0,100}$/.test(id)) throw new Error('DELIVERY_INVALID_ID');
  return id;
}
export function route(value: unknown): DeliveryRoute {
  const r = asRecord(value, 'route');
  assertExactKeys(r, ['host', 'model', 'effort'], 'route');
  if (r.host !== 'codex' && r.host !== 'claude-code') throw new Error('DELIVERY_INVALID_HOST');
  const model = nonempty(r.model, 'model');
  if (!/^[a-zA-Z0-9._-]+$/.test(model) || /openrouter|requesty/i.test(model)) {
    throw new Error('DELIVERY_NATIVE_MODEL_REQUIRED');
  }
  if (r.effort !== 'default') codexReasoningArguments(r.effort as CodexReasoningEffort);
  if (r.effort === undefined || (r.host === 'claude-code' && r.effort !== 'default')) {
    throw new Error('DELIVERY_EXPLICIT_NATIVE_EFFORT_REQUIRED');
  }
  return { host: r.host, model, effort: r.effort as DeliveryRoute['effort'] };
}
export function selectDeliveryRoute(task: DeliveryTask): DeliveryRoute {
  if (task.preserveMainModel && !task.requested) throw new Error('DELIVERY_CURRENT_MAIN_ROUTE_REQUIRED');
  if (task.taskClass === 'difficult' && !task.selectionReason?.trim()) throw new Error('DELIVERY_NAMED_DIFFICULTY_REQUIRED');
  if (task.requested) {
    const selected = route(task.requested);
    if (selected.host !== task.host || !task.selectionReason?.trim()) {
      throw new Error('DELIVERY_SELECTION_REASON_REQUIRED');
    }
    if (selected.effort === 'ultra' && !task.explicitUltra && !task.preserveMainModel) {
      throw new Error('DELIVERY_ULTRA_REQUIRES_EXPLICIT_REQUEST');
    }
    return selected; // No clamp, automatic retry, provider fallback, or quota routing.
  }
  if (task.host === 'claude-code') {
    return { host: task.host, model: task.taskClass === 'mechanical' ? 'haiku'
      : task.taskClass === 'difficult' ? 'opus' : 'sonnet', effort: 'default' };
  }
  const choices: Record<TaskClass, [string, DeliveryRoute['effort']]> = {
    mechanical: ['gpt-5.6-luna', 'low'], pattern: ['gpt-5.6-terra', 'medium'],
    implementation: ['gpt-5.6-sol', 'medium'], correctness: ['gpt-5.6-sol', 'high'],
    difficult: ['gpt-6-astra', 'high'],
  };
  const [model, effort] = choices[task.taskClass];
  return { host: task.host, model, effort };
}
export function parseDeliveryTask(value: unknown): DeliveryTask {
  const t = asRecord(value, 'delivery task');
  const required = ['schemaVersion', 'id', 'requirement', 'owner', 'thread', 'taskClass', 'host', 'scope', 'checks'];
  const optional = ['requested', 'selectionReason', 'preserveMainModel', 'explicitUltra', 'rufloTaskId', 'adoptExistingChanges'];
  for (const key of required) if (!(key in t)) throw new Error(`DELIVERY_MISSING:${key}`);
  for (const key of Object.keys(t)) if (![...required, ...optional].includes(key)) {
    throw new Error(`DELIVERY_UNKNOWN_FIELD:${key}`);
  }
  if (t.schemaVersion !== 1 || !['mechanical', 'pattern', 'implementation', 'correctness', 'difficult'].includes(String(t.taskClass))) {
    throw new Error('DELIVERY_INVALID_TASK');
  }
  if (t.host !== 'codex' && t.host !== 'claude-code') throw new Error('DELIVERY_INVALID_HOST');
  for (const key of ['preserveMainModel', 'explicitUltra']) {
    if (t[key] !== undefined && typeof t[key] !== 'boolean') throw new Error(`DELIVERY_INVALID:${key}`);
  }
  if (!Array.isArray(t.scope) || !t.scope.length || !Array.isArray(t.checks) || !t.checks.length) {
    throw new Error('DELIVERY_SCOPE_AND_CHECKS_REQUIRED');
  }
  const scope = t.scope.map(p => normalizeWorkspacePath(nonempty(p, 'scope'), 'scope'));
  if (new Set(scope).size !== scope.length || scope.some(p => /(^|\/)(\.git|\.env(?:\..*)?|\.metaharness)(\/|$)/.test(p))) {
    throw new Error('DELIVERY_INVALID_SCOPE');
  }
  const checks: DeliveryTask['checks'] = t.checks.map(v => {
    const c = asRecord(v, 'check');
    for (const key of Object.keys(c)) if (!['id', 'kind', 'argv', 'cwd', 'timeoutMs', 'maxOutputBytes'].includes(key)) throw new Error('DELIVERY_INVALID_CHECK_FIELD');
    if (c.kind !== 'acceptance' && c.kind !== 'build') throw new Error('DELIVERY_INVALID_CHECK_KIND');
    if (!Array.isArray(c.argv) || !c.argv.length) throw new Error('DELIVERY_COMMAND_REQUIRED');
    const argv = c.argv.map(v => nonempty(v, 'argv'));
    const cwd = normalizeWorkspacePath(nonempty(c.cwd, 'cwd'), 'cwd', true);
    if (argv[0] === 'git') {
      if (JSON.stringify(argv) !== JSON.stringify(['git', 'diff', '--check'])) throw new Error('DELIVERY_GIT_READ_ONLY');
    } else if (argv[0] === 'npm' || argv[0] === 'node') {
      if (cwd !== 'coding-harness' && !cwd.startsWith('coding-harness/')) throw new Error('DELIVERY_NODE_DEV_ONLY');
      if (argv[0] === 'npm' && !(argv[1] === 'test' || (argv[1] === 'run' && ['build', 'test'].includes(argv[2])))) {
        throw new Error('DELIVERY_NPM_BUILD_TEST_ONLY');
      }
      if (argv[0] === 'node') normalizeWorkspacePath(nonempty(argv[1], 'node script'), 'node script');
    } else if (argv[0] !== 'cargo') throw new Error('DELIVERY_BUILD_TOOL_REQUIRED');
    if (argv[0] === 'cargo' && !['build', 'check', 'test', 'clippy', 'fmt'].includes(argv[1])) {
      throw new Error('DELIVERY_NO_PUBLICATION');
    }
    const limits: { timeoutMs?: number; maxOutputBytes?: number } = {};
    for (const key of ['timeoutMs', 'maxOutputBytes'] as const) if (c[key] !== undefined) {
      const maximum = key === 'timeoutMs' ? 86_400_000 : 100_000_000;
      if (!Number.isSafeInteger(c[key]) || (c[key] as number) < 1 || (c[key] as number) > maximum) throw new Error('DELIVERY_INVALID_CHECK_LIMIT');
      limits[key] = c[key] as number;
    }
    return { id: identifier(c.id), kind: c.kind, argv, cwd, ...limits };
  });
  if (new Set(checks.map(c => c.id)).size !== checks.length
    || !checks.some(c => c.kind === 'acceptance') || !checks.some(c => c.kind === 'build')) {
    throw new Error('DELIVERY_ACCEPTANCE_AND_BUILD_REQUIRED');
  }
  const task: DeliveryTask = {
    schemaVersion: 1, id: identifier(t.id), requirement: nonempty(t.requirement, 'requirement'),
    owner: nonempty(t.owner, 'owner'), thread: nonempty(t.thread, 'thread'), taskClass: t.taskClass as TaskClass,
    host: t.host, scope, checks,
  };
  if (task.id === 'active') throw new Error('DELIVERY_RESERVED_ID');
  if (t.adoptExistingChanges !== undefined) {
    if (!Array.isArray(t.adoptExistingChanges)) throw new Error('DELIVERY_INVALID_ADOPTION');
    task.adoptExistingChanges = t.adoptExistingChanges.map(p => nonempty(p, 'adoption'));
    if (new Set(task.adoptExistingChanges).size !== task.adoptExistingChanges.length
      || task.adoptExistingChanges.some(p => !scope.includes(p))) throw new Error('DELIVERY_INVALID_ADOPTION');
  }
  if (t.requested !== undefined) task.requested = route(t.requested);
  if (t.selectionReason !== undefined) task.selectionReason = nonempty(t.selectionReason, 'selectionReason');
  if (t.rufloTaskId !== undefined) task.rufloTaskId = nonempty(t.rufloTaskId, 'rufloTaskId');
  if (t.preserveMainModel !== undefined) task.preserveMainModel = t.preserveMainModel as boolean;
  if (t.explicitUltra !== undefined) task.explicitUltra = t.explicitUltra as boolean;
  selectDeliveryRoute(task);
  return task;
}

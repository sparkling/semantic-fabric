// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  decideSupervisorRegistrationV1,
  type AuthenticatedTransportPeerV1,
  type SupervisorRegistrationDecisionPortsV1,
} from '../src/index.js';
import {
  ACTIVE_HEAD, ACTIVE_SEMANTIC_HEAD, CANONICAL_REQUEST, DIGEST, PROJECT,
  READY_RECEIPT, READY_SEMANTIC_RECEIPT, REGISTERED_RUN, sha256Text,
} from './registration-fixtures.js';

const PEER = Symbol('authenticated-adapter-boundary-peer') as AuthenticatedTransportPeerV1;
const MAPPED = { kind: 'mapped' as const, project: PROJECT };
type Step = { name: string; reply: unknown };

function scripted(steps: readonly Step[]) {
  const queue = [...steps];
  const calls: Array<{ name: string; input: unknown }> = [];
  const call = async <T>(name: string, input: unknown): Promise<T> => {
    calls.push({ name, input });
    const step = queue.shift();
    if (step?.name !== name) throw new Error(`unexpected ${name}; expected ${step?.name}`);
    return step.reply as T;
  };
  const ports: SupervisorRegistrationDecisionPortsV1 = {
    mapAuthenticatedPeer: (input) => call('map', input),
    lookupExactCommittedResult: (input) => call('exact', input),
    readActiveAuthorityHead: (input) => call('head', input),
    readRequiredPredecessorReceipt: (input) => call('receipt', input),
    readRunState: (input) => call('run', input),
  };
  return { ports, calls, remaining: () => queue.length };
}

function baseSteps(): Step[] {
  return [
    { name: 'map', reply: MAPPED },
    { name: 'exact', reply: { kind: 'absent' } },
    { name: 'head', reply: ACTIVE_HEAD },
    { name: 'receipt', reply: READY_RECEIPT },
    { name: 'run', reply: { kind: 'absent' } },
  ];
}

function outcome(value: Awaited<ReturnType<typeof decideSupervisorRegistrationV1>>) {
  return value.decisionKind === 'fixed-response' || value.decisionKind === 'indeterminate'
    ? value.response.outcomeCode : null;
}

function accessorRecord(value: unknown): { value: unknown; reads: () => number } {
  const copy = structuredClone(value) as Record<string, unknown>;
  const key = Object.keys(copy)[0];
  if (key === undefined) throw new Error('fixture must have a property');
  const original = copy[key];
  let count = 0;
  Object.defineProperty(copy, key, {
    enumerable: true,
    get: () => { count += 1; return original; },
  });
  return { value: copy, reads: () => count };
}

describe('registration adapter trust boundary V1', () => {
  it.each(['map', 'exact', 'head', 'receipt', 'run'])(
    'rejects a %s Proxy with the exact call prefix', async (failed) => {
      const steps = baseSteps();
      const index = steps.findIndex(({ name }) => name === failed);
      const target = structuredClone(steps[index]!.reply) as object;
      steps[index] = { name: failed, reply: new Proxy(target, {}) };
      const expected = steps.slice(0, index + 1);
      const run = scripted(expected);

      const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, run.ports);

      expect(decision.decisionKind).toBe('indeterminate');
      expect(outcome(decision)).toBe('transaction-resolution-unknown-v2');
      expect(run.calls.map(({ name }) => name)).toEqual(expected.map(({ name }) => name));
      expect(run.remaining()).toBe(0);
    },
  );

  it.each(['map', 'exact', 'head', 'receipt', 'run'])(
    'rejects a %s accessor without invoking it', async (failed) => {
      const steps = baseSteps();
      const index = steps.findIndex(({ name }) => name === failed);
      const hostile = accessorRecord(steps[index]!.reply);
      steps[index] = { name: failed, reply: hostile.value };
      const expected = steps.slice(0, index + 1);
      const run = scripted(expected);

      const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, run.ports);

      expect(decision.decisionKind).toBe('indeterminate');
      expect(outcome(decision)).toBe('transaction-resolution-unknown-v2');
      expect(run.calls.map(({ name }) => name)).toEqual(expected.map(({ name }) => name));
      expect(hostile.reads()).toBe(0);
      expect(run.remaining()).toBe(0);
    },
  );

  it.each([
    ['genesis', ACTIVE_HEAD, READY_RECEIPT, { kind: 'absent' }],
    ['semantic predecessor', ACTIVE_SEMANTIC_HEAD, READY_SEMANTIC_RECEIPT, REGISTERED_RUN],
  ])('passes exact H/P/R arguments on the %s path', async (_label, head, receipt, state) => {
    const run = scripted([
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: head }, { name: 'receipt', reply: receipt },
      { name: 'run', reply: state },
    ]);

    await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, run.ports);

    expect(run.calls[2]?.input).toEqual({ projectAuthorityDigest: DIGEST.project });
    expect(run.calls[3]?.input).toEqual({
      projectAuthorityDigest: DIGEST.project,
      requiredPredecessor: head.requiredPredecessor,
    });
    expect(run.calls[4]?.input).toEqual({
      projectAuthorityDigest: DIGEST.project, runId: 'capture_run_20260829',
    });
  });

  it.each([
    ['digest', { projectAuthorityDigest: sha256Text('foreign-project') }],
    ['principal', { principalId: 'foreign_project_principal_20260830' }],
    ['policy', { authenticationPolicyDigest: sha256Text('foreign-policy') }],
  ])('rejects a stored-head project with a foreign %s', async (_label, mutation) => {
    const head = { ...ACTIVE_HEAD, project: { ...PROJECT, ...mutation } };
    const run = scripted([
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: head },
    ]);

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, run.ports);

    expect(decision.decisionKind).toBe('indeterminate');
    expect(outcome(decision)).toBe('transaction-resolution-unknown-v2');
    expect(run.calls.map(({ name }) => name)).toEqual(['map', 'exact', 'head']);
  });

  it('maps an explicit authority-head admission denial to the fixed 403', async () => {
    const run = scripted([
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: { kind: 'not-admitted' } },
    ]);
    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, run.ports);
    expect(decision.decisionKind).toBe('fixed-response');
    expect(outcome(decision)).toBe('registration-not-admitted-v2');
  });

  it('rejects the current principal after an exact miss despite a consistent rotated head', async () => {
    const rotatedProject = { ...PROJECT, principalId: 'rotated_project_principal_20260830' };
    const run = scripted([
      { name: 'map', reply: { kind: 'mapped', project: rotatedProject } },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: { ...ACTIVE_HEAD, project: rotatedProject } },
    ]);
    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, run.ports);
    expect(decision.decisionKind).toBe('fixed-response');
    expect(outcome(decision)).toBe('registration-not-admitted-v2');
    expect(run.calls.map(({ name }) => name)).toEqual(['map', 'exact', 'head']);
  });

  it('creates run genesis after an unrelated semantic global predecessor', async () => {
    const run = scripted([
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: ACTIVE_SEMANTIC_HEAD },
      { name: 'receipt', reply: READY_SEMANTIC_RECEIPT },
      { name: 'run', reply: { kind: 'absent' } },
    ]);
    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, run.ports);
    expect(decision.decisionKind).toBe('append-registration-candidate');
    if (decision.decisionKind !== 'append-registration-candidate') return;
    expect(decision.candidate).toMatchObject({
      expectedNextGlobalSequence: '2', runSequence: '0',
      previousGlobal: READY_SEMANTIC_RECEIPT.previousGlobal,
      previousRun: { kind: 'run-genesis', eventDigest: null },
    });
  });
});

// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  decideSupervisorRegistrationV1,
  type AuthenticatedTransportPeerV1,
  type SupervisorRegistrationDecisionPortsV1,
} from '../src/index.js';
import {
  ACTIVE_HEAD, ACTIVE_SEMANTIC_HEAD, CANONICAL_REQUEST, DIGEST, PROJECT,
  READY_RECEIPT, READY_SEMANTIC_RECEIPT, REGISTERED_RUN, exactStoredResult, sha256Text,
} from './registration-fixtures.js';

const PEER = Symbol('authenticated-adversarial-peer') as AuthenticatedTransportPeerV1;
const MAPPED = { kind: 'mapped' as const, project: PROJECT };
type Step = Readonly<{ name: string; reply?: unknown; error?: Error; before?: () => void }>;

function scripted(steps: readonly Step[]) {
  const queue = [...steps];
  const calls: Array<{ name: string; input: unknown }> = [];
  const call = async <T>(name: string, input: unknown): Promise<T> => {
    calls.push({ name, input });
    const step = queue.shift();
    if (step?.name !== name) throw new Error(`unexpected ${name}; expected ${step?.name}`);
    step.before?.();
    if (step.error !== undefined) throw step.error;
    return step.reply as T;
  };
  const ports: SupervisorRegistrationDecisionPortsV1 = {
    mapAuthenticatedPeer: (peer) => call('map', peer),
    lookupExactCommittedResult: (input) => call('exact', input),
    readActiveAuthorityHead: (input) => call('head', input),
    readRequiredPredecessorReceipt: (input) => call('receipt', input),
    readRunState: (input) => call('run', input),
  };
  return { ports, calls, remaining: () => queue.length };
}

function fullSteps(run: unknown = { kind: 'absent' }): Step[] {
  const { head, receipt } = stateReads(run);
  return [
    { name: 'map', reply: MAPPED },
    { name: 'exact', reply: { kind: 'absent' } },
    { name: 'head', reply: head },
    { name: 'receipt', reply: receipt },
    { name: 'run', reply: run },
  ];
}

function stateReads(run: unknown): { head: unknown; receipt: unknown } {
  const state = run as { kind?: unknown; lastRunGlobalSequence?: unknown;
    lastRunSequence?: unknown; lastRunEventDigest?: unknown };
  if (state?.kind === 'absent' || state?.lastRunSequence === undefined) {
    return { head: ACTIVE_HEAD, receipt: READY_RECEIPT };
  }
  if (state.lastRunSequence === '0'
    && state.lastRunEventDigest === REGISTERED_RUN.registrationEventDigest) {
    return { head: ACTIVE_SEMANTIC_HEAD, receipt: READY_SEMANTIC_RECEIPT };
  }
  const expectedNextGlobalSequence =
    (BigInt(String(state.lastRunGlobalSequence)) + 1n).toString();
  const requiredPredecessor = {
    kind: 'semantic-event' as const, eventDigest: state.lastRunEventDigest,
  };
  return {
    head: { ...ACTIVE_SEMANTIC_HEAD, expectedNextGlobalSequence, requiredPredecessor },
    receipt: {
      kind: 'ready',
      previousGlobal: {
        ...requiredPredecessor,
        semanticReceiptDigest: sha256Text(`receipt:${expectedNextGlobalSequence}`),
      },
    },
  };
}

function outcome(value: Awaited<ReturnType<typeof decideSupervisorRegistrationV1>>) {
  return value.decisionKind === 'fixed-response' || value.decisionKind === 'indeterminate'
    ? value.response.outcomeCode : null;
}

async function expectIndeterminateAt(failed: string, reply: unknown): Promise<void> {
  const steps = fullSteps();
  const index = steps.findIndex(({ name }) => name === failed);
  steps[index] = { name: failed, reply };
  const expected = steps.slice(0, index + 1);
  const run = scripted(expected);
  const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, run.ports);
  expect(decision.decisionKind).toBe('indeterminate');
  expect(outcome(decision)).toBe('transaction-resolution-unknown-v2');
  expect(run.calls.map(({ name }) => name)).toEqual(expected.map(({ name }) => name));
  expect(run.remaining()).toBe(0);
}

function nonPlain(value: Record<string, unknown>): unknown {
  return Object.assign(Object.create(null), value);
}

describe('registration decision V1 adversarial snapshots', () => {
  it.each(['map', 'exact', 'head', 'receipt', 'run'])(
    'fails closed without retry when %s throws', async (failed) => {
      const steps = fullSteps();
      const index = steps.findIndex(({ name }) => name === failed);
      steps[index] = { name: failed, error: new Error('adapter failed') };
      const expected = steps.slice(0, index + 1);
      const run = scripted(expected);

      const decision = await decideSupervisorRegistrationV1(
        CANONICAL_REQUEST, PEER, run.ports,
      );

      expect(decision.decisionKind).toBe('indeterminate');
      expect(run.calls.map(({ name }) => name)).toEqual(expected.map(({ name }) => name));
      expect(run.remaining()).toBe(0);
    },
  );

  it.each(['map', 'exact', 'head', 'receipt', 'run'])(
    'fails closed without retry on an explicit %s indeterminate read', async (failed) => {
      await expectIndeterminateAt(failed, { kind: 'indeterminate' });
    },
  );

  it.each([
    ['map', undefined],
    ['map', { kind: 'unknown' }],
    ['map', { ...MAPPED, extra: false }],
    ['map', nonPlain(MAPPED)],
    ['exact', null],
    ['exact', { kind: 'unknown' }],
    ['exact', { kind: 'absent', extra: false }],
    ['exact', nonPlain({ kind: 'absent' })],
    ['head', undefined],
    ['head', { kind: 'unknown' }],
    ['head', { ...ACTIVE_HEAD, extra: false }],
    ['head', nonPlain(ACTIVE_HEAD)],
    ['receipt', null],
    ['receipt', { kind: 'unknown' }],
    ['receipt', { kind: 'pending', extra: false }],
    ['receipt', nonPlain(READY_RECEIPT)],
    ['receipt', { ...READY_RECEIPT, previousGlobal: {
      ...READY_RECEIPT.previousGlobal,
      kind: 'semantic-event',
      eventDigest: sha256Text('wrong-predecessor'),
    } }],
    ['run', undefined],
    ['run', { kind: 'unknown' }],
    ['run', { kind: 'absent', extra: false }],
    ['run', nonPlain({ kind: 'absent' })],
    ['run', { ...REGISTERED_RUN, projectAuthorityDigest: sha256Text('wrong-project') }],
    ['run', { ...REGISTERED_RUN, runId: 'wrong_run_20260830' }],
  ] as const)('rejects malformed %s adapter state %#', async (failed, reply) => {
    await expectIndeterminateAt(failed, reply);
  });

  it('passes only frozen scope wrappers to asynchronous ports', async () => {
    const assertFrozen = (value: unknown) => {
      expect(Object.isFrozen(value)).toBe(true);
      return value as Record<string, unknown>;
    };
    const ports: SupervisorRegistrationDecisionPortsV1 = {
      mapAuthenticatedPeer: async (peer) => {
        expect(peer).toBe(PEER);
        return MAPPED;
      },
      lookupExactCommittedResult: async (input) => {
        assertFrozen(input);
        expect(() => { (input as any).projectAuthorityDigest = sha256Text('attack'); })
          .toThrow();
        return { kind: 'absent' };
      },
      readActiveAuthorityHead: async (input) => {
        assertFrozen(input);
        expect(() => { (input as any).projectAuthorityDigest = sha256Text('attack'); })
          .toThrow();
        return ACTIVE_HEAD;
      },
      readRequiredPredecessorReceipt: async (input) => {
        assertFrozen(input);
        assertFrozen(input.requiredPredecessor);
        expect(() => { (input.requiredPredecessor as any).kind = 'semantic-event'; })
          .toThrow();
        return READY_RECEIPT;
      },
      readRunState: async (input) => {
        assertFrozen(input);
        expect(() => { (input as any).runId = 'capture_run_attack'; }).toThrow();
        return { kind: 'absent' };
      },
    };

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

    expect(decision.decisionKind).toBe('append-registration-candidate');
  });

  it('severs every adapter result before the next await and before returning', async () => {
    const mapping = structuredClone(MAPPED);
    const head = structuredClone(ACTIVE_SEMANTIC_HEAD);
    const receipt = structuredClone(READY_SEMANTIC_RECEIPT);
    const runState = structuredClone(REGISTERED_RUN);
    const original = {
      project: mapping.project.projectAuthorityDigest,
      head: head.authorityHead.headDigest,
      receipt: receipt.previousGlobal.semanticReceiptDigest,
      runEvent: runState.registrationEventDigest,
    };
    const sequence = scripted([
      { name: 'map', reply: mapping },
      { name: 'exact', reply: { kind: 'absent' }, before: () => {
        (mapping.project as any).projectAuthorityDigest = sha256Text('mutated-mapping');
      } },
      { name: 'head', reply: head },
      { name: 'receipt', reply: receipt, before: () => {
        (head.authorityHead as any).headDigest = sha256Text('mutated-head');
      } },
      { name: 'run', reply: runState, before: () => {
        (receipt.previousGlobal as any).semanticReceiptDigest = sha256Text('mutated-receipt');
      } },
    ]);

    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, sequence.ports,
    );

    expect(decision.decisionKind).toBe('append-changed-replay-candidate');
    if (decision.decisionKind !== 'append-changed-replay-candidate') return;
    expect(decision.candidate.project).toMatchObject({ projectAuthorityDigest: original.project });
    expect(decision.candidate.authorityHead).toMatchObject({ headDigest: original.head });
    expect(decision.candidate.previousGlobal).toMatchObject({
      semanticReceiptDigest: original.receipt,
    });
    expect(decision.candidate.expectedRunState).not.toBe(runState);
    expect(Object.isFrozen(runState)).toBe(false);
    (runState as any).registrationEventDigest = sha256Text('mutated-after-return');
    expect(decision.candidate.expectedRunState).toMatchObject({
      registrationEventDigest: original.runEvent,
    });

    const stored = structuredClone(exactStoredResult(201));
    const exact = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, scripted([
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: stored },
    ]).ports);
    stored.row.serializedResponse = 'mutated';
    expect(exact).toMatchObject({ response: { body: expect.not.stringMatching(/^mutated$/) } });
  });

  it.each([
    ['configuration epoch', { configurationEpoch: '1' }],
    ['configuration digest', { configurationDigest: sha256Text('new-configuration') }],
    ['head digest', { headDigest: sha256Text('new-head') }],
  ])('rejects a stale request when the active %s changed', async (_label, change) => {
    const head = {
      ...ACTIVE_HEAD,
      authorityHead: { ...ACTIVE_HEAD.authorityHead, ...change },
    };
    const run = scripted([
      { name: 'map', reply: MAPPED },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: head },
    ]);

    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, run.ports,
    );

    expect(outcome(decision)).toBe('registration-not-admitted-v2');
    expect(run.calls.map(({ name }) => name)).toEqual(['map', 'exact', 'head']);
  });

  it.each([
    ['sequence two with genesis', {
      ...ACTIVE_HEAD, expectedNextGlobalSequence: '2',
    }],
    ['sequence one with semantic predecessor', {
      ...ACTIVE_HEAD,
      requiredPredecessor: {
        kind: 'semantic-event', eventDigest: sha256Text('semantic-predecessor'),
      },
    }],
  ])('rejects inconsistent global-head state: %s', async (_label, head) => {
    const run = scripted([
      { name: 'map', reply: MAPPED },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: head },
    ]);

    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, run.ports,
    );

    expect(decision.decisionKind).toBe('indeterminate');
    expect(run.calls.map(({ name }) => name)).toEqual(['map', 'exact', 'head']);
  });

  it.each([
    ['absent run', { kind: 'absent' }, 'registration-authority-pending-v2'],
    ['distinct closed run', {
      kind: 'advanced-or-closed',
      projectAuthorityDigest: DIGEST.project,
      runId: 'capture_run_20260829',
      originalRegistrationRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
      registrationEventDigest: REGISTERED_RUN.registrationEventDigest,
      lastRunEventDigest: sha256Text('closed-run-last-event'),
      lastRunGlobalSequence: '3',
      currentControllerStateHeadDigest: REGISTERED_RUN.currentControllerStateHeadDigest,
      lastRunSequence: '2',
      firstChangedReplayRequestDigest: null,
    }, 'registration-authority-pending-v2'],
    ['missing original exact row', {
      ...REGISTERED_RUN, originalRegistrationRequestDigest: DIGEST.request,
    }, 'transaction-resolution-unknown-v2'],
    ['missing first-change exact row', {
      kind: 'advanced-or-closed',
      projectAuthorityDigest: DIGEST.project,
      runId: 'capture_run_20260829',
      originalRegistrationRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
      registrationEventDigest: REGISTERED_RUN.registrationEventDigest,
      lastRunEventDigest: sha256Text('first-changed-replay-event'),
      lastRunGlobalSequence: '2',
      currentControllerStateHeadDigest: REGISTERED_RUN.currentControllerStateHeadDigest,
      lastRunSequence: '1',
      firstChangedReplayRequestDigest: DIGEST.request,
    }, 'transaction-resolution-unknown-v2'],
    ['malformed run', { kind: 'absent', extra: false },
      'transaction-resolution-unknown-v2'],
  ] as const)(
    'uses pending only after one consistency-only run read: %s',
    async (_label, runState, expectedOutcome) => {
      const { head } = stateReads(runState);
      const run = scripted([
        { name: 'map', reply: MAPPED },
        { name: 'exact', reply: { kind: 'absent' } },
        { name: 'head', reply: head },
        { name: 'receipt', reply: { kind: 'pending' } },
        { name: 'run', reply: runState },
      ]);

      const decision = await decideSupervisorRegistrationV1(
        CANONICAL_REQUEST, PEER, run.ports,
      );

      expect(outcome(decision)).toBe(expectedOutcome);
      expect(decision).not.toHaveProperty('candidate');
      expect(run.calls.map(({ name }) => name)).toEqual([
        'map', 'exact', 'head', 'receipt', 'run',
      ]);
      expect(run.remaining()).toBe(0);
    },
  );

  it('maps a pending-receipt run-read failure to fixed 500 without retry', async () => {
    const run = scripted([
      { name: 'map', reply: MAPPED },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: ACTIVE_HEAD },
      { name: 'receipt', reply: { kind: 'pending' } },
      { name: 'run', error: new Error('run store unavailable') },
    ]);

    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, run.ports,
    );

    expect(outcome(decision)).toBe('transaction-resolution-unknown-v2');
    expect(run.calls.map(({ name }) => name)).toEqual([
      'map', 'exact', 'head', 'receipt', 'run',
    ]);
    expect(run.remaining()).toBe(0);
  });

  it('rejects a minimum global head whose predecessor is not the last run event', async () => {
    const wrongHead = {
      ...ACTIVE_SEMANTIC_HEAD,
      requiredPredecessor: {
        kind: 'semantic-event' as const,
        eventDigest: sha256Text('foreign-global-predecessor'),
      },
    };
    const run = scripted([
      { name: 'map', reply: MAPPED },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: wrongHead },
      { name: 'receipt', reply: {
        kind: 'ready',
        previousGlobal: {
          ...wrongHead.requiredPredecessor,
          semanticReceiptDigest: sha256Text('foreign-predecessor-receipt'),
        },
      } },
      { name: 'run', reply: REGISTERED_RUN },
    ]);

    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, run.ports,
    );

    expect(decision.decisionKind).toBe('indeterminate');
    expect(run.calls.map(({ name }) => name)).toEqual([
      'map', 'exact', 'head', 'receipt', 'run',
    ]);
  });

  it('permits an interleaved global predecessor strictly beyond the run minimum', async () => {
    const predecessor = {
      kind: 'semantic-event' as const,
      eventDigest: sha256Text('interleaved-other-run-event'),
    };
    const head = {
      ...ACTIVE_SEMANTIC_HEAD,
      expectedNextGlobalSequence: '3',
      requiredPredecessor: predecessor,
    };
    const run = scripted([
      { name: 'map', reply: MAPPED },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: head },
      { name: 'receipt', reply: {
        kind: 'ready',
        previousGlobal: {
          ...predecessor, semanticReceiptDigest: sha256Text('interleaved-receipt'),
        },
      } },
      { name: 'run', reply: REGISTERED_RUN },
    ]);

    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, run.ports,
    );

    expect(decision.decisionKind).toBe('append-changed-replay-candidate');
    expect((decision as any).candidate.expectedNextGlobalSequence).toBe('3');
  });

  it('rejects an authority head rolled back behind the last run global position', async () => {
    const predecessor = {
      kind: 'semantic-event' as const, eventDigest: sha256Text('rolled-back-head-event'),
    };
    const head = {
      ...ACTIVE_SEMANTIC_HEAD, expectedNextGlobalSequence: '50',
      requiredPredecessor: predecessor,
    };
    const run = scripted([
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: head },
      { name: 'receipt', reply: { kind: 'ready', previousGlobal: {
        ...predecessor, semanticReceiptDigest: sha256Text('rolled-back-head-receipt'),
      } } },
      { name: 'run', reply: { ...REGISTERED_RUN, lastRunGlobalSequence: '100' } },
    ]);
    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, run.ports);
    expect(decision.decisionKind).toBe('indeterminate');
  });

  it.each([
    ['terminal after sequence one', {
      kind: 'advanced-or-closed',
      projectAuthorityDigest: DIGEST.project,
      runId: 'capture_run_20260829',
      originalRegistrationRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
      registrationEventDigest: REGISTERED_RUN.registrationEventDigest,
      lastRunEventDigest: sha256Text('impossible-later-event'),
      lastRunGlobalSequence: '3',
      currentControllerStateHeadDigest: REGISTERED_RUN.currentControllerStateHeadDigest,
      lastRunSequence: '2',
      firstChangedReplayRequestDigest: sha256Text('first-changed-request'),
    }],
    ['same original and changed request', {
      kind: 'advanced-or-closed',
      projectAuthorityDigest: DIGEST.project,
      runId: 'capture_run_20260829',
      originalRegistrationRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
      registrationEventDigest: REGISTERED_RUN.registrationEventDigest,
      lastRunEventDigest: sha256Text('first-changed-replay-event'),
      lastRunGlobalSequence: '2',
      currentControllerStateHeadDigest: REGISTERED_RUN.currentControllerStateHeadDigest,
      lastRunSequence: '1',
      firstChangedReplayRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
    }],
  ])('rejects an impossible closed-run state: %s', async (_label, runState) => {
    await expectIndeterminateAt('run', runState);
  });
});

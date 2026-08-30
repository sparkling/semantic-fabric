// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import {
  fixedRegistrationTransportResponseV2,
  type AuthenticatedTransportPeerV1,
  type SupervisorRegistrationDecisionPortsV1,
} from '../src/index.js';
import {
  executeSupervisorRegistrationTransactionV1,
  recoverExactSupervisorRegistrationV1,
  type AuthenticatedExactRecoveryPeerRegistryV1,
  type AuthenticatedExactRecoveryPeerV1,
  type AuthenticatedRegistrationPeerRegistryV1,
  type SupervisorRegistrationRecoveryStoreV1,
  type SupervisorRegistrationRecoveryTransactionV1,
  type SupervisorRegistrationTransactionStoreV1,
  type SupervisorRegistrationTransactionV1,
} from '../src/registration-transaction-v1.js';
import {
  ACTIVE_HEAD, ACTIVE_SEMANTIC_HEAD, CANONICAL_REQUEST, PROJECT,
  READY_RECEIPT, READY_SEMANTIC_RECEIPT, REGISTERED_RUN,
  coherentlyMutatedRegistrationEnvelope, exactStoredResult, sha256Text,
} from './registration-fixtures.js';

const PEER = Symbol('adversarial-registration-peer') as AuthenticatedTransportPeerV1;
const RECOVERY_PEER = Symbol('adversarial-recovery-peer') as AuthenticatedExactRecoveryPeerV1;
const MAPPED = Object.freeze({ kind: 'mapped' as const, project: PROJECT });
const CHANGED_REPLAY_STORED = changedStored();
type Step = Readonly<{ name: string; reply?: unknown; error?: Error }>;

function scenario(input: Readonly<{
  steps: readonly Step[];
  commit?: () => Promise<unknown>;
  stage?: (candidate: Readonly<Record<string, unknown>>) => Promise<void>;
  rollback?: () => Promise<void>;
}> = { steps: [] }) {
  const queue = [...input.steps];
  const calls: string[] = [];
  const call = async <T>(name: string): Promise<T> => {
    calls.push(name);
    const step = queue.shift();
    if (step?.name !== name) throw new Error(`unexpected ${name}; expected ${step?.name}`);
    if (step.error !== undefined) throw step.error;
    return step.reply as T;
  };
  const ports: SupervisorRegistrationDecisionPortsV1 = {
    mapAuthenticatedPeer: () => call('map'),
    lookupExactCommittedResult: () => call('exact'),
    readActiveAuthorityHead: () => call('head'),
    readRequiredPredecessorReceipt: () => call('receipt'),
    readRunState: () => call('run'),
  };
  const transaction = {
    ports,
    stageCandidate: async (candidate: Readonly<Record<string, unknown>>) => {
      calls.push('stage'); await input.stage?.(candidate);
    },
    commit: async () => { calls.push('commit'); return input.commit?.() ?? 'committed'; },
    rollback: async () => { calls.push('rollback'); await input.rollback?.(); },
    quarantine: async () => { calls.push('quarantine'); },
  } as unknown as SupervisorRegistrationTransactionV1;
  const recoveryTransaction = {
    ports: {
      mapAuthenticatedRecoveryPeer: () => call('map'),
      lookupExactCommittedResult: () => call('exact'),
    },
    commit: transaction.commit,
    rollback: transaction.rollback,
    quarantine: transaction.quarantine,
  } as SupervisorRegistrationRecoveryTransactionV1;
  const registrationCheckout = {
    open: vi.fn(async () => { calls.push('begin:registration'); return transaction; }),
    discardMalformed: vi.fn(async () => { calls.push('discard:registration'); }),
  };
  const recoveryCheckout = {
    open: vi.fn(async () => { calls.push('begin:exact-recovery'); return recoveryTransaction; }),
    discardMalformed: vi.fn(async () => { calls.push('discard:exact-recovery'); }),
  };
  const store: SupervisorRegistrationTransactionStoreV1 = {
    checkoutRegistration: vi.fn(async () => registrationCheckout),
  };
  const recoveryStore: SupervisorRegistrationRecoveryStoreV1 = {
    checkoutExactRecovery: vi.fn(async () => recoveryCheckout),
  };
  const registry: AuthenticatedRegistrationPeerRegistryV1 = {
    consumeRegistration: vi.fn((peer) => peer === PEER),
  };
  const recoveryRegistry: AuthenticatedExactRecoveryPeerRegistryV1 = {
    consumeExactRecovery: vi.fn((peer) => peer === RECOVERY_PEER),
  };
  return {
    calls, registry, recoveryRegistry, store, recoveryStore,
    transaction, recoveryTransaction, registrationCheckout, recoveryCheckout,
    remaining: () => queue.length,
  };
}

function registrationCandidateSteps(
  head: unknown = ACTIVE_HEAD,
  receipt: unknown = READY_RECEIPT,
): Step[] {
  return [
    { name: 'map', reply: MAPPED }, { name: 'exact', reply: { kind: 'absent' } },
    { name: 'head', reply: head }, { name: 'receipt', reply: receipt },
    { name: 'run', reply: { kind: 'absent' } },
    { name: 'map', reply: MAPPED }, { name: 'exact', reply: exactStoredResult(201) },
  ];
}

function changedStored(
  mutate: (event: Record<string, any>) => void = () => undefined,
  rowOverrides: Record<string, unknown> = {},
) {
  return exactStoredResult(409, {
    serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(409, (event) => {
      event.previousGlobal = READY_SEMANTIC_RECEIPT.previousGlobal;
      mutate(event);
    }),
  }, rowOverrides);
}

function changedReplaySteps(staged: unknown = CHANGED_REPLAY_STORED): Step[] {
  return [
    { name: 'map', reply: MAPPED }, { name: 'exact', reply: { kind: 'absent' } },
    { name: 'head', reply: ACTIVE_SEMANTIC_HEAD },
    { name: 'receipt', reply: READY_SEMANTIC_RECEIPT },
    { name: 'run', reply: REGISTERED_RUN },
    { name: 'map', reply: MAPPED },
    { name: 'exact', reply: staged },
  ];
}

function expectDeepFrozen(value: unknown, seen = new Set<object>()): void {
  if (value === null || typeof value !== 'object' || seen.has(value)) return;
  seen.add(value);
  expect(Object.isFrozen(value)).toBe(true);
  for (const child of Object.values(value)) expectDeepFrozen(child, seen);
}

describe('registration transaction adversarial closure V1', () => {
  it('commits a changed-replay candidate only against its bound staged 409', async () => {
    const run = scenario({ steps: changedReplaySteps() });

    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );

    expect(result.response).toEqual({
      status: 409,
      contentType: 'application/json; charset=utf-8',
      body: CHANGED_REPLAY_STORED.row.serializedResponse,
    });
    expect(run.calls.at(-1)).toBe('commit');
    expect(run.calls).not.toContain('rollback');
    expect(run.remaining()).toBe(0);
  });

  it.each([
    ['changed replay with staged 201', changedReplaySteps(exactStoredResult(201))],
    ['registration with stale staged sequence', registrationCandidateSteps(
      ACTIVE_SEMANTIC_HEAD, READY_SEMANTIC_RECEIPT,
    )],
  ] as const)('rolls back %s instead of committing a self-consistent wrong result',
    async (_label, steps) => {
      const run = scenario({ steps });
      const result = await executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, PEER, run.registry, run.store,
      );
      expect(result.response.status).toBe(500);
      expect(run.calls.at(-1)).toBe('rollback');
      expect(run.calls).not.toContain('commit');
      expect(run.remaining()).toBe(0);
    });

  it.each([
    ['registration', registrationCandidateSteps(), {
      candidateKind: 'claim-registered-v2', expectedNextGlobalSequence: '1',
      expectedRunState: { kind: 'absent' }, runSequence: '0',
      previousRun: { kind: 'run-genesis', eventDigest: null },
    }],
    ['changed replay', changedReplaySteps(), {
      candidateKind: 'capture-run-terminal-v2', expectedNextGlobalSequence: '2',
      expectedRunState: REGISTERED_RUN, runSequence: '1',
      previousRun: { kind: 'run-event', eventDigest: REGISTERED_RUN.registrationEventDigest },
      priorControllerStateHeadDigest: REGISTERED_RUN.currentControllerStateHeadDigest,
      body: { outcomeCode: 'registration-changed-replay-v2' },
    }],
  ] as const)('stages the exact deeply frozen %s candidate', async (_label, steps, expected) => {
    let staged: Readonly<Record<string, unknown>> | undefined;
    const run = scenario({ steps, stage: async (candidate) => { staged = candidate; } });
    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );
    expect(result.response.status).toBe(expected.candidateKind === 'claim-registered-v2' ? 201 : 409);
    expect(Object.keys(staged ?? {}).sort()).toEqual([
      'authorityHead', 'body', 'candidateKind', 'expectedNextGlobalSequence',
      'expectedRunState', 'previousGlobal', 'previousRun',
      'priorControllerStateHeadDigest', 'project', 'request', 'resourceTransition',
      'runSequence', 'transactionScope',
    ]);
    expect(staged).toMatchObject(expected);
    expectDeepFrozen(staged);
  });

  it.each([
    ['next global sequence', changedStored((event) => { event.globalSequence = '3'; })],
    ['predecessor receipt', changedStored((event) => {
      event.previousGlobal = { ...event.previousGlobal,
        semanticReceiptDigest: sha256Text('foreign-predecessor-receipt') };
    })],
    ['prior controller state', (() => {
      const digest = sha256Text('foreign-prior-controller-state');
      return changedStored((event) => { event.priorControllerStateHeadDigest = digest; }, {
        changedReplayPriorControllerStateHeadDigest: digest,
      });
    })()],
  ] as const)('rejects a staged row differing only in %s', async (_label, stored) => {
    const run = scenario({ steps: changedReplaySteps(stored) });
    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );
    expect(result.response.status).toBe(500);
    expect(run.calls.at(-1)).toBe('rollback');
    expect(run.calls).not.toContain('commit');
  });

  it('binds changed-replay original global provenance to the staged run snapshot', async () => {
    const predecessor = sha256Text('interleaved-global-event');
    const head = { ...ACTIVE_SEMANTIC_HEAD, expectedNextGlobalSequence: '4',
      requiredPredecessor: { kind: 'semantic-event' as const, eventDigest: predecessor } };
    const receipt = { kind: 'ready' as const, previousGlobal: {
      kind: 'semantic-event' as const, eventDigest: predecessor,
      semanticReceiptDigest: sha256Text('interleaved-global-receipt'),
    } };
    const stored = changedStored((event) => {
      event.globalSequence = '4'; event.previousGlobal = receipt.previousGlobal;
    }, { originalRegistrationGlobalSequence: '2' });
    const steps = changedReplaySteps(stored);
    steps[2] = { name: 'head', reply: head };
    steps[3] = { name: 'receipt', reply: receipt };
    const run = scenario({ steps });
    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );
    expect(result.response.status).toBe(500);
    expect(run.calls.at(-1)).toBe('rollback');
  });

  it.each([
    ['registration', 201, '4', (head: unknown, receipt: unknown) => {
      const stored = exactStoredResult(201, {
        serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
          event.globalSequence = '4'; event.previousGlobal = (receipt as any).previousGlobal;
        }),
      }, { originalRegistrationGlobalSequence: '4' });
      const steps = registrationCandidateSteps(head, receipt);
      steps[6] = { name: 'exact', reply: stored };
      return steps;
    }],
    ['changed replay', 409, '1', (head: unknown, receipt: unknown) => {
      const stored = changedStored((event) => {
        event.globalSequence = '4'; event.previousGlobal = (receipt as any).previousGlobal;
      });
      const steps = changedReplaySteps(stored);
      steps[2] = { name: 'head', reply: head };
      steps[3] = { name: 'receipt', reply: receipt };
      return steps;
    }],
  ] as const)('commits an interleaved %s at global 4 with original global %s',
    async (_label, status, _originalGlobal, makeSteps) => {
      const predecessor = sha256Text('interleaved-valid-global-event');
      const head = { ...ACTIVE_SEMANTIC_HEAD, expectedNextGlobalSequence: '4',
        requiredPredecessor: { kind: 'semantic-event' as const, eventDigest: predecessor } };
      const receipt = { kind: 'ready' as const, previousGlobal: {
        kind: 'semantic-event' as const, eventDigest: predecessor,
        semanticReceiptDigest: sha256Text('interleaved-valid-global-receipt'),
      } };
      const run = scenario({ steps: makeSteps(head, receipt) });
      const result = await executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, PEER, run.registry, run.store,
      );
      expect(result.response.status).toBe(status);
      expect(run.calls.at(-1)).toBe('commit');
      expect(run.calls).not.toContain('rollback');
    });

  it('returns an ordinary exact hit without head reads or staging', async () => {
    const stored = exactStoredResult(201);
    const run = scenario({ steps: [
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: stored },
    ] });
    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );
    expect(result.response).toEqual({
      status: 201, contentType: 'application/json; charset=utf-8',
      body: stored.row.serializedResponse,
    });
    expect(run.calls).toEqual(['begin:registration', 'map', 'exact', 'commit']);
  });

  it('preclassifies malformed bytes without consuming a peer or opening either store',
    async () => {
      const registration = scenario();
      const recovery = scenario();
      const [writeResult, recoveryResult] = await Promise.all([
        executeSupervisorRegistrationTransactionV1(
          '{', PEER, registration.registry, registration.store,
        ),
        recoverExactSupervisorRegistrationV1(
          '{', RECOVERY_PEER, recovery.recoveryRegistry, recovery.recoveryStore,
        ),
      ]);
      expect([writeResult.response, recoveryResult.response]).toEqual([
        fixedRegistrationTransportResponseV2('registration-not-admitted-v2'),
        fixedRegistrationTransportResponseV2('registration-not-admitted-v2'),
      ]);
      expect(registration.calls).toEqual([]);
      expect(recovery.calls).toEqual([]);
      expect(registration.registry.consumeRegistration).not.toHaveBeenCalled();
      expect(recovery.recoveryRegistry.consumeExactRecovery).not.toHaveBeenCalled();
    });

  it.each([
    ['throw', async () => { throw new Error('commit transport lost'); }],
    ['object', async () => ({ committed: true })],
  ] as const)('quarantines a %s registration commit outcome without releasing bytes',
    async (_label, commit) => {
      const run = scenario({
        steps: [{ name: 'map', reply: MAPPED }, {
          name: 'exact', reply: exactStoredResult(201),
        }],
        commit,
      });
      const result = await executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, PEER, run.registry, run.store,
      );
      expect(result.response.status).toBe(500);
      expect(result.response.body).not.toContain('supervisor-registration-result-v2');
      expect(run.calls.slice(-2)).toEqual(['commit', 'quarantine']);
      expect(run.calls).not.toContain('rollback');
    });

  it.each([
    ['throw', async (): Promise<never> => { throw new Error('commit transport lost'); }],
    ['boolean', async (): Promise<boolean> => true],
  ] as const)('quarantines a %s recovery commit outcome without releasing bytes',
    async (_label, commit) => {
      const run = scenario({
        steps: [{ name: 'map', reply: MAPPED }, {
          name: 'exact', reply: exactStoredResult(201),
        }],
        commit,
      });
      const result = await recoverExactSupervisorRegistrationV1(
        CANONICAL_REQUEST, RECOVERY_PEER, run.recoveryRegistry, run.recoveryStore,
      );
      expect(result.response.status).toBe(500);
      expect(result.response.body).not.toContain('supervisor-registration-result-v2');
      expect(run.calls.slice(-2)).toEqual(['commit', 'quarantine']);
      expect(run.calls).not.toContain('rollback');
    });

  it('invokes capability methods without the raw record as receiver', async () => {
    const run = scenario({ steps: registrationCandidateSteps() });
    let receiver: unknown = 'not-called';
    run.transaction.stageCandidate = async function (this: unknown) {
      receiver = this;
      run.calls.push('stage');
    };
    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );
    expect(result.response.status).toBe(201);
    expect(receiver).toBeUndefined();
    expect(run.calls.filter((call) => call === 'commit')).toHaveLength(1);
  });

  it('rejects Proxy-wrapped capability methods without invoking their apply traps', async () => {
    for (const target of ['registry', 'store', 'transaction', 'ports'] as const) {
      const run = scenario();
      let traps = 0;
      const method = new Proxy(async () => undefined, {
        apply: () => { traps += 1; return undefined; },
      });
      if (target === 'registry') run.registry.consumeRegistration = method as any;
      if (target === 'store') run.store.checkoutRegistration = method as any;
      if (target === 'transaction') run.transaction.commit = method as any;
      if (target === 'ports') run.transaction.ports.mapAuthenticatedPeer = method as any;

      const result = await executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, PEER, run.registry, run.store,
      );
      expect(result.response.status, target).toBe(500);
      expect(traps, target).toBe(0);
      if (target === 'transaction' || target === 'ports') {
        expect(run.calls).toContain('discard:registration');
      }
    }
  });

  it('rejects an accessor capability method without invoking its getter', async () => {
    const run = scenario();
    let getters = 0;
    Object.defineProperty(run.registry, 'consumeRegistration', {
      enumerable: true,
      get: () => { getters += 1; return () => true; },
    });
    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );
    expect(result.response.status).toBe(500);
    expect(getters).toBe(0);
    expect(run.calls).toEqual([]);
  });

  it('rejects an extra symbol-keyed capability before invoking any method', async () => {
    const run = scenario();
    Object.defineProperty(run.registry, Symbol('hidden-capability'), {
      enumerable: true, value: async () => undefined,
    });
    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );
    expect(result.response.status).toBe(500);
    expect(run.calls).toEqual([]);
  });

  it('keeps registration and exact recovery capability roots disjoint', async () => {
    const write = scenario();
    const recovery = scenario();
    expect(Object.keys(write.registry)).toEqual(['consumeRegistration']);
    expect(Object.keys(write.store).sort()).toEqual([
      'checkoutRegistration',
    ]);
    expect(Object.keys(recovery.recoveryRegistry)).toEqual(['consumeExactRecovery']);
    expect(Object.keys(recovery.recoveryStore).sort()).toEqual([
      'checkoutExactRecovery',
    ]);
    expect(Object.keys(write.registrationCheckout).sort()).toEqual([
      'discardMalformed', 'open',
    ]);
    expect(Object.keys(recovery.recoveryCheckout).sort()).toEqual([
      'discardMalformed', 'open',
    ]);
    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST,
      RECOVERY_PEER as unknown as AuthenticatedTransportPeerV1,
      recovery.recoveryRegistry as unknown as AuthenticatedRegistrationPeerRegistryV1,
      recovery.recoveryStore as unknown as SupervisorRegistrationTransactionStoreV1,
    );
    expect(result.response.status).toBe(500);
    expect(recovery.calls).toEqual([]);
  });

  it('discards malformed begins and quarantines rollback failure', async () => {
    const malformed = scenario();
    malformed.registrationCheckout.open = vi.fn(async () => {
      malformed.calls.push('begin:registration');
      return {} as SupervisorRegistrationTransactionV1;
    });
    const malformedResult = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, malformed.registry, malformed.store,
    );
    expect(malformedResult.response.status).toBe(500);
    expect(malformed.calls).toEqual(['begin:registration', 'discard:registration']);

    const rollback = scenario({
      steps: registrationCandidateSteps().slice(0, 5),
      stage: async () => { throw new Error('stage failed'); },
      rollback: async () => { throw new Error('rollback failed'); },
    });
    const rollbackResult = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, rollback.registry, rollback.store,
    );
    expect(rollbackResult.response.status).toBe(500);
    expect(rollback.calls.slice(-2)).toEqual(['rollback', 'quarantine']);
    expect(rollback.calls).not.toContain('commit');
  });

  it('rejects recovery ports widened with head/run capabilities before any read', async () => {
    const run = scenario();
    (run.recoveryTransaction as unknown as { ports: unknown }).ports = {
      ...run.recoveryTransaction.ports,
      readActiveAuthorityHead: async () => ({ kind: 'indeterminate' }),
      readRequiredPredecessorReceipt: async () => ({ kind: 'indeterminate' }),
      readRunState: async () => ({ kind: 'indeterminate' }),
    } as any;
    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY_PEER, run.recoveryRegistry, run.recoveryStore,
    );
    expect(result.response.status).toBe(500);
    expect(run.calls).toEqual(['begin:exact-recovery', 'discard:exact-recovery']);
  });
});

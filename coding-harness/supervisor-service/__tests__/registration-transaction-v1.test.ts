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
  type SupervisorRegistrationCommitResolutionV1,
  type SupervisorRegistrationRecoveryStoreV1,
  type SupervisorRegistrationRecoveryTransactionV1,
  type SupervisorRegistrationTransactionStoreV1,
  type SupervisorRegistrationTransactionV1,
} from '../src/registration-transaction-v1.js';
import {
  ACTIVE_HEAD, ACTIVE_SEMANTIC_HEAD,
  CANONICAL_REQUEST,
  PROJECT,
  READY_RECEIPT, READY_SEMANTIC_RECEIPT, REGISTERED_RUN,
  exactStoredResult,
} from './registration-fixtures.js';

const PEER = Symbol('transaction-test-peer') as AuthenticatedTransportPeerV1;
const RECOVERY_PEER = Symbol('exact-recovery-test-peer') as AuthenticatedExactRecoveryPeerV1;
const MAPPED = Object.freeze({ kind: 'mapped' as const, project: PROJECT });
type Step = Readonly<{ name: string; reply?: unknown; error?: Error }>;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function fixture(input: Readonly<{
  steps: readonly Step[];
  commit?: () => Promise<SupervisorRegistrationCommitResolutionV1>;
  stage?: (candidate: Readonly<Record<string, unknown>>) => Promise<void>;
  admitted?: boolean;
}>) {
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
  const transaction: SupervisorRegistrationTransactionV1 = {
    ports,
    stageCandidate: async (candidate) => {
      calls.push('stage');
      return input.stage?.(candidate);
    },
    commit: async () => {
      calls.push('commit');
      return input.commit?.() ?? 'committed';
    },
    rollback: async () => { calls.push('rollback'); },
    quarantine: async () => { calls.push('quarantine'); },
  };
  const recoveryTransaction: SupervisorRegistrationRecoveryTransactionV1 = {
    ports: {
      mapAuthenticatedRecoveryPeer: () => call('map'),
      lookupExactCommittedResult: ports.lookupExactCommittedResult,
    },
    commit: transaction.commit,
    rollback: transaction.rollback,
    quarantine: transaction.quarantine,
  };
  const registrationCheckout = {
    open: vi.fn(async () => {
      calls.push('begin:registration'); return transaction;
    }),
    discardMalformed: vi.fn(async () => {
      calls.push('discard:registration');
    }),
  };
  const recoveryCheckout = {
    open: vi.fn(async () => {
      calls.push('begin:exact-recovery'); return recoveryTransaction;
    }),
    discardMalformed: vi.fn(async () => {
      calls.push('discard:exact-recovery');
    }),
  };
  const store: SupervisorRegistrationTransactionStoreV1 = {
    checkoutRegistration: vi.fn(async () => registrationCheckout),
  };
  const recoveryStore: SupervisorRegistrationRecoveryStoreV1 = {
    checkoutExactRecovery: vi.fn(async () => recoveryCheckout),
  };
  let consumed = false;
  const registry: AuthenticatedRegistrationPeerRegistryV1 = {
    consumeRegistration: vi.fn((peer) => {
      if (consumed || input.admitted === false || peer !== PEER) return false;
      consumed = true;
      return true;
    }),
  };
  let recoveryConsumed = false;
  const recoveryRegistry: AuthenticatedExactRecoveryPeerRegistryV1 = {
    consumeExactRecovery: vi.fn((peer) => {
      if (recoveryConsumed || input.admitted === false || peer !== RECOVERY_PEER) return false;
      recoveryConsumed = true;
      return true;
    }),
  };
  return {
    calls, registry, recoveryRegistry, store, recoveryStore,
    transaction, recoveryTransaction, registrationCheckout, recoveryCheckout,
    remaining: () => queue.length,
  };
}

function candidateSteps(secondExact: unknown = exactStoredResult(201)): Step[] {
  return [
    { name: 'map', reply: MAPPED },
    { name: 'exact', reply: { kind: 'absent' } },
    { name: 'head', reply: ACTIVE_HEAD },
    { name: 'receipt', reply: READY_RECEIPT },
    { name: 'run', reply: { kind: 'absent' } },
    { name: 'map', reply: MAPPED },
    { name: 'exact', reply: secondExact },
  ];
}

function responseBody(value: Awaited<ReturnType<
  typeof executeSupervisorRegistrationTransactionV1
>>): string {
  return value.response.body;
}

describe('nonoperational supervisor registration transaction coordinator V1', () => {
  it('returns fixed 500 without retrying or exposing staged bytes when commit is unknown',
    async () => {
      const run = fixture({
        steps: candidateSteps(),
        commit: async () => 'unknown',
      });

      const result = await executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, PEER, run.registry, run.store,
      );

      expect(result).toEqual({
        authority: 'none',
        mutationAuthorized: false,
        response: fixedRegistrationTransportResponseV2(
          'transaction-resolution-unknown-v2',
        ),
      });
      expect(run.calls).toEqual([
        'begin:registration', 'map', 'exact', 'head', 'receipt', 'run',
        'stage', 'map', 'exact', 'commit', 'quarantine',
      ]);
      expect(run.store.checkoutRegistration).toHaveBeenCalledTimes(1);
      expect(responseBody(result)).not.toContain('supervisor-registration-result-v2');
      expect(run.remaining()).toBe(0);
    });

  it('does not settle staged response bytes until commit is acknowledged', async () => {
    const acknowledgement = deferred<SupervisorRegistrationCommitResolutionV1>();
    const commitEntered = deferred<void>();
    const run = fixture({
      steps: candidateSteps(),
      commit: () => {
        commitEntered.resolve(undefined);
        return acknowledgement.promise;
      },
    });
    let settled = false;

    const pending = executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    ).finally(() => { settled = true; });
    await commitEntered.promise;

    expect(run.calls).toContain('commit');
    expect(settled).toBe(false);
    acknowledgement.resolve('committed');
    const result = await pending;

    expect(result.response).toEqual({
      status: 201,
      contentType: 'application/json; charset=utf-8',
      body: exactStoredResult(201).row.serializedResponse,
    });
    expect(run.calls.at(-1)).toBe('commit');
    expect(run.remaining()).toBe(0);
    expect(Object.isFrozen(result)).toBe(true);
    expect(Object.isFrozen(result.response)).toBe(true);
    expect(result).not.toHaveProperty('candidate');
  });

  it.each([
    ['not admitted', [
      { name: 'map', reply: { kind: 'not-admitted' } },
    ], 'registration-not-admitted-v2'],
    ['receipt pending', [
      { name: 'map', reply: MAPPED },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: ACTIVE_HEAD },
      { name: 'receipt', reply: { kind: 'pending' } },
      { name: 'run', reply: { kind: 'absent' } },
    ], 'registration-authority-pending-v2'],
  ] as const)(
    'commits the read transaction for %s without staging',
    async (_label, steps, outcome) => {
      const run = fixture({ steps });

      const result = await executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, PEER, run.registry, run.store,
      );

      expect(result.response).toEqual(fixedRegistrationTransportResponseV2(outcome));
      expect(run.calls.at(-1)).toBe('commit');
      expect(run.calls).not.toContain('stage');
      expect(run.calls).not.toContain('rollback');
      expect(run.remaining()).toBe(0);
    },
  );

  it.each([
    ['stage failure', exactStoredResult(201), true],
    ['missing staged exact row', { kind: 'absent' }, false],
    ['corrupt staged exact row', exactStoredResult(201, {}, {
      serializedResponseSha256: '0'.repeat(64),
    }), false],
  ] as const)('rolls back %s and returns only fixed 500', async (_label, exact, failStage) => {
    const run = fixture({
      steps: failStage ? candidateSteps().slice(0, 5) : candidateSteps(exact),
      ...(failStage ? {
        stage: async () => { throw new Error('signer failed'); },
      } : {}),
    });

    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );

    expect(result.response).toEqual(fixedRegistrationTransportResponseV2(
      'transaction-resolution-unknown-v2',
    ));
    expect(run.calls.at(-1)).toBe('rollback');
    expect(run.calls).not.toContain('commit');
    expect(responseBody(result)).not.toContain('supervisor-registration-result-v2');
    expect(run.remaining()).toBe(0);
  });

  it('rejects an unregistered Symbol before opening a transaction', async () => {
    const run = fixture({ steps: [], admitted: false });

    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );

    expect(result.response).toEqual(fixedRegistrationTransportResponseV2(
      'registration-not-admitted-v2',
    ));
    expect(run.store.checkoutRegistration).not.toHaveBeenCalled();
    expect(run.recoveryStore.checkoutExactRecovery).not.toHaveBeenCalled();
    expect(run.calls).toEqual([]);
  });

  it('atomically consumes a registered peer before a concurrent replay can begin', async () => {
    const run = fixture({ steps: [{ name: 'map', reply: { kind: 'not-admitted' } }] });

    const [first, replay] = await Promise.all([
      executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, PEER, run.registry, run.store,
      ),
      executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, PEER, run.registry, run.store,
      ),
    ]);

    expect(first.response).toEqual(fixedRegistrationTransportResponseV2(
      'registration-not-admitted-v2',
    ));
    expect(replay.response).toEqual(first.response);
    expect(run.store.checkoutRegistration).toHaveBeenCalledTimes(1);
    expect(run.registry.consumeRegistration).toHaveBeenCalledTimes(2);
    expect(run.calls).toEqual(['begin:registration', 'map', 'commit']);
  });

  it('rejects a nested Proxy adapter result without invoking any Proxy trap', async () => {
    let traps = 0;
    const target = { ...PROJECT };
    const project = new Proxy(target, {
      get: () => { traps += 1; return undefined; },
      getPrototypeOf: () => { traps += 1; return Object.prototype; },
      ownKeys: () => { traps += 1; return Reflect.ownKeys(target); },
      getOwnPropertyDescriptor: (_value, key) => {
        traps += 1;
        return Reflect.getOwnPropertyDescriptor(target, key);
      },
    });
    const reply = { kind: 'mapped', project };
    const run = fixture({ steps: [{ name: 'map', reply }] });

    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );

    expect(result.response).toEqual(fixedRegistrationTransportResponseV2(
      'transaction-resolution-unknown-v2',
    ));
    expect(traps).toBe(0);
    expect(run.calls).toEqual(['begin:registration', 'map', 'commit']);
  });

  it('rejects an accessor adapter result without invoking its getter', async () => {
    let getterCalls = 0;
    const reply: Record<string, unknown> = { kind: 'mapped' };
    Object.defineProperty(reply, 'project', {
      enumerable: true,
      get: () => { getterCalls += 1; return PROJECT; },
    });
    const run = fixture({ steps: [{ name: 'map', reply }] });

    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );

    expect(result.response.status).toBe(500);
    expect(getterCalls).toBe(0);
    expect(run.calls).toEqual(['begin:registration', 'map', 'commit']);
  });

  it('maps begin and capability-registry failures to fixed 500', async () => {
    const run = fixture({ steps: [] });
    run.store.checkoutRegistration = vi.fn(async () => { throw new Error('pool unavailable'); });
    const failedBegin = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, run.registry, run.store,
    );
    const brokenRegistry: AuthenticatedRegistrationPeerRegistryV1 = {
      consumeRegistration: () => { throw new Error('registry unavailable'); },
    };
    const failedRegistry = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, PEER, brokenRegistry, run.store,
    );

    expect([failedBegin, failedRegistry].map(({ response }) => response)).toEqual([
      fixedRegistrationTransportResponseV2('transaction-resolution-unknown-v2'),
      fixedRegistrationTransportResponseV2('transaction-resolution-unknown-v2'),
    ]);
  });
});

describe('exact-only supervisor registration recovery V1', () => {
  it.each([201, 409] as const)(
    'returns exact stored %s bytes after a confirmed read-only commit', async (status) => {
      const stored = exactStoredResult(status);
      const run = fixture({ steps: [
        { name: 'map', reply: MAPPED }, { name: 'exact', reply: stored },
      ] });

      const result = await recoverExactSupervisorRegistrationV1(
        CANONICAL_REQUEST, RECOVERY_PEER, run.recoveryRegistry, run.recoveryStore,
      );

      expect(result.response).toEqual({
        status,
        contentType: 'application/json; charset=utf-8',
        body: stored.row.serializedResponse,
      });
      expect(run.calls).toEqual(['begin:exact-recovery', 'map', 'exact', 'commit']);
      expect(run.calls).not.toContain('stage');
      expect(run.remaining()).toBe(0);
    },
  );

  it('keeps an exact miss nonmutating and returns fixed 500', async () => {
    const run = fixture({ steps: [
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: { kind: 'absent' } },
    ] });

    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY_PEER, run.recoveryRegistry, run.recoveryStore,
    );

    expect(result.response).toEqual(fixedRegistrationTransportResponseV2(
      'transaction-resolution-unknown-v2',
    ));
    expect(run.calls).toEqual(['begin:exact-recovery', 'map', 'exact', 'commit']);
    expect(run.calls).not.toContain('stage');
    expect(run.calls).not.toContain('head');
    expect(run.remaining()).toBe(0);
  });

  it('has no write or head/run capability and rejects a widened recovery transaction',
    async () => {
      const run = fixture({ steps: [] });
      expect(Object.keys(run.recoveryTransaction).sort()).toEqual([
        'commit', 'ports', 'quarantine', 'rollback',
      ]);
      expect(Object.keys(run.recoveryTransaction.ports).sort()).toEqual([
        'lookupExactCommittedResult', 'mapAuthenticatedRecoveryPeer',
      ]);
      run.recoveryCheckout.open = vi.fn(async () => {
        run.calls.push('begin:exact-recovery');
        const widened = ({ ...run.recoveryTransaction,
          stageCandidate: async () => undefined } as unknown as SupervisorRegistrationRecoveryTransactionV1);
        return widened;
      });

      const result = await recoverExactSupervisorRegistrationV1(
        CANONICAL_REQUEST, RECOVERY_PEER, run.recoveryRegistry, run.recoveryStore,
      );

      expect(result.response.status).toBe(500);
      expect(run.calls).toEqual(['begin:exact-recovery', 'discard:exact-recovery']);
    });

  it.each([
    ['corrupt', exactStoredResult(201, {}, { serializedResponseSha256: '0'.repeat(64) })],
    ['failed', new Error('exact lookup failed')],
  ] as const)('returns fixed 500 for a %s exact read without broader reads', async (_label, read) => {
    const run = fixture({ steps: [
      { name: 'map', reply: MAPPED },
      read instanceof Error
        ? { name: 'exact', error: read }
        : { name: 'exact', reply: read },
    ] });

    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY_PEER, run.recoveryRegistry, run.recoveryStore,
    );

    expect(result.response).toEqual(fixedRegistrationTransportResponseV2(
      'transaction-resolution-unknown-v2',
    ));
    expect(run.calls).toEqual(['begin:exact-recovery', 'map', 'exact', 'commit']);
    expect(run.calls).not.toContain('head');
    expect(run.calls).not.toContain('stage');
  });

  it('does not release exact bytes when the recovery commit is unknown', async () => {
    const run = fixture({
      steps: [{ name: 'map', reply: MAPPED }, {
        name: 'exact', reply: exactStoredResult(201),
      }],
      commit: async () => 'unknown',
    });

    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY_PEER, run.recoveryRegistry, run.recoveryStore,
    );

    expect(result.response).toEqual(fixedRegistrationTransportResponseV2(
      'transaction-resolution-unknown-v2',
    ));
    expect(responseBody(result)).not.toContain('supervisor-registration-result-v2');
    expect(run.calls.slice(-2)).toEqual(['commit', 'quarantine']);
    expect(run.calls).not.toContain('rollback');
  });
});

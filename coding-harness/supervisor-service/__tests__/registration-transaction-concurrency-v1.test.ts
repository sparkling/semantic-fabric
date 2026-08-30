// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import type {
  AuthenticatedTransportPeerV1, ExactCommittedResultReadV1,
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
import { CANONICAL_REQUEST, PROJECT, exactStoredResult } from './registration-fixtures.js';

const WRITE_A = Symbol('write-a') as AuthenticatedTransportPeerV1;
const WRITE_B = Symbol('write-b') as AuthenticatedTransportPeerV1;
const RECOVERY = Symbol('recovery') as AuthenticatedExactRecoveryPeerV1;

describe('registration transaction concurrent capability closure V1', () => {
  it('uses the exact per-begin cleanup closure for concurrent malformed checkouts', async () => {
    const admitted = new Set<symbol>();
    const registry: AuthenticatedRegistrationPeerRegistryV1 = {
      consumeRegistration: (peer) => {
        if (admitted.has(peer)) return false;
        admitted.add(peer); return true;
      },
    };
    const discarded: number[] = [];
    let opened = 0;
    const store: SupervisorRegistrationTransactionStoreV1 = {
      checkoutRegistration: vi.fn(async () => {
        const checkout = ++opened;
        return {
          open: async () => ({} as SupervisorRegistrationTransactionV1),
          discardMalformed: async () => { discarded.push(checkout); },
        };
      }),
    };

    const results = await Promise.all([WRITE_A, WRITE_B].map((peer) =>
      executeSupervisorRegistrationTransactionV1(CANONICAL_REQUEST, peer, registry, store)));

    expect(results.map((result) => result.response.status)).toEqual([500, 500]);
    expect(store.checkoutRegistration).toHaveBeenCalledTimes(2);
    expect(discarded.sort()).toEqual([1, 2]);
  });

  it('atomically consumes an exact-recovery peer before a concurrent replay begins', async () => {
    const run = recoveryFixture();

    const results = await Promise.all([
      recoverExactSupervisorRegistrationV1(
        CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
      ),
      recoverExactSupervisorRegistrationV1(
        CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
      ),
    ]);

    expect(results.map((result) => result.response.status).sort()).toEqual([201, 403]);
    expect(run.registry.consumeExactRecovery).toHaveBeenCalledTimes(2);
    expect(run.store.checkoutExactRecovery).toHaveBeenCalledTimes(1);
    expect(run.calls).toEqual(['begin', 'map', 'exact', 'commit']);
  });

  it('never opens a transaction through an invalid checkout shell', async () => {
    const run = recoveryFixture();
    run.store.checkoutExactRecovery = vi.fn(async () => ({
      ...run.checkout, unexpected: true,
    } as any));

    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
    );

    expect(result.response.status).toBe(500);
    expect(run.checkout.open).not.toHaveBeenCalled();
    expect(run.calls).toEqual([]);
  });

  it('uses a captured checkout cleanup when open throws', async () => {
    const run = recoveryFixture();
    run.checkout.open = vi.fn(async () => {
      run.calls.push('begin'); throw new Error('provider open failed');
    });

    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
    );

    expect(result.response.status).toBe(500);
    expect(run.calls).toEqual(['begin', 'discard']);
  });

  it.each([
    'registry', 'store', 'checkout', 'discard', 'commit', 'rollback', 'quarantine',
    'ports', 'exact',
  ] as const)(
    'rejects a Proxy-wrapped recovery %s method without applying it', async (target) => {
      const run = recoveryFixture();
      let traps = 0;
      replaceRecoveryMethod(run, target, new Proxy(async () => undefined, {
        apply: () => { traps += 1; return undefined; },
      }));

      const result = await recoverExactSupervisorRegistrationV1(
        CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
      );

      expect(result.response.status).toBe(500);
      expect(traps).toBe(0);
      if (['commit', 'rollback', 'quarantine', 'ports', 'exact'].includes(target)) {
        expect(run.calls).toContain('discard');
      }
    },
  );

  it.each([
    'registry', 'store', 'checkout', 'discard', 'commit', 'rollback', 'quarantine',
    'ports', 'exact',
  ] as const)(
    'rejects an accessor recovery %s method without reading it', async (target) => {
      const run = recoveryFixture();
      let getters = 0;
      defineRecoveryAccessor(run, target, () => {
        getters += 1; return async () => undefined;
      });

      const result = await recoverExactSupervisorRegistrationV1(
        CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
      );

      expect(result.response.status).toBe(500);
      expect(getters).toBe(0);
      if (['commit', 'rollback', 'quarantine', 'ports', 'exact'].includes(target)) {
        expect(run.calls).toContain('discard');
      }
    },
  );

});

function recoveryFixture() {
  const calls: string[] = [];
  const transaction: SupervisorRegistrationRecoveryTransactionV1 = {
    ports: {
      mapAuthenticatedRecoveryPeer: async () => {
        calls.push('map'); return { kind: 'mapped', project: PROJECT };
      },
      lookupExactCommittedResult: async () => {
        calls.push('exact'); return exactStoredResult(201) as ExactCommittedResultReadV1;
      },
    },
    commit: async () => { calls.push('commit'); return 'committed'; },
    rollback: async () => { calls.push('rollback'); },
    quarantine: async () => { calls.push('quarantine'); },
  };
  const checkout = {
    open: vi.fn(async () => { calls.push('begin'); return transaction; }),
    discardMalformed: async () => { calls.push('discard'); },
  };
  const store: SupervisorRegistrationRecoveryStoreV1 = {
    checkoutExactRecovery: vi.fn(async () => checkout),
  };
  let consumed = false;
  const registry: AuthenticatedExactRecoveryPeerRegistryV1 = {
    consumeExactRecovery: vi.fn((peer) => {
      if (consumed || peer !== RECOVERY) return false;
      consumed = true; return true;
    }),
  };
  return { calls, registry, store, transaction, checkout };
}

type RecoveryFixture = ReturnType<typeof recoveryFixture>;
type RecoveryTarget = 'registry' | 'store' | 'checkout' | 'discard' | 'commit' | 'rollback'
  | 'quarantine' | 'ports' | 'exact';

function targetAndKey(run: RecoveryFixture, target: RecoveryTarget): [Record<string, any>, string] {
  if (target === 'registry') return [run.registry as any, 'consumeExactRecovery'];
  if (target === 'store') return [run.store as any, 'checkoutExactRecovery'];
  if (target === 'checkout') return [run.checkout as any, 'open'];
  if (target === 'discard') return [run.checkout as any, 'discardMalformed'];
  if (target === 'ports') return [run.transaction.ports as any, 'mapAuthenticatedRecoveryPeer'];
  if (target === 'exact') return [run.transaction.ports as any, 'lookupExactCommittedResult'];
  return [run.transaction as any, target];
}

function replaceRecoveryMethod(
  run: RecoveryFixture, target: RecoveryTarget, method: (...args: any[]) => any,
): void {
  const [record, key] = targetAndKey(run, target);
  record[key] = method;
}

function defineRecoveryAccessor(
  run: RecoveryFixture, target: RecoveryTarget, getter: () => unknown,
): void {
  const [record, key] = targetAndKey(run, target);
  Object.defineProperty(record, key, { enumerable: true, get: getter });
}

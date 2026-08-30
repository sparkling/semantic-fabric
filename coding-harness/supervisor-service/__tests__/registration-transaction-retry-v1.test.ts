// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import type { AuthenticatedTransportPeerV1, ExactCommittedResultReadV1 } from
  '../src/index.js';
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
import { throwSupervisorRegistrationRetryableAbortV1 } from
  '../src/registration-transaction-retry-v1.js';
import {
  ACTIVE_HEAD,
  ACTIVE_SEMANTIC_HEAD,
  CANONICAL_REQUEST,
  PROJECT,
  READY_RECEIPT,
  READY_SEMANTIC_RECEIPT,
  coherentlyMutatedRegistrationEnvelope,
  exactStoredResult,
} from './registration-fixtures.js';

const WRITE = Symbol('retry-write') as AuthenticatedTransportPeerV1;
const RECOVERY = Symbol('retry-recovery') as AuthenticatedExactRecoveryPeerV1;
describe('registration transaction bounded retry V1', () => {
  it.each([
    'map', 'first-exact', 'head', 'receipt', 'run', 'stage', 'second-exact', 'commit',
  ] as const)('retries the complete registration decision after a known %s abort',
    async (retryPoint) => {
      const run = registrationRetryFixture(retryPoint);

      const result = await executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, WRITE, run.registry, run.store,
      );

      expect(result.response.status).toBe(201);
      expect(run.originalConsume).toHaveBeenCalledTimes(1);
      expect(run.originalCheckout).toHaveBeenCalledTimes(2);
      expect(run.mappedPeers.every((peer) => peer === WRITE)).toBe(true);
      expect(run.mappedPeers).toHaveLength(
        run.calls.filter((call) => call.endsWith(':map')).length,
      );
      expect(run.calls).toContain(
        retryPoint === 'commit' ? '1:quarantine' : '1:rollback',
      );
      expect(run.calls.filter((call) => call.startsWith('2:'))).toEqual([
        '2:open', '2:map', '2:exact:1', '2:head', '2:receipt', '2:run',
        '2:stage', '2:map', '2:exact:2', '2:commit',
      ]);
      if (['stage', 'second-exact', 'commit'].includes(retryPoint)) {
        expect(run.candidates).toHaveLength(2);
        expect(run.candidates[0]).not.toBe(run.candidates[1]);
      }
    });

  it('restarts exact-first and observes a concurrent committed result', async () => {
    const run = registrationRetryFixture('commit', { secondAttemptExactHit: true });

    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, WRITE, run.registry, run.store,
    );

    expect(result.response.status).toBe(201);
    expect(run.candidates).toHaveLength(1);
    expect(run.calls).toEqual([
      '1:open', '1:map', '1:exact:1', '1:head', '1:receipt', '1:run',
      '1:stage', '1:map', '1:exact:2', '1:commit', '1:quarantine',
      '2:open', '2:map', '2:exact:1', '2:commit',
    ]);
  });

  it('recomputes a candidate only from the second attempt snapshots', async () => {
    const run = registrationRetryFixture('stage', { varySecondAttemptSnapshots: true });

    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, WRITE, run.registry, run.store,
    );

    expect(result.response.status).toBe(201);
    expect(run.candidates).toHaveLength(2);
    expect(run.candidates.map((candidate) => candidate.expectedNextGlobalSequence))
      .toEqual(['1', '2']);
    expect(run.candidates[0]?.previousGlobal).toEqual(READY_RECEIPT.previousGlobal);
    expect(run.candidates[1]?.previousGlobal).toEqual(READY_SEMANTIC_RECEIPT.previousGlobal);
  });

  it('snapshots peer and checkout roots before hashing and reuses them across retries',
    async () => {
      const run = registrationRetryFixture('map');

      const pending = executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, WRITE, run.registry, run.store,
      );
      run.registry.consumeRegistration = run.replacementConsume;
      run.store.checkoutRegistration = run.replacementCheckout;
      const result = await pending;

      expect(result.response.status).toBe(201);
      expect(run.originalConsume).toHaveBeenCalledTimes(1);
      expect(run.replacementConsume).not.toHaveBeenCalled();
      expect(run.originalCheckout).toHaveBeenCalledTimes(2);
      expect(run.replacementCheckout).not.toHaveBeenCalled();
    });

  it.each(['map', 'commit'] as const)(
    'bounds repeated %s aborts at three fresh attempts', async (retryPoint) => {
      const run = registrationRetryFixture(retryPoint, { alwaysAbort: true });

      const result = await executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, WRITE, run.registry, run.store,
      );

      expect(result.response.status).toBe(500);
      expect(run.originalConsume).toHaveBeenCalledTimes(1);
      expect(run.originalCheckout).toHaveBeenCalledTimes(3);
      const terminal = retryPoint === 'commit' ? ':quarantine' : ':rollback';
      expect(run.calls.filter((call) => call.endsWith(terminal))).toHaveLength(3);
    },
  );

  it('does not retry rollback failure, ambiguous commit, or forged retry data', async () => {
    const rollback = registrationRetryFixture('map', { rollbackFails: true });
    const unknown = registrationRetryFixture('none', { unknownCommit: true });
    const forged = registrationRetryFixture('none', { forgedCommitResult: true });

    const results = await Promise.all([rollback, unknown, forged].map((run) =>
      executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, WRITE, run.registry, run.store,
      )));

    expect(results.map((result) => result.response.status)).toEqual([500, 500, 500]);
    expect(rollback.originalCheckout).toHaveBeenCalledTimes(1);
    expect(rollback.calls.slice(-2)).toEqual(['1:rollback', '1:quarantine']);
    expect(unknown.originalCheckout).toHaveBeenCalledTimes(1);
    expect(unknown.calls.slice(-2)).toEqual(['1:commit', '1:quarantine']);
    expect(forged.originalCheckout).toHaveBeenCalledTimes(1);
    expect(forged.calls.slice(-2)).toEqual(['1:commit', '1:quarantine']);
  });

  it.each([
    ['ordinary failure', { quarantineFails: true }],
    ['retry marker', { quarantineThrowsMarker: true }],
  ] as const)('requires successful commit-abort quarantine after %s', async (_label, option) => {
    const run = registrationRetryFixture('commit', option);

    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, WRITE, run.registry, run.store,
    );

    expect(result.response.status).toBe(500);
    expect(run.originalCheckout).toHaveBeenCalledTimes(1);
    expect(run.calls.slice(-2)).toEqual(['1:commit', '1:quarantine']);
  });

  it('stops when a fresh retry checkout cannot open', async () => {
    const run = registrationRetryFixture('map', { failOpenAttempt: 2 });

    const result = await executeSupervisorRegistrationTransactionV1(
      CANONICAL_REQUEST, WRITE, run.registry, run.store,
    );

    expect(result.response.status).toBe(500);
    expect(run.originalCheckout).toHaveBeenCalledTimes(2);
    expect(run.calls.slice(-2)).toEqual(['2:open', '2:discard']);
  });

  it('awaits write cleanup before opening the next attempt', async () => {
    const rollbackGate = cleanupGate();
    const quarantineGate = cleanupGate();
    const rollback = registrationRetryFixture('map', { rollbackGate });
    const quarantine = registrationRetryFixture('commit', { quarantineGate });
    const pending = [rollback, quarantine].map((run) =>
      executeSupervisorRegistrationTransactionV1(
        CANONICAL_REQUEST, WRITE, run.registry, run.store,
      ));
    await Promise.all([rollbackGate.entered, quarantineGate.entered]);
    expect([rollback.originalCheckout.mock.calls.length,
      quarantine.originalCheckout.mock.calls.length]).toEqual([1, 1]);
    rollbackGate.release(); quarantineGate.release();
    expect((await Promise.all(pending)).map((result) => result.response.status))
      .toEqual([201, 201]);
  });

  it.each([
    ['map', [
      '1:open', '1:map', '1:rollback',
      '2:open', '2:map', '2:exact', '2:commit',
    ]],
    ['exact', [
      '1:open', '1:map', '1:exact', '1:rollback',
      '2:open', '2:map', '2:exact', '2:commit',
    ]],
    ['commit', [
      '1:open', '1:map', '1:exact', '1:commit', '1:quarantine',
      '2:open', '2:map', '2:exact', '2:commit',
    ]],
  ] as const)('uses fresh recovery capabilities after a known %s abort',
    async (retryPoint, expectedCalls) => {
      const run = recoveryRetryFixture(retryPoint);

      const result = await recoverExactSupervisorRegistrationV1(
        CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
      );

      expect(result.response.status).toBe(201);
      expect(run.originalConsume).toHaveBeenCalledTimes(1);
      expect(run.originalCheckout).toHaveBeenCalledTimes(2);
      expect(new Set(run.checkouts).size).toBe(2);
      expect(new Set(run.transactions).size).toBe(2);
      expect(new Set(run.ports).size).toBe(2);
      expect(run.mappedPeers.every((peer) => peer === RECOVERY)).toBe(true);
      expect(run.calls).toEqual(expectedCalls);
    });

  it('snapshots recovery roots and requires commit-abort quarantine', async () => {
    const stable = recoveryRetryFixture('exact');
    const pending = recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, stable.registry, stable.store,
    );
    stable.registry.consumeExactRecovery = stable.replacementConsume;
    stable.store.checkoutExactRecovery = stable.replacementCheckout;
    const stableResult = await pending;

    const cleanup = recoveryRetryFixture('commit', { quarantineFails: true });
    const cleanupResult = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, cleanup.registry, cleanup.store,
    );

    expect(stableResult.response.status).toBe(201);
    expect(stable.originalConsume).toHaveBeenCalledTimes(1);
    expect(stable.replacementConsume).not.toHaveBeenCalled();
    expect(stable.originalCheckout).toHaveBeenCalledTimes(2);
    expect(stable.replacementCheckout).not.toHaveBeenCalled();
    expect(cleanupResult.response.status).toBe(500);
    expect(cleanup.originalCheckout).toHaveBeenCalledTimes(1);
    expect(cleanup.calls.slice(-2)).toEqual(['1:commit', '1:quarantine']);
  });

  it.each(['exact', 'commit'] as const)('bounds repeated recovery %s aborts',
    async (retryPoint) => {
    const run = recoveryRetryFixture(retryPoint, { alwaysAbort: true });

    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
    );

    expect(result.response.status).toBe(500);
    expect(run.originalConsume).toHaveBeenCalledTimes(1);
    expect(run.originalCheckout).toHaveBeenCalledTimes(3);
    const terminal = retryPoint === 'commit' ? ':quarantine' : ':rollback';
    expect(run.calls.filter((call) => call.endsWith(terminal))).toHaveLength(3);
  });

  it('awaits recovery cleanup and stops after rollback failure', async () => {
    const rollbackGate = cleanupGate();
    const quarantineGate = cleanupGate();
    const rollback = recoveryRetryFixture('exact', { rollbackGate });
    const quarantine = recoveryRetryFixture('commit', { quarantineGate });
    const pending = [rollback, quarantine].map((run) => recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
    ));
    await Promise.all([rollbackGate.entered, quarantineGate.entered]);
    expect([rollback.originalCheckout.mock.calls.length,
      quarantine.originalCheckout.mock.calls.length]).toEqual([1, 1]);
    rollbackGate.release(); quarantineGate.release();
    expect((await Promise.all(pending)).map((result) => result.response.status))
      .toEqual([201, 201]);
    const failure = recoveryRetryFixture('exact', { rollbackFails: true });
    expect((await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, failure.registry, failure.store,
    )).response.status).toBe(500);
    expect(failure.originalCheckout).toHaveBeenCalledTimes(1);
  });
});

type CleanupGate = ReturnType<typeof cleanupGate>;

type RegistrationRetryPoint = 'none' | 'map' | 'first-exact' | 'head' | 'receipt'
  | 'run' | 'stage' | 'second-exact' | 'commit';

type RegistrationRetryOptions = Readonly<{
  alwaysAbort?: boolean;
  failOpenAttempt?: number;
  forgedCommitResult?: boolean;
  quarantineFails?: boolean;
  quarantineThrowsMarker?: boolean;
  quarantineGate?: CleanupGate;
  rollbackFails?: boolean;
  rollbackGate?: CleanupGate;
  secondAttemptExactHit?: boolean;
  unknownCommit?: boolean;
  varySecondAttemptSnapshots?: boolean;
}>;

function registrationRetryFixture(
  retryPoint: RegistrationRetryPoint,
  options: RegistrationRetryOptions = {},
) {
  const calls: string[] = [];
  const candidates: Readonly<Record<string, unknown>>[] = [];
  const mappedPeers: unknown[] = [];
  const secondStored = exactStoredResult(201, {
    serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
      event.globalSequence = '2';
      event.previousGlobal = READY_SEMANTIC_RECEIPT.previousGlobal;
    }),
  }, { originalRegistrationGlobalSequence: '2' });
  let attempts = 0;
  let consumed = false;
  const shouldAbort = (attempt: number, point: RegistrationRetryPoint) =>
    retryPoint === point && (options.alwaysAbort === true || attempt === 1);
  const abortAt = (attempt: number, point: RegistrationRetryPoint): void => {
    if (shouldAbort(attempt, point)) throwSupervisorRegistrationRetryableAbortV1();
  };
  const originalCheckout = vi.fn(async () => {
    const attempt = ++attempts;
    let exactReads = 0;
    const prefix = String(attempt);
    const ports = {
      mapAuthenticatedPeer: async (peer: AuthenticatedTransportPeerV1) => {
        calls.push(prefix + ':map'); mappedPeers.push(peer); abortAt(attempt, 'map');
        return { kind: 'mapped' as const, project: PROJECT };
      },
      lookupExactCommittedResult: async () => {
        const read = ++exactReads;
        calls.push(prefix + ':exact:' + String(read));
        abortAt(attempt, read === 1 ? 'first-exact' : 'second-exact');
        if (attempt === 2 && read === 1 && options.secondAttemptExactHit === true) {
          return exactStoredResult(201) as ExactCommittedResultReadV1;
        }
        if (read === 1) return { kind: 'absent' as const };
        return options.varySecondAttemptSnapshots === true && attempt === 2
          ? secondStored as ExactCommittedResultReadV1
          : exactStoredResult(201) as ExactCommittedResultReadV1;
      },
      readActiveAuthorityHead: async () => {
        calls.push(prefix + ':head'); abortAt(attempt, 'head');
        return options.varySecondAttemptSnapshots === true && attempt === 2
          ? ACTIVE_SEMANTIC_HEAD : ACTIVE_HEAD;
      },
      readRequiredPredecessorReceipt: async () => {
        calls.push(prefix + ':receipt'); abortAt(attempt, 'receipt');
        return options.varySecondAttemptSnapshots === true && attempt === 2
          ? READY_SEMANTIC_RECEIPT : READY_RECEIPT;
      },
      readRunState: async () => {
        calls.push(prefix + ':run'); abortAt(attempt, 'run');
        return { kind: 'absent' as const };
      },
    };
    const transaction: SupervisorRegistrationTransactionV1 = {
      ports,
      stageCandidate: async (candidate) => {
        calls.push(prefix + ':stage'); candidates.push(candidate); abortAt(attempt, 'stage');
      },
      commit: async () => {
        calls.push(prefix + ':commit');
        if (shouldAbort(attempt, 'commit')) throwSupervisorRegistrationRetryableAbortV1();
        if (options.forgedCommitResult === true) return 'aborted-retryable' as never;
        return options.unknownCommit === true ? 'unknown' : 'committed';
      },
      rollback: async () => {
        calls.push(prefix + ':rollback');
        if (options.rollbackFails === true) throw new Error('rollback failed');
        options.rollbackGate?.enter(); await options.rollbackGate?.blocked;
      },
      quarantine: async () => {
        calls.push(prefix + ':quarantine');
        if (options.quarantineThrowsMarker === true) {
          throwSupervisorRegistrationRetryableAbortV1();
        }
        if (options.quarantineFails === true) throw new Error('quarantine failed');
        options.quarantineGate?.enter(); await options.quarantineGate?.blocked;
      },
    };
    return {
      open: async () => {
        calls.push(prefix + ':open');
        if (options.failOpenAttempt === attempt) throw new Error('open failed');
        return transaction;
      },
      discardMalformed: async () => { calls.push(prefix + ':discard'); },
    };
  });
  const replacementCheckout = vi.fn(async () => { throw new Error('swapped checkout'); });
  const store: SupervisorRegistrationTransactionStoreV1 = {
    checkoutRegistration: originalCheckout,
  };
  const originalConsume = vi.fn((peer: AuthenticatedTransportPeerV1) => {
    if (consumed || peer !== WRITE) return false;
    consumed = true; return true;
  });
  const replacementConsume = vi.fn(() => false);
  const registry: AuthenticatedRegistrationPeerRegistryV1 = {
    consumeRegistration: originalConsume,
  };
  return {
    calls, candidates, mappedPeers, originalCheckout, originalConsume,
    registry, replacementCheckout, replacementConsume, store,
  };
}

type RecoveryRetryPoint = 'map' | 'exact' | 'commit';

function recoveryRetryFixture(
  retryPoint: RecoveryRetryPoint,
  options: Readonly<{
    alwaysAbort?: boolean;
    quarantineFails?: boolean;
    quarantineThrowsMarker?: boolean;
    quarantineGate?: CleanupGate;
    rollbackFails?: boolean;
    rollbackGate?: CleanupGate;
  }> = {},
) {
  const calls: string[] = [];
  const checkouts: object[] = [];
  const mappedPeers: unknown[] = [];
  const ports: object[] = [];
  const transactions: object[] = [];
  let attempts = 0;
  let consumed = false;
  const shouldAbort = (attempt: number, point: RecoveryRetryPoint) =>
    retryPoint === point && (options.alwaysAbort === true || attempt === 1);
  const originalCheckout = vi.fn(async () => {
    const attempt = ++attempts;
    const prefix = String(attempt);
    const attemptPorts = {
      mapAuthenticatedRecoveryPeer: async (peer: AuthenticatedExactRecoveryPeerV1) => {
        calls.push(prefix + ':map'); mappedPeers.push(peer);
        if (shouldAbort(attempt, 'map')) throwSupervisorRegistrationRetryableAbortV1();
        return { kind: 'mapped' as const, project: PROJECT };
      },
      lookupExactCommittedResult: async () => {
        calls.push(prefix + ':exact');
        if (shouldAbort(attempt, 'exact')) throwSupervisorRegistrationRetryableAbortV1();
        return exactStoredResult(201) as ExactCommittedResultReadV1;
      },
    };
    const transaction: SupervisorRegistrationRecoveryTransactionV1 = {
      ports: attemptPorts,
      commit: async () => {
        calls.push(prefix + ':commit');
        if (shouldAbort(attempt, 'commit')) throwSupervisorRegistrationRetryableAbortV1();
        return 'committed';
      },
      rollback: async () => {
        calls.push(prefix + ':rollback');
        if (options.rollbackFails === true) throw new Error('rollback failed');
        options.rollbackGate?.enter(); await options.rollbackGate?.blocked;
      },
      quarantine: async () => {
        calls.push(prefix + ':quarantine');
        if (options.quarantineThrowsMarker === true) {
          throwSupervisorRegistrationRetryableAbortV1();
        }
        if (options.quarantineFails === true) throw new Error('quarantine failed');
        options.quarantineGate?.enter(); await options.quarantineGate?.blocked;
      },
    };
    const checkout = {
      open: async () => { calls.push(prefix + ':open'); return transaction; },
      discardMalformed: async () => { calls.push(prefix + ':discard'); },
    };
    ports.push(attemptPorts); transactions.push(transaction); checkouts.push(checkout);
    return checkout;
  });
  const replacementCheckout = vi.fn(async () => { throw new Error('swapped checkout'); });
  const store: SupervisorRegistrationRecoveryStoreV1 = {
    checkoutExactRecovery: originalCheckout,
  };
  const originalConsume = vi.fn((peer: AuthenticatedExactRecoveryPeerV1) => {
    if (consumed || peer !== RECOVERY) return false;
    consumed = true; return true;
  });
  const replacementConsume = vi.fn(() => false);
  const registry: AuthenticatedExactRecoveryPeerRegistryV1 = {
    consumeExactRecovery: originalConsume,
  };
  return {
    calls, checkouts, mappedPeers, originalCheckout, originalConsume, ports,
    registry, replacementCheckout, replacementConsume, store, transactions,
  };
}

function cleanupGate() {
  let enter!: () => void;
  let release!: () => void;
  const entered = new Promise<void>((resolve) => { enter = resolve; });
  const blocked = new Promise<void>((resolve) => { release = resolve; });
  return { blocked, enter, entered, release };
}

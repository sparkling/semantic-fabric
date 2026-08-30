// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import type { ExactCommittedResultReadV1 } from '../src/index.js';
import {
  recoverExactSupervisorRegistrationV1,
  type AuthenticatedExactRecoveryPeerRegistryV1,
  type AuthenticatedExactRecoveryPeerV1,
  type SupervisorRegistrationRecoveryStoreV1,
  type SupervisorRegistrationRecoveryTransactionV1,
} from '../src/registration-transaction-v1.js';
import { throwSupervisorRegistrationRetryableAbortV1 } from
  '../src/registration-transaction-retry-v1.js';
import { CANONICAL_REQUEST, PROJECT, exactStoredResult } from './registration-fixtures.js';

const RECOVERY = Symbol('retry-rejection-recovery') as AuthenticatedExactRecoveryPeerV1;

describe('registration transaction retry rejection V1', () => {
  it.each([
    ['unknown return', { commitResult: 'unknown' }],
    ['forged retry string', { commitResult: 'aborted-retryable' }],
    ['malformed object', { commitResult: { committed: true } }],
    ['ordinary throw', { commitError: new Error('connection lost') }],
    ['SQLSTATE-shaped throw', { commitError: { code: '40001' } }],
    ['independent symbol', { commitError: Symbol('supervisor-registration-retryable-abort-v1') }],
  ] as const)('does not retry a recovery %s', async (_label, option) => {
    const run = recoveryRejectionFixture(option);

    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
    );

    expect(result.response.status).toBe(500);
    expect(run.checkout).toHaveBeenCalledTimes(1);
    expect(run.calls.slice(-2)).toEqual(['commit', 'quarantine']);
  });

  it.each([
    ['SQLSTATE-shaped object', { code: '40001' }],
    ['SQLSTATE text', new Error('SQLSTATE 40001 serialization failure')],
    ['independent symbol', Symbol('supervisor-registration-retryable-abort-v1')],
  ] as const)('does not retry a recovery read %s', async (_label, readError) => {
    const run = recoveryRejectionFixture({ readError });

    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
    );

    expect(result.response.status).toBe(500);
    expect(run.checkout).toHaveBeenCalledTimes(1);
    expect(run.calls).toEqual(['open', 'map', 'exact', 'commit']);
  });

  it('does not retry when recovery rollback fails after the opaque marker', async () => {
    const run = recoveryRejectionFixture({ retryRead: true, rollbackFails: true });

    const result = await recoverExactSupervisorRegistrationV1(
      CANONICAL_REQUEST, RECOVERY, run.registry, run.store,
    );

    expect(result.response.status).toBe(500);
    expect(run.checkout).toHaveBeenCalledTimes(1);
    expect(run.calls).toEqual(['open', 'map', 'exact', 'rollback', 'quarantine']);
  });
});

type RejectionOptions = Readonly<{
  commitError?: unknown;
  commitResult?: unknown;
  readError?: unknown;
  retryRead?: boolean;
  rollbackFails?: boolean;
}>;

function recoveryRejectionFixture(options: RejectionOptions) {
  const calls: string[] = [];
  const transaction: SupervisorRegistrationRecoveryTransactionV1 = {
    ports: {
      mapAuthenticatedRecoveryPeer: async (peer) => {
        calls.push('map');
        return peer === RECOVERY
          ? { kind: 'mapped', project: PROJECT } : { kind: 'not-admitted' };
      },
      lookupExactCommittedResult: async () => {
        calls.push('exact');
        if (options.retryRead === true) throwSupervisorRegistrationRetryableAbortV1();
        if (options.readError !== undefined) throw options.readError;
        return exactStoredResult(201) as ExactCommittedResultReadV1;
      },
    },
    commit: async () => {
      calls.push('commit');
      if (options.commitError !== undefined) throw options.commitError;
      return (options.commitResult ?? 'committed') as never;
    },
    rollback: async () => {
      calls.push('rollback');
      if (options.rollbackFails === true) throw new Error('rollback failed');
    },
    quarantine: async () => { calls.push('quarantine'); },
  };
  const checkout = vi.fn(async () => ({
    open: async () => { calls.push('open'); return transaction; },
    discardMalformed: async () => { calls.push('discard'); },
  }));
  const store: SupervisorRegistrationRecoveryStoreV1 = { checkoutExactRecovery: checkout };
  let consumed = false;
  const registry: AuthenticatedExactRecoveryPeerRegistryV1 = {
    consumeExactRecovery: (peer) => {
      const admitted = !consumed && peer === RECOVERY;
      consumed = true; return admitted;
    },
  };
  return { calls, checkout, registry, store };
}

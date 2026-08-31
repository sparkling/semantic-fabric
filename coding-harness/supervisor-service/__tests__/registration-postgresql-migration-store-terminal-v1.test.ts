// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  capturePostgresMigrationCheckoutShellV1,
  capturePostgresMigrationSessionV1,
  capturePostgresMigrationStoreV1,
} from '../src/registration-postgresql-migration-store-contract-v1.js';
import {
  POSTGRES_MIGRATION_RUNNER_RESULT_OUTCOMES_V1,
  postgresMigrationRunnerTerminalResultV1,
} from '../src/registration-postgresql-migration-terminal-results-v1.js';

const OUTCOMES = [
  'applied',
  'exact-no-op',
  'committed-cleanup-failed',
  'rejected',
  'rejected-cleanup-failed',
  'resolution-unknown',
  'commit-resolution-unknown',
] as const;

describe('PostgreSQL migration descriptor-only capability capture V1', () => {
  it('captures exact method descriptors without invocation or Promise assimilation', () => {
    let calls = 0;
    const checkoutMigration = (): unknown => { calls += 1; return null; };
    const open = (): unknown => { calls += 1; return null; };
    const discardMalformed = (): unknown => { calls += 1; return null; };
    const execute = (): unknown => { calls += 1; return null; };
    const release = (): unknown => { calls += 1; return null; };
    const destroy = (): unknown => { calls += 1; return null; };
    const store = { checkoutMigration };
    const shell = { open, discardMalformed };
    const session = { execute, release, destroy };

    const capturedStore = capturePostgresMigrationStoreV1(store);
    const capturedShell = capturePostgresMigrationCheckoutShellV1(shell);
    const capturedSession = capturePostgresMigrationSessionV1(session);
    store.checkoutMigration = () => 'replacement';
    shell.open = () => 'replacement';
    session.execute = () => 'replacement';

    expect(capturedStore.checkoutMigration).not.toBe(checkoutMigration);
    expect(capturedShell.open).not.toBe(open);
    expect(capturedShell.discardMalformed).not.toBe(discardMalformed);
    expect(capturedSession.execute).not.toBe(execute);
    expect(capturedSession.release).not.toBe(release);
    expect(capturedSession.destroy).not.toBe(destroy);
    expect(Reflect.ownKeys(capturedStore)).toEqual(['checkoutMigration']);
    expect(Reflect.ownKeys(capturedShell)).toEqual(['open', 'discardMalformed']);
    expect(Reflect.ownKeys(capturedSession)).toEqual(['execute', 'release', 'destroy']);
    expect(calls).toBe(0);
    expect(Object.isFrozen(capturedStore)).toBe(true);
    expect(Object.isFrozen(capturedShell)).toBe(true);
    expect(Object.isFrozen(capturedSession)).toBe(true);
    expect(Object.isFrozen(capturedStore.checkoutMigration)).toBe(true);
    expect(Object.isFrozen(capturedShell.open)).toBe(true);
    expect(Object.isFrozen(capturedSession.execute)).toBe(true);
  });

  it('does not inspect, call, await, or assimilate a method result', () => {
    let thenReads = 0;
    const hostileThenable = {};
    Object.defineProperty(hostileThenable, 'then', {
      get: () => { thenReads += 1; throw new Error('must not assimilate'); },
    });
    let calls = 0;
    const checkoutMigration = (): unknown => { calls += 1; return hostileThenable; };
    const captured = capturePostgresMigrationStoreV1({ checkoutMigration });
    expect(captured.checkoutMigration).not.toBe(checkoutMigration);
    expect(calls).toBe(0);
    expect(thenReads).toBe(0);
  });

  it('uses undefined as the invocation receiver without assimilating the result', () => {
    let receiver: unknown = 'not-called';
    let thenReads = 0;
    const hostileThenable = {};
    Object.defineProperty(hostileThenable, 'then', {
      get: () => { thenReads += 1; throw new Error('must not assimilate'); },
    });
    const checkoutMigration = function (this: unknown): unknown {
      receiver = this;
      return hostileThenable;
    };
    const captured = capturePostgresMigrationStoreV1({ checkoutMigration });
    const hostileReceiver = { captured: true };

    expect(captured.checkoutMigration.call(hostileReceiver)).toBe(hostileThenable);
    expect(receiver).toBe(undefined);
    expect(thenReads).toBe(0);
  });

  it('rejects widened, reordered, exotic, symbolic, accessor, and proxied records', () => {
    const method = (): null => null;
    const reversed = { discardMalformed: method, open: method };
    const symbolic = { checkoutMigration: method } as Record<PropertyKey, unknown>;
    symbolic[Symbol('hidden')] = method;
    const exotic = Object.create(null) as Record<string, unknown>;
    exotic.checkoutMigration = method;
    let reads = 0;
    const accessor = {};
    Object.defineProperty(accessor, 'checkoutMigration', {
      enumerable: true, get: () => { reads += 1; return method; },
    });
    let traps = 0;
    const proxy = new Proxy({ checkoutMigration: method }, {
      ownKeys: () => { traps += 1; return ['checkoutMigration']; },
      getOwnPropertyDescriptor: () => { traps += 1; return undefined; },
      getPrototypeOf: () => { traps += 1; return Object.prototype; },
    });
    const revoked = Proxy.revocable({ checkoutMigration: method }, {});
    revoked.revoke();
    const nonEnumerable = {};
    Object.defineProperty(nonEnumerable, 'checkoutMigration', {
      value: method, enumerable: false,
    });
    for (const candidate of [
      { checkoutMigration: method, extra: method }, symbolic, exotic, accessor,
      proxy, revoked.proxy, nonEnumerable, null, [], method,
    ]) {
      expect(() => capturePostgresMigrationStoreV1(candidate)).toThrow(
        'PostgreSQL migration store capability is invalid',
      );
    }
    expect(() => capturePostgresMigrationCheckoutShellV1(reversed)).toThrow(
      'PostgreSQL migration checkout shell capability is invalid',
    );
    expect(reads).toBe(0);
    expect(traps).toBe(0);
  });

  it('does not invoke an inherited setter while defining captured methods', () => {
    const original = Object.getOwnPropertyDescriptor(Object.prototype, 'checkoutMigration');
    let setterCalls = 0;
    Object.defineProperty(Object.prototype, 'checkoutMigration', {
      set: () => { setterCalls += 1; },
      configurable: true,
    });
    try {
      const captured = capturePostgresMigrationStoreV1({
        checkoutMigration: (): null => null,
      });
      expect(Object.hasOwn(captured, 'checkoutMigration')).toBe(true);
      expect(setterCalls).toBe(0);
    } finally {
      if (original === undefined) {
        delete (Object.prototype as { checkoutMigration?: unknown }).checkoutMigration;
      } else {
        Object.defineProperty(Object.prototype, 'checkoutMigration', original);
      }
    }
  });

  it('uses captured construction intrinsics after hostile post-import replacement', () => {
    const arrayIsArray = Array.isArray;
    const defineProperty = Object.defineProperty;
    const freeze = Object.freeze;
    const typeError = TypeError;
    let poisonCalls = 0;
    let captured: ReturnType<typeof capturePostgresMigrationStoreV1> | undefined;
    try {
      Array.isArray = (_candidate: unknown): _candidate is unknown[] => {
        poisonCalls += 1;
        throw new Error('poison');
      };
      Object.defineProperty = (): never => { poisonCalls += 1; throw new Error('poison'); };
      Object.freeze = (): never => { poisonCalls += 1; throw new Error('poison'); };
      defineProperty(globalThis, 'TypeError', {
        value: class PoisonTypeError extends Error {},
        configurable: true,
        writable: true,
      });
      captured = capturePostgresMigrationStoreV1({
        checkoutMigration: (): null => null,
      });
    } finally {
      Array.isArray = arrayIsArray;
      Object.defineProperty = defineProperty;
      Object.freeze = freeze;
      defineProperty(globalThis, 'TypeError', {
        value: typeError,
        configurable: true,
        writable: true,
      });
    }
    expect(Reflect.ownKeys(captured ?? {})).toEqual(['checkoutMigration']);
    expect(Object.isFrozen(captured)).toBe(true);
    expect(poisonCalls).toBe(0);
  });

  it('rejects nonfunctions and proxied method values without invoking proxy traps', () => {
    let traps = 0;
    const proxiedMethod = new Proxy((): null => null, {
      apply: () => { traps += 1; return null; },
      get: () => { traps += 1; return undefined; },
    });
    expect(() => capturePostgresMigrationStoreV1({ checkoutMigration: null }))
      .toThrow('PostgreSQL migration store capability is invalid');
    expect(() => capturePostgresMigrationStoreV1({
      checkoutMigration: proxiedMethod,
    })).toThrow('PostgreSQL migration store capability is invalid');
    expect(traps).toBe(0);
  });
});

describe('PostgreSQL migration exact terminal singleton results V1', () => {
  it('pins the seven outcomes and exact non-authorizing record shape', () => {
    expect(POSTGRES_MIGRATION_RUNNER_RESULT_OUTCOMES_V1).toEqual(OUTCOMES);
    expect(Object.isFrozen(POSTGRES_MIGRATION_RUNNER_RESULT_OUTCOMES_V1)).toBe(true);
    const identities = new Set();
    for (const outcome of OUTCOMES) {
      const result = postgresMigrationRunnerTerminalResultV1(outcome);
      identities.add(result);
      expect(result).toEqual({
        resultKind: 'postgresql-migration-runner-result-v1',
        outcome,
        authority: 'none',
        readinessAuthorized: false,
        databaseAccessAuthorized: false,
        migrationApplyAuthorized: false,
      });
      expect(Reflect.ownKeys(result)).toEqual([
        'resultKind', 'outcome', 'authority', 'readinessAuthorized',
        'databaseAccessAuthorized', 'migrationApplyAuthorized',
      ]);
      expect(Object.getPrototypeOf(result)).toBe(Object.prototype);
      expect(Object.isFrozen(result)).toBe(true);
      expect(postgresMigrationRunnerTerminalResultV1(outcome)).toBe(result);
    }
    expect(identities.size).toBe(7);
  });

  it.each([null, undefined, '', 'APPLIED', 'unknown', new String('applied')])(
    'rejects non-outcome %p without reflecting it', (outcome) => {
      expect(() => postgresMigrationRunnerTerminalResultV1(outcome)).toThrow(
        'PostgreSQL migration runner outcome is invalid',
      );
    },
  );
});

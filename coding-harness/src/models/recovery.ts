// SPDX-License-Identifier: MIT

import {
  CircuitBreaker,
  RetryBudget,
  type BreakerState,
} from '@metaharness/harness';
import type { NativeModelCandidate } from './routing.js';
import type { NativeHost } from './types.js';

export type NativeOperation =
  | 'architecture'
  | 'implementation'
  | 'repair'
  | 'review'
  | 'evolution-reflection';

export type NativeRecoveryOutcome =
  | 'success'
  | 'failure'
  | 'transient-retry'
  | 'cancelled'
  | 'circuit-open';

export interface NativeRecoveryEvent {
  readonly host: NativeHost;
  readonly candidateId: string;
  readonly model: string;
  readonly operation: NativeOperation;
  readonly attempt: number;
  readonly outcome: NativeRecoveryOutcome;
  readonly breakerState: BreakerState;
  readonly durationMs: number;
  readonly recordedAt: string;
}

export interface NativeRecoverySnapshot {
  readonly breakers: Readonly<Record<NativeHost, BreakerState>>;
  readonly events: readonly NativeRecoveryEvent[];
}

export interface NativeAttemptContext {
  readonly attempt: number;
  readonly signal?: AbortSignal;
}

export class TransientNativeHostError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'TransientNativeHostError';
  }
}

export class NativeCancellationError extends Error {
  constructor(message = 'HARNESS_NATIVE_INVOCATION_CANCELLED', options?: ErrorOptions) {
    super(message, options);
    this.name = 'NativeCancellationError';
  }
}

export class NativeInvocationRecovery {
  readonly #breakers: Record<NativeHost, CircuitBreaker>;
  readonly #events: NativeRecoveryEvent[] = [];
  readonly #now: () => number;
  readonly #isoNow: () => string;

  constructor(input: {
    readonly breakerThreshold?: number;
    readonly breakerCooldownMs?: number;
    readonly now?: () => number;
    readonly isoNow?: () => string;
  } = {}) {
    const threshold = input.breakerThreshold ?? 2;
    const cooldownMs = input.breakerCooldownMs ?? 60_000;
    this.#now = input.now ?? Date.now;
    this.#isoNow = input.isoNow ?? (() => new Date().toISOString());
    this.#breakers = {
      codex: new CircuitBreaker({ threshold, cooldownMs, now: this.#now }),
      'claude-code': new CircuitBreaker({
        threshold,
        cooldownMs,
        now: this.#now,
      }),
    };
  }

  async invoke<T>(input: {
    readonly candidate: NativeModelCandidate;
    readonly operation: NativeOperation;
    readonly signal?: AbortSignal;
    readonly invoke: (context: NativeAttemptContext) => Promise<T>;
  }): Promise<T> {
    const breaker = this.#breakers[input.candidate.host];
    const retryBudget = new RetryBudget(1, 0);
    let attempt = 0;

    while (attempt < 2) {
      if (input.signal?.aborted === true) {
        this.#record(input, attempt, 'cancelled', breaker.current(), 0);
        throw new NativeCancellationError();
      }
      if (!breaker.canProceed()) {
        this.#record(input, attempt, 'circuit-open', breaker.current(), 0);
        throw new Error(`HARNESS_NATIVE_CIRCUIT_OPEN:${input.candidate.host}`);
      }

      const startedAt = this.#now();
      try {
        const result = await input.invoke({ attempt, signal: input.signal });
        breaker.recordSuccess();
        this.#record(
          input,
          attempt,
          'success',
          breaker.current(),
          elapsed(this.#now(), startedAt),
        );
        return result;
      } catch (error) {
        const durationMs = elapsed(this.#now(), startedAt);
        if (isCancellation(error, input.signal)) {
          this.#record(input, attempt, 'cancelled', breaker.current(), durationMs);
          throw error instanceof NativeCancellationError
            ? error
            : new NativeCancellationError(undefined, { cause: error });
        }

        breaker.recordFailure();
        if (
          error instanceof TransientNativeHostError &&
          breaker.canProceed() &&
          retryBudget.tryConsume(0)
        ) {
          this.#record(
            input,
            attempt,
            'transient-retry',
            breaker.current(),
            durationMs,
          );
          attempt += 1;
          continue;
        }
        this.#record(
          input,
          attempt,
          'failure',
          breaker.current(),
          durationMs,
        );
        throw error;
      }
    }
    throw new Error('HARNESS_NATIVE_RETRY_BUDGET_EXHAUSTED');
  }

  snapshot(): NativeRecoverySnapshot {
    return Object.freeze({
      breakers: Object.freeze({
        codex: this.#breakers.codex.current(),
        'claude-code': this.#breakers['claude-code'].current(),
      }),
      events: Object.freeze(this.#events.map((event) => Object.freeze({ ...event }))),
    });
  }

  #record(
    input: {
      readonly candidate: NativeModelCandidate;
      readonly operation: NativeOperation;
    },
    attempt: number,
    outcome: NativeRecoveryOutcome,
    breakerState: BreakerState,
    durationMs: number,
  ): void {
    this.#events.push(
      Object.freeze({
        host: input.candidate.host,
        candidateId: input.candidate.id,
        model: input.candidate.model,
        operation: input.operation,
        attempt,
        outcome,
        breakerState,
        durationMs,
        recordedAt: this.#isoNow(),
      }),
    );
  }
}

function elapsed(now: number, startedAt: number): number {
  return Math.max(0, Math.round(now - startedAt));
}

function isCancellation(error: unknown, signal?: AbortSignal): boolean {
  return (
    signal?.aborted === true ||
    error instanceof NativeCancellationError ||
    (error instanceof Error && error.name === 'AbortError')
  );
}

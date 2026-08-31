// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';

const ARRAY_IS_ARRAY = Array.isArray;
const DEFINE_PROPERTY = Object.defineProperty;
const FREEZE = Object.freeze;
const GET_OWN_PROPERTY_DESCRIPTOR = Object.getOwnPropertyDescriptor;
const GET_PROTOTYPE_OF = Object.getPrototypeOf;
const HAS_OWN = Object.hasOwn;
const OWN_KEYS = Reflect.ownKeys;
const REFLECT_APPLY = Reflect.apply;
const PLAIN_OBJECT_PROTOTYPE = Object.prototype;
const TYPE_ERROR = TypeError;

export interface CapturedPostgresMigrationStoreV1 {
  readonly checkoutMigration: () => unknown;
}

export interface CapturedPostgresMigrationCheckoutShellV1 {
  readonly open: () => unknown;
  readonly discardMalformed: () => unknown;
}

export interface CapturedPostgresMigrationSessionV1 {
  readonly execute: (message: unknown) => unknown;
  readonly release: () => unknown;
  readonly destroy: () => unknown;
}

/** Capture the sole store method as data. This never calls or awaits the method. */
export function capturePostgresMigrationStoreV1(
  value: unknown,
): CapturedPostgresMigrationStoreV1 {
  return captureCapabilityRecordV1(
    value,
    ['checkoutMigration'],
    'PostgreSQL migration store capability is invalid',
  ) as unknown as CapturedPostgresMigrationStoreV1;
}

/** Capture shell methods as data. No returned value or thenable is inspected. */
export function capturePostgresMigrationCheckoutShellV1(
  value: unknown,
): CapturedPostgresMigrationCheckoutShellV1 {
  return captureCapabilityRecordV1(
    value,
    ['open', 'discardMalformed'],
    'PostgreSQL migration checkout shell capability is invalid',
  ) as unknown as CapturedPostgresMigrationCheckoutShellV1;
}

/** Capture session methods as data. Invocation belongs to the future runner. */
export function capturePostgresMigrationSessionV1(
  value: unknown,
): CapturedPostgresMigrationSessionV1 {
  return captureCapabilityRecordV1(
    value,
    ['execute', 'release', 'destroy'],
    'PostgreSQL migration session capability is invalid',
  ) as unknown as CapturedPostgresMigrationSessionV1;
}

function captureCapabilityRecordV1(
  value: unknown,
  expectedKeys: readonly string[],
  error: string,
): Readonly<Record<string, (...arguments_: readonly unknown[]) => unknown>> {
  try {
    if (isProxy(value) || value === null || typeof value !== 'object'
      || ARRAY_IS_ARRAY(value) || GET_PROTOTYPE_OF(value) !== PLAIN_OBJECT_PROTOTYPE) {
      throw new TYPE_ERROR();
    }
    const keys = OWN_KEYS(value);
    if (keys.length !== expectedKeys.length) throw new TYPE_ERROR();
    const captured: Record<string, (...arguments_: readonly unknown[]) => unknown> = {};
    for (let index = 0; index < expectedKeys.length; index += 1) {
      const key = keys[index];
      if (typeof key !== 'string' || key !== expectedKeys[index]) throw new TYPE_ERROR();
      const descriptor = GET_OWN_PROPERTY_DESCRIPTOR(value, key);
      if (descriptor === undefined || descriptor.enumerable !== true
        || !HAS_OWN(descriptor, 'value') || typeof descriptor.value !== 'function'
        || isProxy(descriptor.value)) {
        throw new TYPE_ERROR();
      }
      const capability = descriptor.value as (...arguments_: readonly unknown[]) => unknown;
      const receiverFree = FREEZE((...arguments_: readonly unknown[]): unknown => (
        REFLECT_APPLY(capability, undefined, arguments_)
      ));
      DEFINE_PROPERTY(captured, key, {
        value: receiverFree,
        enumerable: true,
        configurable: false,
        writable: false,
      });
    }
    return FREEZE(captured);
  } catch {
    throw new TYPE_ERROR(error);
  }
}

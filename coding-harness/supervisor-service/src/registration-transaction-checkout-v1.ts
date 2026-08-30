// SPDX-License-Identifier: MIT

import {
  capabilityRecordV1,
  captureCapabilityMethodV1,
} from './registration-transaction-boundary-v1.js';
import type {
  SupervisorRegistrationRecoveryStoreV1,
  SupervisorRegistrationTransactionStoreV1,
} from './registration-transaction-contract-v1.js';

export function captureRegistrationCheckoutV1(
  store: SupervisorRegistrationTransactionStoreV1,
): SupervisorRegistrationTransactionStoreV1['checkoutRegistration'] | null {
  try {
    return captureCapabilityMethodV1(
      store, ['checkoutRegistration'], 'transaction store', 'checkoutRegistration',
    ) as SupervisorRegistrationTransactionStoreV1['checkoutRegistration'];
  } catch {
    return null;
  }
}

export function captureExactRecoveryCheckoutV1(
  store: SupervisorRegistrationRecoveryStoreV1,
): SupervisorRegistrationRecoveryStoreV1['checkoutExactRecovery'] | null {
  try {
    return captureCapabilityMethodV1(
      store, ['checkoutExactRecovery'], 'recovery store', 'checkoutExactRecovery',
    ) as SupervisorRegistrationRecoveryStoreV1['checkoutExactRecovery'];
  } catch {
    return null;
  }
}

export async function openRegistrationCheckoutV1<T>(
  checkoutRegistration: SupervisorRegistrationTransactionStoreV1['checkoutRegistration'],
  bind: (value: unknown) => T,
): Promise<T | null> {
  return openCheckout(
    checkoutRegistration, 'registration checkout', bind,
  );
}

export async function openExactRecoveryCheckoutV1<T>(
  checkoutExactRecovery: SupervisorRegistrationRecoveryStoreV1['checkoutExactRecovery'],
  bind: (value: unknown) => T,
): Promise<T | null> {
  return openCheckout(
    checkoutExactRecovery, 'recovery checkout', bind,
  );
}

async function openCheckout<T>(
  checkoutCapability: () => Promise<unknown>,
  label: string,
  bind: (value: unknown) => T,
): Promise<T | null> {
  try {
    const keys = ['open', 'discardMalformed'] as const;
    const checkout = capabilityRecordV1(await checkoutCapability(), keys, label);
    const open = captureCapabilityMethodV1(checkout, keys, label, keys[0]);
    const discard = captureCapabilityMethodV1(checkout, keys, label, keys[1]);
    try { return bind(await open()); }
    catch {
      try { await discard(); } catch { /* fixed indeterminate remains the only result */ }
      return null;
    }
  } catch {
    return null;
  }
}

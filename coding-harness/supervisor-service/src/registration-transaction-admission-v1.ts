// SPDX-License-Identifier: MIT

import { ClosedJsonHashError } from './closed-json.js';
import { parseCanonicalRegistrationRequestV2 } from './registration-protocol-v2.js';
import { captureCapabilityMethodV1 } from './registration-transaction-boundary-v1.js';

export type RegistrationPeerConsumerV1 = (peer: unknown) => unknown;

export async function preclassifyRegistrationRequestV1(
  serializedRequest: string,
): Promise<boolean | null> {
  try { await parseCanonicalRegistrationRequestV2(serializedRequest); return true; }
  catch (error) {
    return error instanceof ClosedJsonHashError || !(error instanceof TypeError) ? null : false;
  }
}

export function captureRegistrationPeerConsumerV1(
  registry: unknown,
  operation: 'consumeRegistration' | 'consumeExactRecovery',
  label: string,
): RegistrationPeerConsumerV1 | null {
  try {
    return captureCapabilityMethodV1(
      registry, [operation], label, operation,
    ) as RegistrationPeerConsumerV1;
  } catch {
    return null;
  }
}

export function consumeRegistrationPeerV1(
  peer: unknown,
  consume: RegistrationPeerConsumerV1,
): boolean | null {
  if (typeof peer !== 'symbol') return false;
  try {
    const admitted = consume(peer);
    return typeof admitted === 'boolean' ? admitted : null;
  } catch {
    return null;
  }
}

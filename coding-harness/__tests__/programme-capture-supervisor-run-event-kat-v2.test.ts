// SPDX-License-Identifier: MIT

import {
  createHash,
  createPublicKey,
  verify,
} from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  programmeCaptureSupervisorRunEventSigningPayloadV2,
} from '../src/programme-capture-supervisor-run-event-codec-v2.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_SIGNING_DOMAIN_V2,
} from '../src/programme-capture-supervisor-run-event-contracts-v2.js';
import {
  TEST_SERVICE_PUBLIC_KEY_SPKI,
  signedEnvelope,
  validRunHistory,
} from './programme-capture-supervisor-run-event-v2-fixtures.js';

const PINNED_VECTORS: Record<string, Readonly<{
  eventDigest: string;
  signingPayloadSha256: string;
  signatureBase64Url: string;
  serializedEnvelopeSha256: string;
}>> = {
  'capture-attempt-start-committed-v2': {
    eventDigest: '38641ca12002bf39bb2bf7dd49905aca6076cc9e958f8da42e9475d02771bd6f',
    signingPayloadSha256: '07b6eada50e4cfd7ee55ddbf5aa1552b80bc20d7602deaff3b0e7f55f8e8b453',
    signatureBase64Url:
      'ht9O5NpDvotDVZmdfla6qgq4bfPA4XdzCVixFtA3WDItGNyy43NUAzYABl1Qtr2emhAYZrnljVQawItW40hnCg',
    serializedEnvelopeSha256:
      '8fd71b4b7ec6f40375606335853640d4ea46447b1ba8addc518ed08ee9ca1e37',
  },
  'capture-attempt-terminal-v2': {
    eventDigest: '53707acc0f948ffa42180d00d85a52defb98a22c75587f4086e57e7ff875c373',
    signingPayloadSha256: '47cfeeabd20ed31484cb3e4b8ace0c715dbde4d51f766b6b938b925de341d2c3',
    signatureBase64Url:
      'KwnFeLCiDz0JZmgER36nSPdaOzGEk3YjJTrk5S3hBIQ2sNjJFGfTWX3uIn7c1RuGADHy-fVW5aVFWmjxluxCDQ',
    serializedEnvelopeSha256:
      'da87b427e64005ebd2925e7bdb81fd9ee459f87b4b312870a318333981c50762',
  },
  'capture-final-witness-v2': {
    eventDigest: '2f847d9a29e286b8d1aee2bfca6893c6602e9fb505d67e0fdd9a65cc523a557b',
    signingPayloadSha256: 'e206b229c2f2b5541607190a36c23d443a076a9d4c2b2d7d2a2d39ba9c2c5bcd',
    signatureBase64Url:
      'HScDURwW_1fBaOHmEBQ-yZaSLxqSmZNtAP0HHglpyj-8qkXVi63drmjKUHJjoUUv4Zc8Gaq6qLJxNClZycTECg',
    serializedEnvelopeSha256:
      '2e09a23bd082e1e1f8fbffec220dc17c73064769021b06a20781268f78e0d8eb',
  },
  'capture-run-terminal-v2': {
    eventDigest: '2db130e6ed05c3bfba877c5cd55e507955632c32f615ed819cfd5e6755fe440e',
    signingPayloadSha256: '2464a03f684b9016aac21bce80f6355d8a93913a2486de3a620a262cde7e7bfc',
    signatureBase64Url:
      'tLUxqa3x5bKKep-f5JACDQ1WHlT44AK7d2b_gBvyJmbDtz1Q-LwT4yWwOX0UZLmlayMyCmGdCw9woILuT_5hCA',
    serializedEnvelopeSha256:
      '8511e567f73929bf122a829ed5c5e76eddda5e56448ae952647250f73a6fa192',
  },
  'claim-registered-v2': {
    eventDigest: '746bc20963bcf84817905c38b93ee67111ea162098b1db4ea35f4f10c1b49fff',
    signingPayloadSha256: 'b7b9fdf6d66a80c937d57b01810097e6818d106fd3e07d09eadba2a60ec0936d',
    signatureBase64Url:
      'VM2FRNCsAfEOZ67rCby5vLKzGvZRAW6kK2jj8UdOi1iVfh8gHDeqtCHyk1GwnS1lNIZH8D8r60bxtADnK3fnAA',
    serializedEnvelopeSha256:
      'ada70396e3e18bbd32f8bbe3b304e92e9d6cc1107cfb3719f140a4db6dd554d4',
  },
  'runner-lease-granted-v2': {
    eventDigest: 'd68392da0a048804d81ff58c76d706beb511ca33a99e6a16a5d03cf0a2e0486f',
    signingPayloadSha256: 'cdf47e3bf7001ee24eafaf8911b42d7845844722e44ebd3ff7e4d6ac80474d0a',
    signatureBase64Url:
      'W4REjrA4a74IAJ3FjHZYYI1BvqiFBwQL753YyKDdKYD52XIPlWBshEdt-J4ArZBp72Ti5ITBdiuvij1Sfyj4BA',
    serializedEnvelopeSha256:
      '7960d3847c783474649d75e32750a414f6eec27d302686733617be2175dcaced',
  },
};

describe('programme capture supervisor run-event V2 independent wire KAT', () => {
  it('pins event, signing, signature, and canonical-envelope bytes for all six kinds', () => {
    const eventsByKind = new Map([
      ...validRunHistory(),
      ...validRunHistory({ preStartTerminal: 'registration' }),
    ].map((event) => [event.eventKind, event]));
    const vectors: typeof PINNED_VECTORS = {};
    const publicKey = createPublicKey({
      key: TEST_SERVICE_PUBLIC_KEY_SPKI, format: 'der', type: 'spki',
    });
    for (const [kind, event] of [...eventsByKind.entries()].sort()) {
      const { eventDigest: _ignored, ...eventBody } = event;
      const independentEventDigest = sha256(canonicalOracle({
        domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,
        event: eventBody,
      }));
      const independentSigningPayload = canonicalOracle({
        domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_SIGNING_DOMAIN_V2,
        event,
      });
      const serializedEnvelope = signedEnvelope(event);
      const envelope = JSON.parse(serializedEnvelope) as any;
      expect(event.eventDigest).toBe(independentEventDigest);
      expect(programmeCaptureSupervisorRunEventSigningPayloadV2(event).toString('utf8'))
        .toBe(independentSigningPayload);
      expect(verify(
        null,
        Buffer.from(independentSigningPayload, 'utf8'),
        publicKey,
        Buffer.from(envelope.signature.valueBase64Url, 'base64url'),
      )).toBe(true);
      vectors[kind] = {
        eventDigest: independentEventDigest,
        signingPayloadSha256: sha256(independentSigningPayload),
        signatureBase64Url: envelope.signature.valueBase64Url,
        serializedEnvelopeSha256: sha256(serializedEnvelope),
      };
    }
    expect(vectors).toEqual(PINNED_VECTORS);
  });
});

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function canonicalOracle(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalOracle).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    const input = value as Record<string, unknown>;
    return `{${Object.keys(input).sort().map((key) =>
      `${JSON.stringify(key)}:${canonicalOracle(input[key])}`).join(',')}}`;
  }
  const encoded = JSON.stringify(value);
  if (encoded === undefined) throw new TypeError('KAT value is not JSON-serializable');
  return encoded;
}

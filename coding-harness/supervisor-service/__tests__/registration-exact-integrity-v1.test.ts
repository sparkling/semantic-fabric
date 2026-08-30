// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  decideSupervisorRegistrationV1,
  type AuthenticatedTransportPeerV1,
  type ExactCommittedResultReadV1,
  type SupervisorRegistrationDecisionPortsV1,
} from '../src/index.js';
import {
  CANONICAL_REQUEST, DIGEST, PROJECT, REGISTERED_RUN,
  canonical, canonicalPretty, coherentlyMutatedRegistrationEnvelope, exactStoredResult,
  registrationEnvelope, sha256Text,
} from './registration-fixtures.js';

const PEER = Symbol('authenticated-exact-integrity-peer') as AuthenticatedTransportPeerV1;

function exactOnly(exact: ExactCommittedResultReadV1): SupervisorRegistrationDecisionPortsV1 {
  const unexpected = async (): Promise<never> => { throw new Error('unexpected later read'); };
  return {
    mapAuthenticatedPeer: async () => ({ kind: 'mapped', project: PROJECT }),
    lookupExactCommittedResult: async () => exact,
    readActiveAuthorityHead: unexpected,
    readRequiredPredecessorReceipt: unexpected,
    readRunState: unexpected,
  };
}

async function expectRejected(exact: ExactCommittedResultReadV1): Promise<void> {
  const decision = await decideSupervisorRegistrationV1(
    CANONICAL_REQUEST, PEER, exactOnly(exact),
  );
  expect(decision).toMatchObject({
    decisionKind: 'indeterminate',
    response: { outcomeCode: 'transaction-resolution-unknown-v2' },
  });
}

function invalidSignatureEnvelope(): string {
  const envelope = JSON.parse(registrationEnvelope(201));
  envelope.signature.valueBase64Url = `${'A'.repeat(85)}B`;
  return canonicalPretty(envelope);
}

function forgedEventDigestEnvelope(): string {
  const envelope = JSON.parse(registrationEnvelope(201));
  envelope.event.eventDigest = sha256Text('forged-event-digest');
  return canonicalPretty(envelope);
}

function forgedResultDigest(): ExactCommittedResultReadV1 {
  const exact = structuredClone(exactStoredResult(201));
  const result = JSON.parse(exact.row.serializedResponse);
  result.resultDigest = sha256Text('forged-result-digest');
  const serialized = canonicalPretty(result);
  exact.row.serializedResponse = serialized;
  exact.row.serializedResponseSha256 = sha256Text(serialized);
  return exact as ExactCommittedResultReadV1;
}

function reorderedResult(): ExactCommittedResultReadV1 {
  const exact = structuredClone(exactStoredResult(201));
  const parsed = JSON.parse(exact.row.serializedResponse);
  const { resultDigest: _ignored, ...body } = parsed;
  const reordered = Object.fromEntries(Object.entries(body).reverse());
  const result = { ...reordered, resultDigest: sha256Text(canonical({
    domain: 'semantic-fabric/programme-capture/supervisor-service-result-digest-v2',
    result: reordered,
  })) };
  const serialized = canonicalPretty(result);
  exact.row.serializedResponse = serialized;
  exact.row.serializedResponseSha256 = sha256Text(serialized);
  return exact as ExactCommittedResultReadV1;
}

describe('exact registration recovery provenance V1', () => {
  it.each([
    ['current request as original', exactStoredResult(409, {}, {
      originalRegistrationRequestDigest: DIGEST.request,
    })],
    ['foreign original event', exactStoredResult(409, {}, {
      originalRegistrationEventDigest: sha256Text('foreign-registration-event'),
    })],
    ['zero original global sequence', exactStoredResult(409, {}, {
      originalRegistrationGlobalSequence: '0',
    })],
    ['non-prior original global sequence', exactStoredResult(409, {}, {
      originalRegistrationGlobalSequence: '2',
    })],
    ['missing changed-replay prior state', exactStoredResult(409, {}, {
      changedReplayPriorControllerStateHeadDigest: null,
    })],
    ['unexpected 201 changed-replay prior state', exactStoredResult(201, {}, {
      changedReplayPriorControllerStateHeadDigest: sha256Text('unexpected-prior-state'),
    })],
    ['foreign 201 original request', exactStoredResult(201, {}, {
      originalRegistrationRequestDigest: sha256Text('foreign-original-request'),
    })],
    ['foreign 201 original event', exactStoredResult(201, {}, {
      originalRegistrationEventDigest: sha256Text('foreign-original-event'),
    })],
    ['mismatched 201 original global sequence', exactStoredResult(201, {}, {
      originalRegistrationGlobalSequence: '2',
    })],
    ['forged evidence digest', exactStoredResult(409, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(409, (event) => {
        event.body.outcomeEvidenceDigest = sha256Text('forged-evidence');
      }),
    })],
    ['foreign changed-replay prior state', exactStoredResult(409, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(409, (event) => {
        event.priorControllerStateHeadDigest = sha256Text('foreign-prior-state');
      }),
    })],
    ['wrong immediate global predecessor', exactStoredResult(409, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(409, (event) => {
        event.previousGlobal.eventDigest = sha256Text('foreign-global-event');
      }),
    })],
    ['changed replay at global genesis', exactStoredResult(409, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(409, (event) => {
        event.globalSequence = '1';
        event.previousGlobal = { kind: 'authority-genesis', eventDigest: null,
          semanticReceiptDigest: sha256Text('false-genesis-receipt') };
      }),
    })],
    ['foreign run', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.runId = 'foreign_capture_run_20260830';
      }),
    })],
    ['foreign project authority', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.project.projectAuthorityDigest = sha256Text('foreign-project');
      }),
    })],
    ['registration resource transition', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.resourceTransition = { prohibited: true };
      }),
    })],
    ['zero service key epoch', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.service.keyEpoch = '0';
      }),
    })],
    ['malformed service key fingerprint', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.service.keyFingerprint = 'not-a-digest';
      }),
    })],
    ['noncanonical signature tail', exactStoredResult(201, {
      serializedEventEnvelope: invalidSignatureEnvelope(),
    })],
    ['forged inner event digest', exactStoredResult(201, {
      serializedEventEnvelope: forgedEventDigestEnvelope(),
    }, { originalRegistrationEventDigest: sha256Text('forged-event-digest') })],
    ['foreign changed-replay registration reference', exactStoredResult(409, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(409, (event) => {
        event.body.registrationEventDigest = sha256Text('foreign-registration-reference');
      }),
    })],
    ['reordered changed-replay body', exactStoredResult(409, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(409, (event) => {
        event.body = Object.fromEntries(Object.entries(event.body).reverse());
      }),
    })],
    ['foreign result identity', exactStoredResult(201, {
      resultKind: 'foreign-registration-result-v2',
    })],
    ['foreign result authority', exactStoredResult(201, {
      authority: 'production-authority',
    })],
    ['positive result authority flag', exactStoredResult(201, {
      databaseCommitVerified: true,
    })],
    ['reordered result wrapper', reorderedResult()],
    ['forged inner result digest', forgedResultDigest()],
  ])('rejects %s despite internally rehashed result bytes', async (_label, exact) => {
    await expectRejected(exact as ExactCommittedResultReadV1);
  });

  it('accepts a changed replay after an unrelated interleaved global event', async () => {
    const serializedEventEnvelope = coherentlyMutatedRegistrationEnvelope(409, (event) => {
      event.globalSequence = '3';
      event.previousGlobal.eventDigest = sha256Text('interleaved-global-event');
    });
    const exact = exactStoredResult(409, { serializedEventEnvelope }, {
      originalRegistrationEventDigest: REGISTERED_RUN.registrationEventDigest,
      originalRegistrationGlobalSequence: '1',
    });

    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, exactOnly(exact as ExactCommittedResultReadV1),
    );

    expect(decision.decisionKind).toBe('exact-response');
    if (decision.decisionKind !== 'exact-response') return;
    expect(decision.response.body).toBe(exact.row.serializedResponse);
  });

  it('accepts a first registration recovered after an interleaved global event', async () => {
    const serializedEventEnvelope = coherentlyMutatedRegistrationEnvelope(201, (event) => {
      event.globalSequence = '2';
      event.previousGlobal = { kind: 'semantic-event',
        eventDigest: sha256Text('interleaved-before-registration'),
        semanticReceiptDigest: sha256Text('interleaved-registration-receipt') };
    });
    const exact = exactStoredResult(201, { serializedEventEnvelope }, {
      originalRegistrationGlobalSequence: '2',
    });
    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, exactOnly(exact as ExactCommittedResultReadV1),
    );
    expect(decision.decisionKind).toBe('exact-response');
  });
});

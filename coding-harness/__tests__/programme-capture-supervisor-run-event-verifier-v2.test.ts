// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
  programmeCaptureSupervisorAuthorityGenesisHeadDigestV2,
  parseProgrammeCaptureSupervisorAuthorityConfigurationV2,
  serializeProgrammeCaptureSupervisorAuthorityConfigurationV2,
} from '../src/programme-capture-supervisor-authority-config-v2.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_DIGEST_DOMAIN_V2,
} from '../src/programme-capture-supervisor-authority-transition-v2.js';
import {
  verifyProgrammeCaptureSupervisorRunEventEnvelopeV2,
  verifyProgrammeCaptureSupervisorRunHistoryV2,
} from '../src/programme-capture-supervisor-run-event-verifier-v2.js';
import {
  TEST_SERVICE_PUBLIC_KEY_SPKI,
  digest,
  signedEnvelope,
  validAuthorityConfiguration,
  validRunHistory,
  withEventDigest,
} from './programme-capture-supervisor-run-event-v2-fixtures.js';
import { digestValue } from '../src/receipts.js';

describe('programme capture supervisor signed run-event V2 verification', () => {
  it('verifies a canonical service signature and only supplied structural references', () => {
    const event = validRunHistory()[0];
    const validation = verifyProgrammeCaptureSupervisorRunEventEnvelopeV2(
      eventContext(event),
    );
    expect(validation).toMatchObject({
      eventDigest: event.eventDigest,
      canonicalEnvelopeVerified: true,
      eventDigestVerified: true,
      serviceSignatureVerified: true,
      suppliedAuthorityConfigurationMatched: true,
      authorityAdjacencyVerified: true,
      suppliedAuthorityHeadMatched: true,
      suppliedGlobalReferencesMatched: true,
      suppliedRunReferencesMatched: true,
      externalAdministrationVerified: false,
      authorityActivationVerified: false,
      priorGlobalEventVerified: false,
      globalOrderVerified: false,
      resourceHighWaterVerified: false,
      resourceFencingVerified: false,
      stateTransitionAuthorized: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
    });
    expect(validation.serializedEnvelopeDigest).toMatch(/^[a-f0-9]{64}$/);
    expect(Object.isFrozen(validation)).toBe(true);
    expect('leafIndex' in validation).toBe(false);
  });

  it('rejects signature, trust pin, authority head, and supplied-reference substitutions', () => {
    const event = validRunHistory()[0];
    const base = eventContext(event) as any;
    const envelope = JSON.parse(base.serializedEnvelope) as any;
    const priorLast = envelope.signature.valueBase64Url.at(-1);
    envelope.signature.valueBase64Url = `${envelope.signature.valueBase64Url.slice(0, -1)}${
      priorLast === 'A' ? 'B' : 'A'
    }`;
    const changedSignature = `${JSON.stringify(envelope, null, 2)}\n`;
    const mutations = [
      { ...base, serializedEnvelope: changedSignature },
      { ...base, activeAuthorityHeadDigest: event.authorityHead.configurationDigest },
      { ...base, expectedGlobalSequence: '2' },
      { ...base, expectedRunId: 'another_run_20260829' },
      { ...base, trustedServicePublicKeySpkiDer: new Uint8Array(44) },
      { ...base, expectedPriorControllerStateHeadDigest: event.eventDigest },
    ];
    for (const mutation of mutations) {
      expect(() => verifyProgrammeCaptureSupervisorRunEventEnvelopeV2(mutation)).toThrow();
    }
  });

  it('verifies a complete signed history and derives the same deterministic state', () => {
    const events = validRunHistory();
    const validation = verifyProgrammeCaptureSupervisorRunHistoryV2(
      historyContext(events),
    );
    expect(validation.state).toMatchObject({
      phase: 'final-witnessed',
      attempts: '1',
      runSpent: true,
      finalWitnessRequired: false,
      runAdjacencyVerified: true,
      resourceAdjacencyVerified: true,
    });
    expect(validation).toMatchObject({
      canonicalEnvelopesVerified: true,
      serviceSignaturesVerified: true,
      suppliedResourcePredecessorsMatched: true,
      fullAuthorityHistoryVerified: false,
      priorSemanticReceiptVerified: false,
      publicCommitmentVerified: false,
      semanticWitnessQuorumVerified: false,
    });
    expect(validation.serializedEnvelopeDigests).toHaveLength(5);
    expect(Object.isFrozen(validation.serializedEnvelopeDigests)).toBe(true);
  });

  it('accepts one supplied configuration-transition adjacency without claiming activation', () => {
    const predecessor = validAuthorityConfiguration();
    const predecessorHead = programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(predecessor);
    const successorBody = {
      ...structuredClone(predecessor),
      configurationEpoch: '1',
      predecessor: {
        kind: 'configuration-head',
        configurationDigest: predecessor.configurationDigest,
        headDigest: predecessorHead,
      },
    } as any;
    delete successorBody.configurationDigest;
    const successor = parseProgrammeCaptureSupervisorAuthorityConfigurationV2({
      ...successorBody,
      configurationDigest: digestValue({
        domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
        configuration: successorBody,
      }),
    });
    const transitionBody = {
      schemaVersion: 2,
      transactionKind: 'programme-capture-v2',
      recordKind: 'supervisor-authority-transition-v2',
      authority: 'development-only-no-promotion',
      globalSequence: '1',
      predecessorHead: {
        configurationEpoch: '0',
        configurationDigest: predecessor.configurationDigest,
        headDigest: predecessorHead,
      },
      successorConfiguration: {
        configurationEpoch: '1', configurationDigest: successor.configurationDigest,
      },
      verificationScope: 'configuration-adjacency-only',
      externalAdministrationVerified: false,
      deploymentAttestationVerified: false,
      checkpointWitnessQuorumVerified: false,
      semanticWitnessQuorumVerified: false,
      stateTransitionAuthorized: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
    };
    const transition = {
      ...transitionBody,
      transitionDigest: digestValue({
        domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_TRANSITION_DIGEST_DOMAIN_V2,
        transition: transitionBody,
      }),
    };
    const event = withEventDigest({
      ...structuredClone(validRunHistory()[0]),
      authorityHead: {
        configurationEpoch: '1',
        configurationDigest: successor.configurationDigest,
        headDigest: transition.transitionDigest,
      },
      globalSequence: '2',
      previousGlobal: {
        kind: 'semantic-event',
        eventDigest: transition.transitionDigest,
        semanticReceiptDigest: digest('transition-semantic-receipt'),
      },
    });
    const validation = verifyProgrammeCaptureSupervisorRunEventEnvelopeV2({
      serializedEnvelope: signedEnvelope(event),
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(successor),
      activeAuthorityHeadDigest: transition.transitionDigest,
      activation: {
        kind: 'transition-adjacency',
        transition,
        transitionContext: {
          predecessorConfiguration: predecessor,
          expectedPredecessorHeadDigest: predecessorHead,
          expectedGlobalSequence: '1',
          successorConfiguration: successor,
        },
      },
      trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
      expectedRunId: (event as any).runId,
      expectedGlobalSequence: '2',
      expectedPreviousGlobal: (event as any).previousGlobal,
      expectedRunSequence: '0',
      expectedPreviousRun: { kind: 'run-genesis', eventDigest: null },
      expectedPriorControllerStateHeadDigest: (event as any).priorControllerStateHeadDigest,
    });
    expect(validation).toMatchObject({
      configurationEpoch: '1',
      activeAuthorityHeadDigest: transition.transitionDigest,
      authorityAdjacencyVerified: true,
      authorityActivationVerified: false,
      fullAuthorityHistoryVerified: false,
    });
  });

  it('rejects changed global references and stale supplied resource state', () => {
    const events = validRunHistory();
    const changedGlobal = structuredClone(historyContext(events)) as any;
    changedGlobal.entries[2].expectedPreviousGlobal.eventDigest = events[0].eventDigest;
    expect(() => verifyProgrammeCaptureSupervisorRunHistoryV2(changedGlobal)).toThrow();

    const staleResource = structuredClone(historyContext(events)) as any;
    staleResource.expectedLeaseResourcePredecessors.members[0].priorState = {
      kind: 'resource-event', eventDigest: events[0].eventDigest, fence: '1',
    };
    expect(() => verifyProgrammeCaptureSupervisorRunHistoryV2(staleResource)).toThrow();
  });

  it('rejects Proxy contexts before invoking traps', () => {
    let trapCalls = 0;
    const proxy = new Proxy(eventContext(validRunHistory()[0]), {
      getPrototypeOf() { trapCalls += 1; return Object.prototype; },
    });
    expect(() => verifyProgrammeCaptureSupervisorRunEventEnvelopeV2(proxy)).toThrow(/Proxy/);
    expect(trapCalls).toBe(0);
  });
});

function eventContext(event: ReturnType<typeof validRunHistory>[number]) {
  const configuration = validAuthorityConfiguration();
  return {
    serializedEnvelope: signedEnvelope(event),
    serializedAuthorityConfiguration:
      serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(configuration),
    activeAuthorityHeadDigest:
      programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(configuration),
    activation: { kind: 'genesis' },
    trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
    expectedRunId: event.runId,
    expectedGlobalSequence: event.globalSequence,
    expectedPreviousGlobal: event.previousGlobal,
    expectedRunSequence: event.runSequence,
    expectedPreviousRun: event.previousRun,
    expectedPriorControllerStateHeadDigest: event.priorControllerStateHeadDigest,
  };
}

function historyContext(events: ReturnType<typeof validRunHistory>) {
  const configuration = validAuthorityConfiguration();
  const leaseResource = events.find(({ eventKind }) =>
    eventKind === 'runner-lease-granted-v2')?.resourceTransition;
  return {
    serializedAuthorityConfiguration:
      serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(configuration),
    activeAuthorityHeadDigest:
      programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(configuration),
    activation: { kind: 'genesis' },
    trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
    expectedRunId: events[0].runId,
    expectedInitialControllerStateHeadDigest: events[0].priorControllerStateHeadDigest,
    expectedLeaseResourcePredecessors: leaseResource ? {
      runnerEnrollmentRecordDigest: leaseResource.runnerEnrollmentRecordDigest,
      physicalParentId: leaseResource.physicalParentId,
      members: leaseResource.members,
    } : null,
    entries: events.map((event) => ({
      serializedEnvelope: signedEnvelope(event),
      expectedGlobalSequence: event.globalSequence,
      expectedPreviousGlobal: event.previousGlobal,
    })),
  };
}

// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  programmeCaptureRunClaimKeyDigestV1,
} from '../src/programme-capture-claim-record-v1.js';
import {
  programmeCaptureSupervisorAuthorityGenesisHeadDigestV2,
  serializeProgrammeCaptureSupervisorAuthorityConfigurationV2,
} from '../src/programme-capture-supervisor-authority-config-v2.js';
import {
  verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2,
} from '../src/programme-capture-supervisor-service-client-v2.js';
import {
  buildProgrammeCaptureSupervisorRegistrationRequestV2,
  programmeCaptureSupervisorRegistrationChangedReplayEvidenceDigestV2,
  serializeProgrammeCaptureSupervisorRegistrationRequestV2,
} from '../src/programme-capture-supervisor-registration-request-v2.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_DOMAIN_V2,
} from '../src/programme-capture-supervisor-run-event-contracts-v2.js';
import {
  buildProgrammeCaptureSupervisorServiceResultV2,
  serializeProgrammeCaptureSupervisorServiceResultV2,
} from '../src/programme-capture-supervisor-service-result-v2.js';
import {
  digest,
  signedEnvelope,
  TEST_SERVICE_PUBLIC_KEY_SPKI,
  validAuthorityConfiguration,
  validRunHistory,
  withEventDigest,
} from './programme-capture-supervisor-run-event-v2-fixtures.js';

function fixture() {
  const configuration = validAuthorityConfiguration();
  const original = validRunHistory()[0];
  const request = buildProgrammeCaptureSupervisorRegistrationRequestV2({
    authorityHead: original.authorityHead,
    project: original.project,
    runId: original.runId,
    expectedRegistration: {
      priorControllerStateHeadDigest: digest('controller-state-genesis'),
    },
    claim: {
      claimKeyDigest: programmeCaptureRunClaimKeyDigestV1({
        projectAuthorityDigest: original.project.projectAuthorityDigest,
        runId: original.runId,
      }),
      claimDigest: digest('client-claim'),
      rootedClaimValidationDigest: digest('client-rooted-validation'),
    },
  });
  const event = withEventDigest({
    ...original,
    semanticRequestDigest: request.semanticRequestDigest,
    body: request.claim,
  });
  const serializedEventEnvelope = signedEnvelope(event);
  const result = buildProgrammeCaptureSupervisorServiceResultV2({
    semanticRequestDigest: request.semanticRequestDigest,
    serializedEventEnvelope,
  });
  return {
    configuration,
    event,
    request,
    serializedEventEnvelope,
    serializedRequest: serializeProgrammeCaptureSupervisorRegistrationRequestV2(request),
    serializedResult: serializeProgrammeCaptureSupervisorServiceResultV2(result),
  };
}

function acceptedSuppliedReferences() {
  return {
    expectedRunId: 'capture_run_20260829',
    expectedGlobalSequence: '1',
    expectedPreviousGlobal: {
      kind: 'authority-genesis' as const,
      eventDigest: null,
      semanticReceiptDigest: digest('semantic-genesis'),
    },
    expectedRunSequence: '0',
    expectedPreviousRun: { kind: 'run-genesis' as const, eventDigest: null },
    expectedPriorControllerStateHeadDigest: digest('controller-state-genesis'),
    expectedOriginalRegistrationRequestDigest: null,
    expectedOriginalRegistrationEventDigest: null,
  };
}

function changedReplayFixture(options: Readonly<{
  exactDuplicateEvidence?: boolean;
}> = {}) {
  const configuration = validAuthorityConfiguration();
  const [originalRegistration, terminalTemplate] = validRunHistory({
    preStartTerminal: 'registration',
  });
  const request = buildProgrammeCaptureSupervisorRegistrationRequestV2({
    authorityHead: originalRegistration.authorityHead,
    project: originalRegistration.project,
    runId: originalRegistration.runId,
    expectedRegistration: {
      priorControllerStateHeadDigest: digest('controller-state-genesis'),
    },
    claim: {
      claimKeyDigest: programmeCaptureRunClaimKeyDigestV1({
        projectAuthorityDigest: originalRegistration.project.projectAuthorityDigest,
        runId: originalRegistration.runId,
      }),
      claimDigest: digest('changed-client-claim'),
      rootedClaimValidationDigest: digest('changed-client-rooted-validation'),
    },
  });
  const outcomeEvidenceDigest =
    programmeCaptureSupervisorRegistrationChangedReplayEvidenceDigestV2({
      originalRegistrationRequestDigest: options.exactDuplicateEvidence === true
        ? request.semanticRequestDigest : originalRegistration.semanticRequestDigest,
      originalRegistrationEventDigest: originalRegistration.eventDigest,
      changedRegistrationRequestDigest: request.semanticRequestDigest,
      project: request.project,
      authorityHead: request.authorityHead,
    });
  const event = withEventDigest({
    ...terminalTemplate,
    semanticRequestDigest: request.semanticRequestDigest,
    body: {
      ...(terminalTemplate.body as Record<string, unknown>),
      outcomeCode: 'registration-changed-replay-v2',
      outcomeEvidenceDigest,
    },
  });
  const serializedEventEnvelope = signedEnvelope(event);
  const result = buildProgrammeCaptureSupervisorServiceResultV2({
    semanticRequestDigest: request.semanticRequestDigest,
    serializedEventEnvelope,
  });
  const priorControllerStateHeadDigest = sha256(canonicalOracle({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_DOMAIN_V2,
    priorControllerStateHeadDigest: digest('controller-state-genesis'),
    eventDigest: originalRegistration.eventDigest,
  }));
  return {
    configuration,
    request,
    event,
    serializedRequest: serializeProgrammeCaptureSupervisorRegistrationRequestV2(request),
    serializedResult: serializeProgrammeCaptureSupervisorServiceResultV2(result),
    suppliedReferences: {
      expectedRunId: 'capture_run_20260829',
      expectedGlobalSequence: '2',
      expectedPreviousGlobal: {
        kind: 'semantic-event' as const,
        eventDigest: originalRegistration.eventDigest,
        semanticReceiptDigest: digest('semantic-receipt:1'),
      },
      expectedRunSequence: '1',
      expectedPreviousRun: {
        kind: 'run-event' as const, eventDigest: originalRegistration.eventDigest,
      },
      expectedPriorControllerStateHeadDigest: priorControllerStateHeadDigest,
      expectedOriginalRegistrationRequestDigest: options.exactDuplicateEvidence === true
        ? request.semanticRequestDigest : originalRegistration.semanticRequestDigest,
      expectedOriginalRegistrationEventDigest: originalRegistration.eventDigest,
    },
  };
}

describe('programme capture supervisor verify-only service client V2', () => {
  it('verifies canonical request/result/signature bindings with independent references', () => {
    const value = fixture();
    const validation =
      verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
      serializedRequest: value.serializedRequest,
      serializedResult: value.serializedResult,
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(value.configuration),
      activeAuthorityHeadDigest:
        programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(value.configuration),
      activation: { kind: 'genesis' },
      trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
      ...acceptedSuppliedReferences(),
    });

    expect(validation.canonicalRequestVerified).toBe(true);
    expect(validation.canonicalResultVerified).toBe(true);
    expect(validation.serviceSignatureVerified).toBe(true);
    expect(validation.semanticRequestBindingVerified).toBe(true);
    expect(validation.registrationOutcome).toBe('accepted');
    expect(validation.responseStatus).toBe(201);
    expect(validation.canonicalResponseMetadataBindingVerified).toBe(true);
    expect(validation.changedReplayEvidenceBindingVerified).toBe(false);
    expect(validation.projectAuthenticationVerified).toBe(false);
    expect(validation.rootedClaimVerified).toBe(false);
    expect(validation.globalOrderVerified).toBe(false);
    expect(validation.fullAuthorityHistoryVerified).toBe(false);
    expect(validation.resourceAdjacencyVerified).toBe(false);
    expect(validation.resourceHighWaterVerified).toBe(false);
    expect(validation.runnerAdmissionVerified).toBe(false);
    expect(validation.hostEvidenceVerified).toBe(false);
    expect(validation.databaseCommitVerified).toBe(false);
    expect(validation.stateTransitionAuthorized).toBe(false);
    expect(validation.validationDigest).toBe(
      'f8e274e85166069a1775ff2291aa401add4a52422c7d6cdc7d19d153bca20909',
    );
    expect(Object.isFrozen(validation)).toBe(true);
  });

  it('verifies a typed changed-replay terminal with independently supplied originals', () => {
    const value = changedReplayFixture();
    const validation =
      verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
        serializedRequest: value.serializedRequest,
        serializedResult: value.serializedResult,
        serializedAuthorityConfiguration:
          serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(value.configuration),
        activeAuthorityHeadDigest:
          programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(value.configuration),
        activation: { kind: 'genesis' },
        trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
        ...value.suppliedReferences,
      });

    expect(validation.registrationOutcome).toBe('changed-replay');
    expect(validation.responseStatus).toBe(409);
    expect(validation.changedReplayEvidenceBindingVerified).toBe(true);
    expect(validation.serviceSignatureVerified).toBe(true);
    expect(validation.databaseCommitVerified).toBe(false);
  });

  it('rejects changed-replay evidence or original references that are not exact', () => {
    const value = changedReplayFixture();
    const base = {
      serializedRequest: value.serializedRequest,
      serializedResult: value.serializedResult,
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(value.configuration),
      activeAuthorityHeadDigest:
        programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(value.configuration),
      activation: { kind: 'genesis' as const },
      trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
      ...value.suppliedReferences,
    };
    expect(() =>
      verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
        ...base, expectedOriginalRegistrationRequestDigest: digest('wrong-original-request'),
      })).toThrow(/CHANGED_REPLAY_BINDING_MISMATCH/);
    expect(() =>
      verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
        ...base, expectedOriginalRegistrationEventDigest: digest('wrong-original-event'),
      })).toThrow(/CHANGED_REPLAY_BINDING_MISMATCH/);
  });

  it('rejects exact-request, non-first-terminal, and wrong-predecessor changed replay', () => {
    const exact = changedReplayFixture({ exactDuplicateEvidence: true });
    const exactBase = {
      serializedRequest: exact.serializedRequest,
      serializedResult: exact.serializedResult,
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(exact.configuration),
      activeAuthorityHeadDigest:
        programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(exact.configuration),
      activation: { kind: 'genesis' as const },
      trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
      ...exact.suppliedReferences,
    };
    expect(() =>
      verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2(exactBase))
      .toThrow(/CHANGED_REPLAY_BINDING_MISMATCH/);

    const value = changedReplayFixture();
    const base = {
      serializedRequest: value.serializedRequest,
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(value.configuration),
      activeAuthorityHeadDigest:
        programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(value.configuration),
      activation: { kind: 'genesis' as const },
      trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
      ...value.suppliedReferences,
    };
    const mutants = [
      {
        event: withEventDigest({ ...value.event, runSequence: '2' }),
        supplied: { expectedRunSequence: '2' },
      },
      {
        event: withEventDigest({
          ...value.event,
          previousRun: { kind: 'run-event', eventDigest: digest('wrong-predecessor') },
        }),
        supplied: {
          expectedPreviousRun: {
            kind: 'run-event' as const, eventDigest: digest('wrong-predecessor'),
          },
        },
      },
    ];
    for (const mutant of mutants) {
      const result = buildProgrammeCaptureSupervisorServiceResultV2({
        semanticRequestDigest: value.request.semanticRequestDigest,
        serializedEventEnvelope: signedEnvelope(mutant.event),
      });
      expect(() =>
        verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
          ...base,
          ...mutant.supplied,
          serializedResult: serializeProgrammeCaptureSupervisorServiceResultV2(result),
        })).toThrow(/CHANGED_REPLAY_BINDING_MISMATCH/);
    }
  });

  it('rejects another request, response mutation, and supplied-reference mismatch', () => {
    const value = fixture();
    const base = {
      serializedRequest: value.serializedRequest,
      serializedResult: value.serializedResult,
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(value.configuration),
      activeAuthorityHeadDigest:
        programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(value.configuration),
      activation: { kind: 'genesis' as const },
      trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
      ...acceptedSuppliedReferences(),
    };

    const otherRequest = buildProgrammeCaptureSupervisorRegistrationRequestV2({
      authorityHead: value.event.authorityHead,
      project: value.event.project,
      runId: value.event.runId,
      expectedRegistration: {
        priorControllerStateHeadDigest: digest('controller-state-genesis'),
      },
      claim: {
        claimKeyDigest: programmeCaptureRunClaimKeyDigestV1({
          projectAuthorityDigest: value.event.project.projectAuthorityDigest,
          runId: value.event.runId,
        }),
        claimDigest: digest('other-client-claim'),
        rootedClaimValidationDigest: digest('other-client-validation'),
      },
    });
    expect(() => verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
      ...base,
      serializedRequest: serializeProgrammeCaptureSupervisorRegistrationRequestV2(otherRequest),
    })).toThrow(/REQUEST_BINDING_MISMATCH/);
    expect(() => verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
      ...base, expectedGlobalSequence: '2',
    })).toThrow(/SUPPLIED_REFERENCE_MISMATCH/);
    expect(() => verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
      ...base, serializedResult: value.serializedResult.replace('ed25519', 'ed25518'),
    })).toThrow();
  });

  it('structurally carries but cryptographically rejects a changed signature', () => {
    const value = fixture();
    const result = buildProgrammeCaptureSupervisorServiceResultV2({
      semanticRequestDigest: value.request.semanticRequestDigest,
      serializedEventEnvelope: mutateEnvelopeSignature(value.serializedEventEnvelope),
    });
    expect(result.serviceSignatureVerified).toBe(false);
    expect(() =>
      verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
        serializedRequest: value.serializedRequest,
        serializedResult: serializeProgrammeCaptureSupervisorServiceResultV2(result),
        serializedAuthorityConfiguration:
          serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(value.configuration),
        activeAuthorityHeadDigest:
          programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(value.configuration),
        activation: { kind: 'genesis' },
        trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
        ...acceptedSuppliedReferences(),
      })).toThrow(/SIGNATURE/);
  });

  it('rejects response project and authority-head substitutions', () => {
    const value = fixture();
    for (const event of [
      withEventDigest({
        ...value.event,
        project: { ...value.event.project, principalId: 'another_project_principal' },
      }),
      withEventDigest({
        ...value.event,
        authorityHead: { ...value.event.authorityHead, headDigest: digest('another-head') },
      }),
    ]) {
      const result = buildProgrammeCaptureSupervisorServiceResultV2({
        semanticRequestDigest: value.request.semanticRequestDigest,
        serializedEventEnvelope: signedEnvelope(event),
      });
      expect(() =>
        verifyProgrammeCaptureSupervisorRegistrationResultWithSuppliedReferencesV2({
          serializedRequest: value.serializedRequest,
          serializedResult: serializeProgrammeCaptureSupervisorServiceResultV2(result),
          serializedAuthorityConfiguration:
            serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(value.configuration),
          activeAuthorityHeadDigest:
            programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(value.configuration),
          activation: { kind: 'genesis' },
          trustedServicePublicKeySpkiDer: TEST_SERVICE_PUBLIC_KEY_SPKI,
          ...acceptedSuppliedReferences(),
        })).toThrow(/REGISTRATION_INTENT_BINDING_MISMATCH/);
    }
  });
});

function mutateEnvelopeSignature(serialized: string): string {
  const envelope = JSON.parse(serialized) as {
    signature: { valueBase64Url: string };
  };
  const signature = envelope.signature.valueBase64Url;
  envelope.signature.valueBase64Url = `${signature[0] === 'A' ? 'B' : 'A'}${signature.slice(1)}`;
  return `${JSON.stringify(envelope, null, 2)}\n`;
}

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
  if (encoded === undefined) throw new TypeError('oracle value is not JSON-serializable');
  return encoded;
}

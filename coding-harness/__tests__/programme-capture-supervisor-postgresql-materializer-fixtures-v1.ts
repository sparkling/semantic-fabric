// SPDX-License-Identifier: MIT

import {
  programmeCaptureRunClaimKeyDigestV1,
} from '../src/programme-capture-claim-record-v1.js';
import {
  programmeCaptureSupervisorAuthorityGenesisHeadDigestV2,
  serializeProgrammeCaptureSupervisorAuthorityConfigurationV2,
} from '../src/programme-capture-supervisor-authority-config-v2.js';
import {
  buildProgrammeCaptureSupervisorRegistrationRequestV2,
  serializeProgrammeCaptureSupervisorRegistrationRequestV2,
} from '../src/programme-capture-supervisor-registration-request-v2.js';
import {
  buildProgrammeCaptureSupervisorRunEventV2,
} from '../src/programme-capture-supervisor-run-event-builder-v2.js';
import {
  TEST_SERVICE_PUBLIC_KEY_SPKI,
  digest,
  signedEnvelope,
  validAuthorityConfiguration,
} from './programme-capture-supervisor-run-event-v2-fixtures.js';
import {
  decideSupervisorRegistrationV1,
} from '../supervisor-service/src/registration-decision-v1.js';
import type {
  AuthenticatedTransportPeerV1,
  SupervisorRegistrationDecisionPortsV1,
} from '../supervisor-service/src/registration-ports-v1.js';

const PROJECT_SCOPE_ROLE = 'sf_supervisor_project_scope_v1';
const RUN_ID = 'capture_run_20260829';
const PEER = Symbol('materializer-interop-peer') as AuthenticatedTransportPeerV1;

export async function genesisMaterializerFixtureV1() {
  const configuration = validAuthorityConfiguration();
  const serializedConfiguration =
    serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(configuration);
  const authorityHead = {
    configurationEpoch: configuration.configurationEpoch,
    configurationDigest: configuration.configurationDigest,
    headDigest: programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(configuration),
  };
  const request = buildProgrammeCaptureSupervisorRegistrationRequestV2({
    authorityHead,
    project: {
      projectAuthorityDigest: configuration.project.projectAuthorityDigest,
      principalId: configuration.project.principal.principalId,
    },
    runId: RUN_ID,
    expectedRegistration: {
      priorControllerStateHeadDigest: digest('controller-state-genesis'),
    },
    claim: {
      claimKeyDigest: programmeCaptureRunClaimKeyDigestV1({
        projectAuthorityDigest: configuration.project.projectAuthorityDigest,
        runId: RUN_ID,
      }),
      claimDigest: digest('materializer-interop-claim'),
      rootedClaimValidationDigest: digest('materializer-interop-rooted-claim'),
    },
  });
  const serializedRequest = serializeProgrammeCaptureSupervisorRegistrationRequestV2(request);
  const project = {
    projectAuthorityDigest: configuration.project.projectAuthorityDigest,
    principalId: configuration.project.principal.principalId,
    authenticationPolicyDigest: configuration.project.authenticationPolicyDigest,
  };
  const semanticReceiptDigest = digest('semantic-genesis');
  const ports: SupervisorRegistrationDecisionPortsV1 = {
    mapAuthenticatedPeer: async () => ({ kind: 'mapped', project }),
    lookupExactCommittedResult: async () => ({ kind: 'absent' }),
    readActiveAuthorityHead: async () => ({
      kind: 'active', project, authorityHead, expectedNextGlobalSequence: '1',
      requiredPredecessor: { kind: 'authority-genesis', eventDigest: null },
    }),
    readRequiredPredecessorReceipt: async () => ({
      kind: 'ready',
      previousGlobal: {
        kind: 'authority-genesis', eventDigest: null, semanticReceiptDigest,
      },
    }),
    readRunState: async () => ({ kind: 'absent' }),
  };
  const decision = await decideSupervisorRegistrationV1(serializedRequest, PEER, ports);
  if (decision.decisionKind !== 'append-registration-candidate') {
    throw new Error('expected registration candidate');
  }
  const candidate = decision.candidate as any;
  const expectedEvent = buildProgrammeCaptureSupervisorRunEventV2({
    eventKind: candidate.candidateKind,
    authorityHead: candidate.authorityHead,
    service: {
      principalId: configuration.service.principal.principalId,
      keyEpoch: configuration.service.principal.keyEpoch,
      keyFingerprint: configuration.service.principal.keyFingerprint,
    },
    project: {
      projectAuthorityDigest: candidate.project.projectAuthorityDigest,
      principalId: candidate.project.principalId,
    },
    runId: candidate.request.runId,
    semanticRequestDigest: candidate.request.semanticRequestDigest,
    globalSequence: candidate.expectedNextGlobalSequence,
    runSequence: candidate.runSequence,
    previousGlobal: candidate.previousGlobal,
    previousRun: candidate.previousRun,
    priorControllerStateHeadDigest: candidate.priorControllerStateHeadDigest,
    resourceTransition: candidate.resourceTransition,
    body: candidate.body,
  });
  const projectDigest = digestBytes(configuration.project.projectAuthorityDigest);
  const configurationDigest = digestBytes(configuration.configurationDigest);
  const genesisHead = digestBytes(authorityHead.headDigest);
  const lockedSnapshots = {
    lockedConfiguration: {
      projectAuthorityDigest: projectDigest,
      projectScopeRole: PROJECT_SCOPE_ROLE,
      configurationEpoch: configuration.configurationEpoch,
      configurationDigest,
      genesisAuthorityHeadDigest: genesisHead,
      serializedConfiguration: Buffer.from(serializedConfiguration, 'utf8'),
      serializedConfigurationSha256: digestBytes(rawSha256(serializedConfiguration)),
      projectPrincipalId: configuration.project.principal.principalId,
      projectAuthenticationPolicyDigest:
        digestBytes(configuration.project.authenticationPolicyDigest),
      servicePrincipalId: configuration.service.principal.principalId,
      serviceKeyEpoch: configuration.service.principal.keyEpoch,
      serviceKeyFingerprint: digestBytes(configuration.service.principal.keyFingerprint),
      serviceSigningSpkiDer: Buffer.from(TEST_SERVICE_PUBLIC_KEY_SPKI),
      genesisSemanticReceiptDigest: digestBytes(semanticReceiptDigest),
    },
    lockedAuthorityState: {
      projectAuthorityDigest: Buffer.from(projectDigest),
      projectScopeRole: PROJECT_SCOPE_ROLE,
      singletonKey: true,
      activeConfigurationEpoch: configuration.configurationEpoch,
      activeConfigurationDigest: Buffer.from(configurationDigest),
      authorityHeadDigest: Buffer.from(genesisHead),
      lastGlobalSequence: '0',
      nextGlobalSequence: '1',
      lastEventDigest: null,
    },
    lockedPredecessorReceipt: {
      kind: 'authority-genesis',
      projectAuthorityDigest: Buffer.from(projectDigest),
      projectScopeRole: PROJECT_SCOPE_ROLE,
      configurationEpoch: configuration.configurationEpoch,
      configurationDigest: Buffer.from(configurationDigest),
      semanticReceiptDigest: digestBytes(semanticReceiptDigest),
    },
    lockedRunState: {
      kind: 'absent',
      projectAuthorityDigest: Buffer.from(projectDigest),
      projectScopeRole: PROJECT_SCOPE_ROLE,
      runId: RUN_ID,
    },
  };
  return { candidate: decision.candidate, configuration, expectedEvent,
    lockedSnapshots, request, serializedConfiguration, serializedRequest };
}

export async function changedReplayMaterializerFixtureV1() {
  const base = await genesisMaterializerFixtureV1();
  const originalRequestDigest = digest('materializer-original-request');
  const originalRequestSha256 = digest('materializer-original-request-bytes');
  const originalEventDigest = digest('materializer-original-event');
  const originalControllerHead = digest('materializer-original-controller-head');
  const globalPredecessorDigest = digest('materializer-global-predecessor-three');
  const semanticReceiptDigest = digest('materializer-global-receipt-three');
  const project = {
    projectAuthorityDigest: base.configuration.project.projectAuthorityDigest,
    principalId: base.configuration.project.principal.principalId,
    authenticationPolicyDigest: base.configuration.project.authenticationPolicyDigest,
  };
  const ports: SupervisorRegistrationDecisionPortsV1 = {
    mapAuthenticatedPeer: async () => ({ kind: 'mapped', project }),
    lookupExactCommittedResult: async () => ({ kind: 'absent' }),
    readActiveAuthorityHead: async () => ({
      kind: 'active', project, authorityHead: base.expectedEvent.authorityHead,
      expectedNextGlobalSequence: '4',
      requiredPredecessor: {
        kind: 'semantic-event', eventDigest: globalPredecessorDigest,
      },
    }),
    readRequiredPredecessorReceipt: async () => ({
      kind: 'ready', previousGlobal: {
        kind: 'semantic-event', eventDigest: globalPredecessorDigest,
        semanticReceiptDigest,
      },
    }),
    readRunState: async () => ({
      kind: 'registered', projectAuthorityDigest: project.projectAuthorityDigest,
      runId: RUN_ID, originalRegistrationRequestDigest: originalRequestDigest,
      originalRegistrationRequestSha256: originalRequestSha256,
      registrationEventDigest: originalEventDigest,
      lastRunEventDigest: originalEventDigest, lastRunGlobalSequence: '1',
      currentControllerStateHeadDigest: originalControllerHead, lastRunSequence: '0',
    }),
  };
  const decision = await decideSupervisorRegistrationV1(base.serializedRequest, PEER, ports);
  if (decision.decisionKind !== 'append-changed-replay-candidate') {
    throw new Error('expected changed-replay candidate');
  }
  const candidate = decision.candidate as any;
  const expectedEvent = buildProgrammeCaptureSupervisorRunEventV2({
    eventKind: candidate.candidateKind, authorityHead: candidate.authorityHead,
    service: {
      principalId: base.configuration.service.principal.principalId,
      keyEpoch: base.configuration.service.principal.keyEpoch,
      keyFingerprint: base.configuration.service.principal.keyFingerprint,
    },
    project: {
      projectAuthorityDigest: candidate.project.projectAuthorityDigest,
      principalId: candidate.project.principalId,
    },
    runId: candidate.request.runId,
    semanticRequestDigest: candidate.request.semanticRequestDigest,
    globalSequence: candidate.expectedNextGlobalSequence,
    runSequence: candidate.runSequence,
    previousGlobal: candidate.previousGlobal,
    previousRun: candidate.previousRun,
    priorControllerStateHeadDigest: candidate.priorControllerStateHeadDigest,
    resourceTransition: candidate.resourceTransition,
    body: candidate.body,
  });
  const lockedSnapshots = structuredClone(base.lockedSnapshots) as any;
  lockedSnapshots.lockedAuthorityState.lastGlobalSequence = '3';
  lockedSnapshots.lockedAuthorityState.nextGlobalSequence = '4';
  lockedSnapshots.lockedAuthorityState.lastEventDigest = digestBytes(globalPredecessorDigest);
  lockedSnapshots.lockedPredecessorReceipt = {
    kind: 'semantic-event',
    projectAuthorityDigest: digestBytes(project.projectAuthorityDigest),
    projectScopeRole: PROJECT_SCOPE_ROLE,
    eventDigest: digestBytes(globalPredecessorDigest),
    semanticReceiptDigest: digestBytes(semanticReceiptDigest),
  };
  lockedSnapshots.lockedRunState = {
    kind: 'registered',
    projectAuthorityDigest: digestBytes(project.projectAuthorityDigest),
    projectScopeRole: PROJECT_SCOPE_ROLE, runId: RUN_ID,
    originalRegistrationRequestDigest: digestBytes(originalRequestDigest),
    originalRegistrationRequestSha256: digestBytes(originalRequestSha256),
    originalRegistrationEventDigest: digestBytes(originalEventDigest),
    lastRunEventDigest: digestBytes(originalEventDigest), lastRunGlobalSequence: '1',
    currentControllerStateHeadDigest: digestBytes(originalControllerHead),
    lastRunSequence: '0', firstChangedReplayRequestDigest: null,
  };
  return { candidate: decision.candidate, expectedEvent, lockedSnapshots,
    request: base.request, originalRequestDigest, originalRequestSha256,
    originalEventDigest, originalControllerHead, globalPredecessorDigest,
    semanticReceiptDigest, serializedConfiguration: base.serializedConfiguration };
}

export async function nonGenesisRegistrationMaterializerFixtureV1() {
  const base = await genesisMaterializerFixtureV1();
  const candidate = structuredClone(base.candidate) as any;
  const lockedSnapshots = structuredClone(base.lockedSnapshots) as any;
  const globalPredecessorDigest = digest('materializer-registration-global-predecessor-three');
  const semanticReceiptDigest = digest('materializer-registration-global-receipt-three');
  candidate.expectedNextGlobalSequence = '4';
  candidate.previousGlobal = {
    kind: 'semantic-event', eventDigest: globalPredecessorDigest,
    semanticReceiptDigest,
  };
  lockedSnapshots.lockedAuthorityState.lastGlobalSequence = '3';
  lockedSnapshots.lockedAuthorityState.nextGlobalSequence = '4';
  lockedSnapshots.lockedAuthorityState.lastEventDigest = digestBytes(globalPredecessorDigest);
  lockedSnapshots.lockedPredecessorReceipt = {
    kind: 'semantic-event',
    projectAuthorityDigest: digestBytes(
      base.configuration.project.projectAuthorityDigest,
    ),
    projectScopeRole: PROJECT_SCOPE_ROLE,
    eventDigest: digestBytes(globalPredecessorDigest),
    semanticReceiptDigest: digestBytes(semanticReceiptDigest),
  };
  return {
    ...base, candidate, lockedSnapshots, globalPredecessorDigest, semanticReceiptDigest,
    expectedEvent: rebuildExpectedEvent(candidate, base.expectedEvent),
  };
}

export async function adjacentChangedReplayMaterializerFixtureV1() {
  const base = await changedReplayMaterializerFixtureV1();
  const candidate = structuredClone(base.candidate) as any;
  const lockedSnapshots = structuredClone(base.lockedSnapshots) as any;
  const semanticReceiptDigest = digest('materializer-adjacent-global-receipt');
  candidate.expectedNextGlobalSequence = '2';
  candidate.previousGlobal = {
    kind: 'semantic-event', eventDigest: base.originalEventDigest,
    semanticReceiptDigest,
  };
  lockedSnapshots.lockedAuthorityState.lastGlobalSequence = '1';
  lockedSnapshots.lockedAuthorityState.nextGlobalSequence = '2';
  lockedSnapshots.lockedAuthorityState.lastEventDigest = digestBytes(base.originalEventDigest);
  lockedSnapshots.lockedPredecessorReceipt = {
    kind: 'semantic-event',
    projectAuthorityDigest: digestBytes(
      base.expectedEvent.project.projectAuthorityDigest,
    ),
    projectScopeRole: PROJECT_SCOPE_ROLE,
    eventDigest: digestBytes(base.originalEventDigest),
    semanticReceiptDigest: digestBytes(semanticReceiptDigest),
  };
  return {
    ...base, candidate, lockedSnapshots,
    globalPredecessorDigest: base.originalEventDigest, semanticReceiptDigest,
    expectedEvent: rebuildExpectedEvent(candidate, base.expectedEvent),
  };
}

export function materializerSignatureForV1(
  event: ReturnType<typeof buildProgrammeCaptureSupervisorRunEventV2>,
): Buffer {
  const envelope = JSON.parse(signedEnvelope(event));
  return Buffer.from(envelope.signature.valueBase64Url, 'base64url');
}

function rebuildExpectedEvent(candidate: any, template: any) {
  return buildProgrammeCaptureSupervisorRunEventV2({
    eventKind: candidate.candidateKind,
    authorityHead: candidate.authorityHead,
    service: template.service,
    project: {
      projectAuthorityDigest: candidate.project.projectAuthorityDigest,
      principalId: candidate.project.principalId,
    },
    runId: candidate.request.runId,
    semanticRequestDigest: candidate.request.semanticRequestDigest,
    globalSequence: candidate.expectedNextGlobalSequence,
    runSequence: candidate.runSequence,
    previousGlobal: candidate.previousGlobal,
    previousRun: candidate.previousRun,
    priorControllerStateHeadDigest: candidate.priorControllerStateHeadDigest,
    resourceTransition: candidate.resourceTransition,
    body: candidate.body,
  });
}

function digestBytes(value: string): Buffer {
  return Buffer.from(value, 'hex');
}

function rawSha256(value: string): string {
  return digest(value);
}

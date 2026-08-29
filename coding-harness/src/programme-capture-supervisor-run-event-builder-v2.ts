// SPDX-License-Identifier: MIT

import { assertExactKeys } from './contracts.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,
  closedRunEventRecordV2,
  type ProgrammeCaptureSupervisorRunEventEnvelopeV2,
  type ProgrammeCaptureSupervisorRunEventV2,
} from './programme-capture-supervisor-run-event-contracts-v2.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_NON_AUTHORITY_V2,
  parseProgrammeCaptureSupervisorRunEventEnvelopeV2,
  parseProgrammeCaptureSupervisorRunEventV2,
} from './programme-capture-supervisor-run-event-codec-v2.js';
import { digestValue } from './receipts.js';

const BUILD_KEYS = Object.freeze([
  'eventKind', 'authorityHead', 'service', 'project', 'runId', 'semanticRequestDigest',
  'globalSequence', 'runSequence', 'previousGlobal', 'previousRun',
  'priorControllerStateHeadDigest', 'resourceTransition', 'body',
]);

export function buildProgrammeCaptureSupervisorRunEventV2(
  value: unknown,
): ProgrammeCaptureSupervisorRunEventV2 {
  const input = closedRunEventRecordV2(value, 'supervisor run-event construction input');
  assertExactKeys(input, BUILD_KEYS, 'supervisor run-event construction input');
  const body = {
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    recordKind: 'supervisor-run-event-v2',
    authority: 'development-only-no-promotion',
    eventKind: input.eventKind,
    authorityHead: input.authorityHead,
    service: input.service,
    project: input.project,
    runId: input.runId,
    semanticRequestDigest: input.semanticRequestDigest,
    globalSequence: input.globalSequence,
    runSequence: input.runSequence,
    previousGlobal: input.previousGlobal,
    previousRun: input.previousRun,
    priorControllerStateHeadDigest: input.priorControllerStateHeadDigest,
    resourceTransition: input.resourceTransition,
    body: input.body,
    verificationScope: 'service-signed-structure-only',
    ...PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_NON_AUTHORITY_V2,
  };
  return parseProgrammeCaptureSupervisorRunEventV2({
    ...body,
    eventDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,
      event: body,
    }),
  });
}

export function attachProgrammeCaptureSupervisorRunEventSignatureV2(
  value: unknown,
): ProgrammeCaptureSupervisorRunEventEnvelopeV2 {
  const input = closedRunEventRecordV2(value, 'supervisor run-event signature attachment input');
  assertExactKeys(
    input, ['event', 'signatureBase64Url'],
    'supervisor run-event signature attachment input',
  );
  return parseProgrammeCaptureSupervisorRunEventEnvelopeV2({
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    envelopeKind: 'supervisor-run-event-envelope-v2',
    event: input.event,
    signature: { algorithm: 'ed25519', valueBase64Url: input.signatureBase64Url },
  });
}

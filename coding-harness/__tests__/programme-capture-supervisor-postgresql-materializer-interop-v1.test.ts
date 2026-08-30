// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  buildProgrammeCaptureSupervisorPublicCommitmentV2,
  programmeCaptureSupervisorPublicCommitmentDigestV2,
  programmeCaptureSupervisorPublicCommitmentLeafBytesV2,
} from '../src/programme-capture-supervisor-public-commitment-v2.js';
import {
  programmeCaptureSupervisorRunEventSigningPayloadV2,
} from '../src/programme-capture-supervisor-run-event-codec-v2.js';
import {
  programmeCaptureSupervisorControllerStateHeadDigestV2,
} from '../src/programme-capture-supervisor-run-event-transition-v2.js';
import {
  buildProgrammeCaptureSupervisorServiceResultV2,
  serializeProgrammeCaptureSupervisorServiceResultV2,
} from '../src/programme-capture-supervisor-service-result-v2.js';
import {
  signedEnvelope,
} from './programme-capture-supervisor-run-event-v2-fixtures.js';
import {
  finalizePostgresRegistrationMaterializationV1,
  preparePostgresRegistrationMaterializationV1,
} from '../supervisor-service/src/registration-postgresql-materializer-v1.js';
import {
  adjacentChangedReplayMaterializerFixtureV1,
  changedReplayMaterializerFixtureV1,
  genesisMaterializerFixtureV1,
  materializerSignatureForV1,
  nonGenesisRegistrationMaterializerFixtureV1,
} from './programme-capture-supervisor-postgresql-materializer-fixtures-v1.js';

describe('PostgreSQL registration materializer V1 parent-builder interoperability', () => {
  it('reproduces the exact 201 event, signature payload, result, state head, and leaf',
    async () => {
      const fixture = await genesisMaterializerFixtureV1();
      const prepared = await preparePostgresRegistrationMaterializationV1(
        fixture.candidate, fixture.lockedSnapshots,
      );

      expect(Object.getOwnPropertyDescriptors(prepared)).toEqual({
        authority: {
          value: 'none', writable: false, enumerable: true, configurable: false,
        },
        mutationAuthorized: {
          value: false, writable: false, enumerable: true, configurable: false,
        },
        signingBytes: {
          value: expect.any(Uint8Array),
          writable: false, enumerable: true, configurable: false,
        },
      });
      expect(Reflect.ownKeys(prepared)).toEqual([
        'authority', 'mutationAuthorized', 'signingBytes',
      ]);
      expect(Object.isFrozen(prepared)).toBe(true);

      expect(Buffer.from(prepared.signingBytes)).toEqual(
        programmeCaptureSupervisorRunEventSigningPayloadV2(fixture.expectedEvent),
      );
      const serializedEnvelope = signedEnvelope(fixture.expectedEvent);
      const signature = Buffer.from(
        JSON.parse(serializedEnvelope).signature.valueBase64Url, 'base64url',
      );
      const finalized = await finalizePostgresRegistrationMaterializationV1(
        prepared, signature,
      );
      const expectedResult = serializeProgrammeCaptureSupervisorServiceResultV2(
        buildProgrammeCaptureSupervisorServiceResultV2({
          semanticRequestDigest: fixture.request.semanticRequestDigest,
          serializedEventEnvelope: serializedEnvelope,
        }),
      );
      const expectedHead = programmeCaptureSupervisorControllerStateHeadDigestV2(
        fixture.expectedEvent.priorControllerStateHeadDigest,
        fixture.expectedEvent.eventDigest,
      );
      const commitment = buildProgrammeCaptureSupervisorPublicCommitmentV2({
        serializedAuthorityConfiguration: fixture.serializedConfiguration,
        serializedEventEnvelope: serializedEnvelope,
      });
      const expectedLeaf = programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment);

      expectExactGenesisRows(finalized, fixture, {
        serializedEnvelope, expectedResult, expectedHead, expectedLeaf,
      });
      expect(hex(finalized.publicationOutboxRow.publicCommitmentDigest))
        .toBe(programmeCaptureSupervisorPublicCommitmentDigestV2(commitment));
      expect(finalized.registrationRunMutation.expectedOld.projectAuthorityDigest)
        .not.toBe(fixture.lockedSnapshots.lockedRunState.projectAuthorityDigest);
    });

  it('snapshots inputs, exposed signing bytes, and the signature before yielding', async () => {
    const fixture = await genesisMaterializerFixtureV1();
    const candidate = structuredClone(fixture.candidate) as any;
    const lockedSnapshots = structuredClone(fixture.lockedSnapshots);
    const pendingPrepare = preparePostgresRegistrationMaterializationV1(
      candidate, lockedSnapshots,
    );
    candidate.project.principalId = 'attacker_principal_20260830';
    lockedSnapshots.lockedConfiguration.serializedConfiguration.fill(0);
    const prepared = await pendingPrepare;
    prepared.signingBytes.fill(0);
    const signature = materializerSignatureForV1(fixture.expectedEvent);
    const pendingFinalize = finalizePostgresRegistrationMaterializationV1(
      prepared, signature,
    );
    signature.fill(0);

    await expect(pendingFinalize).resolves.toMatchObject({ response: { status: 201 } });
  });

  it('rejects forged and replayed handles and burns a handle on signature failure', async () => {
    const fixture = await genesisMaterializerFixtureV1();
    const signature = materializerSignatureForV1(fixture.expectedEvent);
    const genuine = await preparePostgresRegistrationMaterializationV1(
      fixture.candidate, fixture.lockedSnapshots,
    );
    let traps = 0;
    const forged = new Proxy(genuine, {
      get() { traps += 1; throw new Error('prepared token trap invoked'); },
    });
    await expect(finalizePostgresRegistrationMaterializationV1(
      forged, signature,
    )).rejects.toThrow(/identity/);
    expect(traps).toBe(0);
    const concurrent = await Promise.allSettled([
      finalizePostgresRegistrationMaterializationV1(genuine, signature),
      finalizePostgresRegistrationMaterializationV1(genuine, signature),
    ]);
    expect(concurrent.filter(({ status }) => status === 'fulfilled')).toHaveLength(1);
    expect(concurrent.filter(({ status }) => status === 'rejected')).toHaveLength(1);

    const burned = await preparePostgresRegistrationMaterializationV1(
      fixture.candidate, fixture.lockedSnapshots,
    );
    await expect(finalizePostgresRegistrationMaterializationV1(
      burned, new Uint8Array(63),
    )).rejects.toThrow(/64|exact bounded/);
    await expect(finalizePostgresRegistrationMaterializationV1(
      burned, signature,
    )).rejects.toThrow(/identity/);
  });

  it('keeps interleaved 409 global and run predecessors independent', async () => {
    const fixture = await changedReplayMaterializerFixtureV1();
    const prepared = await preparePostgresRegistrationMaterializationV1(
      fixture.candidate, fixture.lockedSnapshots,
    );
    const finalized = await finalizePostgresRegistrationMaterializationV1(
      prepared, materializerSignatureForV1(fixture.expectedEvent),
    );
    const serializedEnvelope = signedEnvelope(fixture.expectedEvent);
    const expectedResult = serializeProgrammeCaptureSupervisorServiceResultV2(
      buildProgrammeCaptureSupervisorServiceResultV2({
        semanticRequestDigest: fixture.request.semanticRequestDigest,
        serializedEventEnvelope: serializedEnvelope,
      }),
    );
    const expectedHead = programmeCaptureSupervisorControllerStateHeadDigestV2(
      fixture.expectedEvent.priorControllerStateHeadDigest,
      fixture.expectedEvent.eventDigest,
    );
    const commitment = buildProgrammeCaptureSupervisorPublicCommitmentV2({
      serializedAuthorityConfiguration: fixture.serializedConfiguration,
      serializedEventEnvelope: serializedEnvelope,
    });
    const expectedLeaf = programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment);

    expectExactChangedReplayRows(finalized, fixture, {
      serializedEnvelope, expectedResult, expectedHead, expectedLeaf,
    });
    expect(hex(finalized.semanticEventRow.previousGlobalEventDigest!))
      .not.toBe(hex(finalized.semanticEventRow.previousRunEventDigest!));
  });

  it('materializes exact non-genesis 201 rows without status-derived genesis fields',
    async () => {
      const fixture = await nonGenesisRegistrationMaterializerFixtureV1();
      const { finalized, artifacts } = await finalizeFixture(fixture);
      expectExactGenesisRows(finalized, fixture, artifacts);
      expect(finalized.semanticEventRow.previousGlobalKind).toBe('semantic-event');
    });

  it('materializes exact adjacent 409 rows when global and run predecessors agree',
    async () => {
      const fixture = await adjacentChangedReplayMaterializerFixtureV1();
      const { finalized, artifacts } = await finalizeFixture(fixture);
      expectExactChangedReplayRows(finalized, fixture, artifacts);
      expect(hex(finalized.semanticEventRow.previousGlobalEventDigest!))
        .toBe(hex(finalized.semanticEventRow.previousRunEventDigest!));
    });

  it('rejects an adjacent global predecessor that differs from the run predecessor', async () => {
    const fixture = await changedReplayMaterializerFixtureV1();
    const candidate = structuredClone(fixture.candidate) as any;
    const snapshots = structuredClone(fixture.lockedSnapshots) as any;
    candidate.expectedNextGlobalSequence = '2';
    snapshots.lockedAuthorityState.lastGlobalSequence = '1';
    snapshots.lockedAuthorityState.nextGlobalSequence = '2';
    await expect(preparePostgresRegistrationMaterializationV1(candidate, snapshots))
      .rejects.toThrow(/adjacent global\/run predecessor/);
  });

  it('burns the prepared identity after cryptographic signature rejection', async () => {
    const fixture = await genesisMaterializerFixtureV1();
    const prepared = await preparePostgresRegistrationMaterializationV1(
      fixture.candidate, fixture.lockedSnapshots,
    );
    const signature = materializerSignatureForV1(fixture.expectedEvent);
    signature[0] ^= 1;
    await expect(finalizePostgresRegistrationMaterializationV1(prepared, signature))
      .rejects.toThrow(/signature is invalid/);
    await expect(finalizePostgresRegistrationMaterializationV1(
      prepared, materializerSignatureForV1(fixture.expectedEvent),
    )).rejects.toThrow(/identity/);
  });
});

type Finalized = Awaited<ReturnType<typeof finalizePostgresRegistrationMaterializationV1>>;
type GenesisFixture = Awaited<ReturnType<typeof genesisMaterializerFixtureV1>>;
type ChangedFixture = Awaited<ReturnType<typeof changedReplayMaterializerFixtureV1>>;
type ExactArtifacts = Readonly<{
  serializedEnvelope: string; expectedResult: string;
  expectedHead: string; expectedLeaf: Uint8Array;
}>;

function expectExactGenesisRows(
  finalized: Finalized, fixture: GenesisFixture, artifacts: ExactArtifacts,
): void {
  const event = fixture.expectedEvent;
  const project = event.project.projectAuthorityDigest;
  const configuration = event.authorityHead.configurationDigest;
  const authorityHead = event.authorityHead.headDigest;
  const requestSha = rawSha256(fixture.serializedRequest);
  const previousGlobalSequence = event.previousGlobal.kind === 'semantic-event'
    ? String(BigInt(event.globalSequence) - 1n) : null;
  const nextGlobalSequence = String(BigInt(event.globalSequence) + 1n);
  expect(finalized).toEqual({
    response: {
      status: 201, contentType: 'application/json; charset=utf-8',
      body: artifacts.expectedResult,
    },
    semanticEventRow: {
      projectAuthorityDigest: digestBytes(project),
      projectScopeRole: 'sf_supervisor_project_scope_v1',
      eventDigest: digestBytes(event.eventDigest),
      eventKind: 'claim-registered-v2',
      semanticRequestDigest: digestBytes(event.semanticRequestDigest),
      runId: event.runId,
      authorityConfigurationEpoch: '0',
      authorityConfigurationDigest: digestBytes(configuration),
      authorityHeadDigest: digestBytes(authorityHead),
      globalSequence: event.globalSequence, runSequence: '0',
      previousGlobalKind: event.previousGlobal.kind,
      previousGlobalSequence,
      previousGlobalEventDigest: event.previousGlobal.eventDigest === null
        ? null : digestBytes(event.previousGlobal.eventDigest),
      previousGlobalGenesisConfigurationEpoch:
        event.previousGlobal.kind === 'authority-genesis' ? '0' : null,
      previousGlobalGenesisConfigurationDigest:
        event.previousGlobal.kind === 'authority-genesis' ? digestBytes(configuration) : null,
      previousGlobalGenesisReceiptDigest: event.previousGlobal.kind === 'authority-genesis'
        ? digestBytes(event.previousGlobal.semanticReceiptDigest) : null,
      previousGlobalEventReceiptDigest: event.previousGlobal.kind === 'semantic-event'
        ? digestBytes(event.previousGlobal.semanticReceiptDigest) : null,
      previousRunKind: 'run-genesis', previousRunSequence: null,
      previousRunGlobalSequence: null, previousRunEventDigest: null,
      priorControllerStateHeadDigest: digestBytes(event.priorControllerStateHeadDigest),
      resultingControllerStateHeadDigest: digestBytes(artifacts.expectedHead),
      serializedEnvelope: utf8Bytes(artifacts.serializedEnvelope),
      serializedEnvelopeSha256: digestBytes(rawSha256(artifacts.serializedEnvelope)),
    },
    registrationResultRow: {
      projectAuthorityDigest: digestBytes(project),
      projectScopeRole: 'sf_supervisor_project_scope_v1',
      semanticRequestDigest: digestBytes(event.semanticRequestDigest), runId: event.runId,
      originalRegistrationRequestDigest: digestBytes(event.semanticRequestDigest),
      originalRegistrationRequestSha256: digestBytes(requestSha),
      originalRegistrationEventDigest: digestBytes(event.eventDigest),
      serializedRequest: utf8Bytes(fixture.serializedRequest),
      serializedRequestSha256: digestBytes(requestSha), responseStatus: 201,
      responseContentType: 'application/json; charset=utf-8',
      serializedResponse: utf8Bytes(artifacts.expectedResult),
      serializedResponseSha256: digestBytes(rawSha256(artifacts.expectedResult)),
      currentEventDigest: digestBytes(event.eventDigest),
    },
    registrationRunMutation: {
      kind: 'insert',
      expectedOld: {
        kind: 'absent', projectAuthorityDigest: digestBytes(project),
        projectScopeRole: 'sf_supervisor_project_scope_v1', runId: event.runId,
      },
      resulting: {
        projectAuthorityDigest: digestBytes(project),
        projectScopeRole: 'sf_supervisor_project_scope_v1', runId: event.runId,
        originalRegistrationRequestDigest: digestBytes(event.semanticRequestDigest),
        originalRegistrationRequestSha256: digestBytes(requestSha),
        originalRegistrationEventDigest: digestBytes(event.eventDigest),
        lastRunEventDigest: digestBytes(event.eventDigest),
        lastRunGlobalSequence: event.globalSequence,
        currentControllerStateHeadDigest: digestBytes(artifacts.expectedHead),
        lastRunSequence: '0', firstChangedReplayRequestDigest: null,
      },
    },
    publicationOutboxRow: {
      projectAuthorityDigest: digestBytes(project),
      projectScopeRole: 'sf_supervisor_project_scope_v1',
      eventDigest: digestBytes(event.eventDigest),
      publicCommitmentLeafBytes: new Uint8Array(artifacts.expectedLeaf),
      publicCommitmentDigest: digestBytes(rawSha256(artifacts.expectedLeaf)),
      publicationState: 'pending',
    },
    authorityStateMutation: {
      expectedOld: authorityState(
        project, configuration, authorityHead, previousGlobalSequence ?? '0',
        event.globalSequence, event.previousGlobal.eventDigest,
      ),
      resulting: authorityState(
        project, configuration, authorityHead, event.globalSequence,
        nextGlobalSequence, event.eventDigest,
      ),
    },
  });
}

function expectExactChangedReplayRows(
  finalized: Finalized, fixture: ChangedFixture, artifacts: ExactArtifacts,
): void {
  const event = fixture.expectedEvent;
  const candidate = fixture.candidate as any;
  const project = event.project.projectAuthorityDigest;
  const configuration = event.authorityHead.configurationDigest;
  const authorityHead = event.authorityHead.headDigest;
  const serializedRequest = candidate.request.serialized as string;
  const requestSha = rawSha256(serializedRequest);
  const previousGlobalSequence = String(BigInt(event.globalSequence) - 1n);
  const nextGlobalSequence = String(BigInt(event.globalSequence) + 1n);
  const expectedOldRun = {
    kind: 'registered', projectAuthorityDigest: digestBytes(project),
    projectScopeRole: 'sf_supervisor_project_scope_v1', runId: event.runId,
    originalRegistrationRequestDigest: digestBytes(fixture.originalRequestDigest),
    originalRegistrationRequestSha256: digestBytes(fixture.originalRequestSha256),
    originalRegistrationEventDigest: digestBytes(fixture.originalEventDigest),
    lastRunEventDigest: digestBytes(fixture.originalEventDigest),
    lastRunGlobalSequence: '1',
    currentControllerStateHeadDigest: digestBytes(fixture.originalControllerHead),
    lastRunSequence: '0', firstChangedReplayRequestDigest: null,
  };
  expect(finalized).toEqual({
    response: {
      status: 409, contentType: 'application/json; charset=utf-8',
      body: artifacts.expectedResult,
    },
    semanticEventRow: {
      projectAuthorityDigest: digestBytes(project),
      projectScopeRole: 'sf_supervisor_project_scope_v1',
      eventDigest: digestBytes(event.eventDigest),
      eventKind: 'capture-run-terminal-v2',
      semanticRequestDigest: digestBytes(event.semanticRequestDigest), runId: event.runId,
      authorityConfigurationEpoch: '0',
      authorityConfigurationDigest: digestBytes(configuration),
      authorityHeadDigest: digestBytes(authorityHead),
      globalSequence: event.globalSequence, runSequence: '1',
      previousGlobalKind: 'semantic-event', previousGlobalSequence,
      previousGlobalEventDigest: digestBytes(event.previousGlobal.eventDigest!),
      previousGlobalGenesisConfigurationEpoch: null,
      previousGlobalGenesisConfigurationDigest: null,
      previousGlobalGenesisReceiptDigest: null,
      previousGlobalEventReceiptDigest: digestBytes(fixture.semanticReceiptDigest),
      previousRunKind: 'run-event', previousRunSequence: '0',
      previousRunGlobalSequence: '1',
      previousRunEventDigest: digestBytes(fixture.originalEventDigest),
      priorControllerStateHeadDigest: digestBytes(fixture.originalControllerHead),
      resultingControllerStateHeadDigest: digestBytes(artifacts.expectedHead),
      serializedEnvelope: utf8Bytes(artifacts.serializedEnvelope),
      serializedEnvelopeSha256: digestBytes(rawSha256(artifacts.serializedEnvelope)),
    },
    registrationResultRow: {
      projectAuthorityDigest: digestBytes(project),
      projectScopeRole: 'sf_supervisor_project_scope_v1',
      semanticRequestDigest: digestBytes(event.semanticRequestDigest), runId: event.runId,
      originalRegistrationRequestDigest: digestBytes(fixture.originalRequestDigest),
      originalRegistrationRequestSha256: digestBytes(fixture.originalRequestSha256),
      originalRegistrationEventDigest: digestBytes(fixture.originalEventDigest),
      serializedRequest: utf8Bytes(serializedRequest),
      serializedRequestSha256: digestBytes(requestSha), responseStatus: 409,
      responseContentType: 'application/json; charset=utf-8',
      serializedResponse: utf8Bytes(artifacts.expectedResult),
      serializedResponseSha256: digestBytes(rawSha256(artifacts.expectedResult)),
      currentEventDigest: digestBytes(event.eventDigest),
    },
    registrationRunMutation: {
      kind: 'update', expectedOld: expectedOldRun,
      resulting: {
        projectAuthorityDigest: digestBytes(project),
        projectScopeRole: 'sf_supervisor_project_scope_v1', runId: event.runId,
        originalRegistrationRequestDigest: digestBytes(fixture.originalRequestDigest),
        originalRegistrationRequestSha256: digestBytes(fixture.originalRequestSha256),
        originalRegistrationEventDigest: digestBytes(fixture.originalEventDigest),
        lastRunEventDigest: digestBytes(event.eventDigest),
        lastRunGlobalSequence: event.globalSequence,
        currentControllerStateHeadDigest: digestBytes(artifacts.expectedHead),
        lastRunSequence: '1',
        firstChangedReplayRequestDigest: digestBytes(event.semanticRequestDigest),
      },
    },
    publicationOutboxRow: {
      projectAuthorityDigest: digestBytes(project),
      projectScopeRole: 'sf_supervisor_project_scope_v1',
      eventDigest: digestBytes(event.eventDigest),
      publicCommitmentLeafBytes: new Uint8Array(artifacts.expectedLeaf),
      publicCommitmentDigest: digestBytes(rawSha256(artifacts.expectedLeaf)),
      publicationState: 'pending',
    },
    authorityStateMutation: {
      expectedOld: authorityState(
        project, configuration, authorityHead, previousGlobalSequence,
        event.globalSequence, event.previousGlobal.eventDigest,
      ),
      resulting: authorityState(
        project, configuration, authorityHead, event.globalSequence,
        nextGlobalSequence, event.eventDigest,
      ),
    },
  });
}

function authorityState(
  project: string, configuration: string, head: string,
  last: string, next: string, event: string | null,
) {
  return {
    projectAuthorityDigest: digestBytes(project),
    projectScopeRole: 'sf_supervisor_project_scope_v1', singletonKey: true,
    activeConfigurationEpoch: '0', activeConfigurationDigest: digestBytes(configuration),
    authorityHeadDigest: digestBytes(head), lastGlobalSequence: last,
    nextGlobalSequence: next, lastEventDigest: event === null ? null : digestBytes(event),
  };
}

function digestBytes(value: string): Uint8Array {
  return new Uint8Array(Buffer.from(value, 'hex'));
}

function utf8Bytes(value: string): Uint8Array {
  return new Uint8Array(Buffer.from(value, 'utf8'));
}

function hex(value: Uint8Array): string {
  return Buffer.from(value).toString('hex');
}

function rawSha256(value: string | Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

async function finalizeFixture(fixture: GenesisFixture | ChangedFixture) {
  const prepared = await preparePostgresRegistrationMaterializationV1(
    fixture.candidate, fixture.lockedSnapshots,
  );
  const finalized = await finalizePostgresRegistrationMaterializationV1(
    prepared, materializerSignatureForV1(fixture.expectedEvent),
  );
  const serializedEnvelope = signedEnvelope(fixture.expectedEvent);
  const expectedResult = serializeProgrammeCaptureSupervisorServiceResultV2(
    buildProgrammeCaptureSupervisorServiceResultV2({
      semanticRequestDigest: fixture.request.semanticRequestDigest,
      serializedEventEnvelope: serializedEnvelope,
    }),
  );
  const expectedHead = programmeCaptureSupervisorControllerStateHeadDigestV2(
    fixture.expectedEvent.priorControllerStateHeadDigest,
    fixture.expectedEvent.eventDigest,
  );
  const commitment = buildProgrammeCaptureSupervisorPublicCommitmentV2({
    serializedAuthorityConfiguration: fixture.serializedConfiguration,
    serializedEventEnvelope: serializedEnvelope,
  });
  return {
    finalized,
    artifacts: {
      serializedEnvelope, expectedResult, expectedHead,
      expectedLeaf: programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment),
    },
  };
}

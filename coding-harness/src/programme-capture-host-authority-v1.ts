// SPDX-License-Identifier: MIT

import { asClosedRecord, assertExactKeys } from './contracts.js';
import {
  readProgrammeCaptureRunClaimV1,
  type ProgrammeCaptureRunClaimAuthorityInputV1,
} from './programme-capture-claim-io-v1.js';
import {
  deriveProgrammeCaptureHostNonAdmissionViewV1,
  verifyProgrammeCaptureHostNonAdmissionViewV1,
  type ProgrammeCaptureHostNonAdmissionV1,
} from './programme-capture-host-preflight-v1.js';
import type { ProgrammeCaptureHostObservationV1 } from './programme-capture-host-observation-v1.js';
import {
  transitionProgrammeCaptureStateV1,
  type ProgrammeCaptureStateV1,
} from './programme-capture-state-v1.js';

export interface ProgrammeCaptureHostClaimAuthorityV1 {
  readonly claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1;
}

export async function rejectProgrammeCaptureHostPreflightV1(value: Readonly<{
  claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1;
  profileBytes: Uint8Array | undefined;
  observation: ProgrammeCaptureHostObservationV1;
}>): Promise<Readonly<{
  record: ProgrammeCaptureHostNonAdmissionV1;
  state: ProgrammeCaptureStateV1;
}>> {
  const input = asClosedRecord(value, 'programme capture authoritative host rejection input');
  assertExactKeys(
    input, ['claimAuthority', 'profileBytes', 'observation'],
    'programme capture authoritative host rejection input',
  );
  const rooted = await rootedInputsAttestedState(
    input.claimAuthority as ProgrammeCaptureRunClaimAuthorityInputV1,
  );
  return deriveProgrammeCaptureHostNonAdmissionViewV1({
    state: rooted.state,
    inputAttestation: rooted.inputAttestation,
    profileBytes: input.profileBytes as Uint8Array | undefined,
    observation: input.observation as ProgrammeCaptureHostObservationV1,
  });
}

export async function verifyProgrammeCaptureHostNonAdmissionV1(value: Readonly<{
  record: ProgrammeCaptureHostNonAdmissionV1;
  state: ProgrammeCaptureStateV1;
  claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1;
  profileBytes: Uint8Array | undefined;
}>): Promise<ProgrammeCaptureHostNonAdmissionV1> {
  const input = asClosedRecord(value, 'programme capture authoritative host verification input');
  assertExactKeys(
    input, ['record', 'state', 'claimAuthority', 'profileBytes'],
    'programme capture authoritative host verification input',
  );
  const rooted = await rootedInputsAttestedState(
    input.claimAuthority as ProgrammeCaptureRunClaimAuthorityInputV1,
  );
  return verifyProgrammeCaptureHostNonAdmissionViewV1({
    record: input.record as ProgrammeCaptureHostNonAdmissionV1,
    beforeState: rooted.state,
    afterState: input.state as ProgrammeCaptureStateV1,
    inputAttestation: rooted.inputAttestation,
    profileBytes: input.profileBytes as Uint8Array | undefined,
  });
}

async function rootedInputsAttestedState(
  claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1,
): Promise<Readonly<{
  state: ProgrammeCaptureStateV1;
  inputAttestation: Awaited<ReturnType<typeof readProgrammeCaptureRunClaimV1>>['inputAttestation'];
}>> {
  const reservation = await readProgrammeCaptureRunClaimV1(claimAuthority);
  const state = transitionProgrammeCaptureStateV1(reservation.stateView, {
    kind: 'attest-inputs',
    evidenceDigest: reservation.inputAttestation.attestationDigest,
  });
  return Object.freeze({ state, inputAttestation: reservation.inputAttestation });
}

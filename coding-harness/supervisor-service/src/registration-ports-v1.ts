// SPDX-License-Identifier: MIT

declare const AUTHENTICATED_TRANSPORT_PEER_BRAND: unique symbol;

/** Adapter-issued Symbol capability; the mapper resolves identity and never reads fields. */
export type AuthenticatedTransportPeerV1 = symbol & Readonly<{
  readonly [AUTHENTICATED_TRANSPORT_PEER_BRAND]: 'authenticated-transport-peer-v1';
}>;

export interface TrustedProjectBindingV1 {
  readonly projectAuthorityDigest: string;
  readonly principalId: string;
  readonly authenticationPolicyDigest: string;
}

export type ProjectMappingReadV1 =
  | Readonly<{ kind: 'mapped'; project: TrustedProjectBindingV1 }>
  | Readonly<{ kind: 'not-admitted' }>
  | Readonly<{ kind: 'indeterminate' }>;

export type ExactCommittedResultReadV1 =
  | Readonly<{ kind: 'absent' }>
  | Readonly<{ kind: 'indeterminate' }>
  | Readonly<{
    kind: 'found';
    /** One joined committed-result/original-event/run row; provenance is not synthesized. */
    row: Readonly<{
      projectAuthorityDigest: string;
      semanticRequestDigest: string;
      originalRegistrationRequestDigest: string;
      originalRegistrationEventDigest: string;
      originalRegistrationGlobalSequence: string;
      changedReplayPriorControllerStateHeadDigest: string | null;
      serializedRequest: string;
      serializedRequestSha256: string;
      responseStatus: 201 | 409;
      responseContentType: 'application/json; charset=utf-8';
      serializedResponse: string;
      serializedResponseSha256: string;
    }>;
  }>;

export interface RequiredPredecessorRefV1 {
  readonly kind: 'authority-genesis' | 'semantic-event';
  readonly eventDigest: string | null;
}

export type ActiveAuthorityHeadReadV1 =
  | Readonly<{ kind: 'not-admitted' }>
  | Readonly<{ kind: 'indeterminate' }>
  | Readonly<{
    kind: 'active';
    project: TrustedProjectBindingV1;
    authorityHead: Readonly<{
      configurationEpoch: string;
      configurationDigest: string;
      headDigest: string;
    }>;
    expectedNextGlobalSequence: string;
    requiredPredecessor: RequiredPredecessorRefV1;
  }>;

export type RequiredPredecessorReceiptReadV1 =
  | Readonly<{ kind: 'pending' }>
  | Readonly<{ kind: 'indeterminate' }>
  | Readonly<{
    kind: 'ready';
    previousGlobal: Readonly<{
      kind: 'authority-genesis' | 'semantic-event';
      eventDigest: string | null;
      semanticReceiptDigest: string;
    }>;
  }>;

interface StoredRunStateV1 {
  readonly projectAuthorityDigest: string;
  readonly runId: string;
  readonly originalRegistrationRequestDigest: string;
  readonly registrationEventDigest: string;
  readonly lastRunEventDigest: string;
  readonly lastRunGlobalSequence: string;
  readonly currentControllerStateHeadDigest: string;
  readonly lastRunSequence: string;
}

export type RegistrationRunStateReadV1 =
  | Readonly<{ kind: 'absent' }>
  | Readonly<StoredRunStateV1 & {
    kind: 'registered';
    originalRegistrationRequestSha256: string;
    lastRunSequence: '0';
  }>
  | Readonly<StoredRunStateV1 & {
    kind: 'advanced-or-closed';
    firstChangedReplayRequestDigest: string | null;
  }>
  | Readonly<{ kind: 'indeterminate' }>;

export interface SupervisorRegistrationDecisionPortsV1 {
  mapAuthenticatedPeer(peer: AuthenticatedTransportPeerV1): Promise<ProjectMappingReadV1>;
  lookupExactCommittedResult(input: Readonly<{
    projectAuthorityDigest: string;
    semanticRequestDigest: string;
  }>): Promise<ExactCommittedResultReadV1>;
  readActiveAuthorityHead(input: Readonly<{
    projectAuthorityDigest: string;
  }>): Promise<ActiveAuthorityHeadReadV1>;
  readRequiredPredecessorReceipt(input: Readonly<{
    projectAuthorityDigest: string;
    requiredPredecessor: RequiredPredecessorRefV1;
  }>): Promise<RequiredPredecessorReceiptReadV1>;
  readRunState(input: Readonly<{
    projectAuthorityDigest: string;
    runId: string;
  }>): Promise<RegistrationRunStateReadV1>;
}

// SPDX-License-Identifier: MIT

import type { SupervisorRegistrationDecisionV1 } from './registration-decision-v1.js';
import type {
  AuthenticatedTransportPeerV1,
  SupervisorRegistrationDecisionPortsV1,
} from './registration-ports-v1.js';
import type { SupervisorRegistrationCommitResolutionV1 } from
  './registration-transaction-retry-v1.js';

type AppendDecisionV1 = Extract<SupervisorRegistrationDecisionV1, Readonly<{
  decisionKind: 'append-registration-candidate' | 'append-changed-replay-candidate';
}>>;

declare const AUTHENTICATED_EXACT_RECOVERY_PEER_BRAND: unique symbol;
export type AuthenticatedExactRecoveryPeerV1 = symbol & Readonly<{
  readonly [AUTHENTICATED_EXACT_RECOVERY_PEER_BRAND]: 'authenticated-exact-recovery-peer-v1';
}>;

export interface AuthenticatedRegistrationPeerRegistryV1 {
  consumeRegistration(peer: AuthenticatedTransportPeerV1): boolean;
}

export interface AuthenticatedExactRecoveryPeerRegistryV1 {
  consumeExactRecovery(peer: AuthenticatedExactRecoveryPeerV1): boolean;
}

export interface SupervisorRegistrationTransactionV1 {
  readonly ports: SupervisorRegistrationDecisionPortsV1;
  stageCandidate(candidate: AppendDecisionV1['candidate']): Promise<void>;
  commit(): Promise<SupervisorRegistrationCommitResolutionV1>;
  rollback(): Promise<void>;
  quarantine(): Promise<void>;
}

export interface SupervisorRegistrationTransactionCheckoutV1 {
  /** The sole transaction-opening operation; checkout acquisition is allocation-free. */
  open(): Promise<SupervisorRegistrationTransactionV1>;
  discardMalformed(): Promise<void>;
}

export interface SupervisorRegistrationTransactionStoreV1 {
  checkoutRegistration(): Promise<SupervisorRegistrationTransactionCheckoutV1>;
}

export interface SupervisorRegistrationRecoveryPortsV1 {
  mapAuthenticatedRecoveryPeer(
    peer: AuthenticatedExactRecoveryPeerV1,
  ): ReturnType<SupervisorRegistrationDecisionPortsV1['mapAuthenticatedPeer']>;
  lookupExactCommittedResult(
    input: Parameters<SupervisorRegistrationDecisionPortsV1[
      'lookupExactCommittedResult'
    ]>[0],
  ): ReturnType<SupervisorRegistrationDecisionPortsV1['lookupExactCommittedResult']>;
}

export interface SupervisorRegistrationRecoveryTransactionV1 {
  readonly ports: SupervisorRegistrationRecoveryPortsV1;
  commit(): Promise<SupervisorRegistrationCommitResolutionV1>;
  rollback(): Promise<void>;
  quarantine(): Promise<void>;
}

export interface SupervisorRegistrationRecoveryCheckoutV1 {
  /** The sole transaction-opening operation; checkout acquisition is allocation-free. */
  open(): Promise<SupervisorRegistrationRecoveryTransactionV1>;
  discardMalformed(): Promise<void>;
}

export interface SupervisorRegistrationRecoveryStoreV1 {
  checkoutExactRecovery(): Promise<SupervisorRegistrationRecoveryCheckoutV1>;
}

export type { SupervisorRegistrationCommitResolutionV1 } from
  './registration-transaction-retry-v1.js';

// SPDX-License-Identifier: MIT

import { ClosedJsonHashError, deepFreeze } from './closed-json.js';
import {
  decideSupervisorRegistrationExactPhaseV1,
  decideSupervisorRegistrationV1,
  type SupervisorRegistrationDecisionV1,
  type SupervisorRegistrationExactPhaseDecisionV1,
} from './registration-decision-v1.js';
import type {
  AuthenticatedTransportPeerV1,
  SupervisorRegistrationDecisionPortsV1,
} from './registration-ports-v1.js';
import {
  fixedRegistrationTransportResponseV2,
  parseCanonicalRegistrationRequestV2,
  type FixedRegistrationTransportResponseV2,
} from './registration-protocol-v2.js';
import {
  capabilityRecordV1,
  captureCapabilityMethodV1,
  closeRegistrationAdapterOperationV1,
  stagedResponseMatchesCandidateV1,
} from './registration-transaction-boundary-v1.js';

type AppendDecisionV1 = Extract<SupervisorRegistrationDecisionV1, Readonly<{
  decisionKind: 'append-registration-candidate' | 'append-changed-replay-candidate';
}>>;
type ExactResponseDecisionV1 = Extract<SupervisorRegistrationExactPhaseDecisionV1,
  Readonly<{ decisionKind: 'exact-response' }>>;
export type SupervisorRegistrationCommitResolutionV1 = 'committed' | 'unknown';
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

type RegistrationResponseV1 =
  | FixedRegistrationTransportResponseV2
  | ExactResponseDecisionV1['response'];

export interface SupervisorRegistrationTransactionResultV1 {
  readonly authority: 'none';
  readonly mutationAuthorized: false;
  readonly response: RegistrationResponseV1;
}

type BoundTransactionV1 = Readonly<{
  ports: SupervisorRegistrationDecisionPortsV1;
  stageCandidate(candidate: AppendDecisionV1['candidate']): Promise<void>;
  commit(): Promise<SupervisorRegistrationCommitResolutionV1>;
  rollback(): Promise<void>;
  quarantine(): Promise<void>;
  lastExactRead(): unknown;
}>;

type BoundRecoveryTransactionV1 = Readonly<{
  ports: SupervisorRegistrationRecoveryPortsV1;
  commit(): Promise<SupervisorRegistrationCommitResolutionV1>;
  rollback(): Promise<void>;
  quarantine(): Promise<void>;
}>;

export async function executeSupervisorRegistrationTransactionV1(
  serializedRequest: string,
  authenticatedPeer: AuthenticatedTransportPeerV1,
  peerRegistry: AuthenticatedRegistrationPeerRegistryV1,
  store: SupervisorRegistrationTransactionStoreV1,
): Promise<SupervisorRegistrationTransactionResultV1> {
  const requestAdmission = await preclassifyRequest(serializedRequest);
  if (requestAdmission === false) return notAdmitted();
  if (requestAdmission === null) return indeterminate();
  const admission = consumePeer(
    authenticatedPeer, peerRegistry, 'consumeRegistration', 'registration peer registry',
  );
  if (admission === false) return notAdmitted();
  if (admission === null) return indeterminate();
  const transaction = await beginRegistration(store);
  if (transaction === null) return indeterminate();

  let decision: SupervisorRegistrationDecisionV1;
  try {
    decision = await decideSupervisorRegistrationV1(
      serializedRequest, authenticatedPeer, transaction.ports,
    );
  } catch {
    return rollbackIndeterminate(transaction);
  }
  if (!isAppendDecision(decision)) {
    return commitResponse(transaction, responseOf(decision));
  }

  try {
    await transaction.stageCandidate(decision.candidate);
  } catch {
    return rollbackIndeterminate(transaction);
  }

  let staged: SupervisorRegistrationExactPhaseDecisionV1;
  try {
    staged = await decideSupervisorRegistrationExactPhaseV1(
      serializedRequest, authenticatedPeer, transaction.ports,
    );
  } catch {
    return rollbackIndeterminate(transaction);
  }
  const expectedStatus = decision.decisionKind === 'append-registration-candidate' ? 201 : 409;
  if (staged.decisionKind !== 'exact-response'
    || staged.response.status !== expectedStatus
    || !await stagedResponseMatchesCandidateV1(
      staged.response.body, decision.candidate, transaction.lastExactRead(),
    )) {
    return rollbackIndeterminate(transaction);
  }
  return commitResponse(transaction, staged.response);
}

export async function recoverExactSupervisorRegistrationV1(
  serializedRequest: string,
  authenticatedPeer: AuthenticatedExactRecoveryPeerV1,
  peerRegistry: AuthenticatedExactRecoveryPeerRegistryV1,
  store: SupervisorRegistrationRecoveryStoreV1,
): Promise<SupervisorRegistrationTransactionResultV1> {
  const requestAdmission = await preclassifyRequest(serializedRequest);
  if (requestAdmission === false) return notAdmitted();
  if (requestAdmission === null) return indeterminate();
  const admission = consumePeer(
    authenticatedPeer, peerRegistry, 'consumeExactRecovery', 'exact recovery peer registry',
  );
  if (admission === false) return notAdmitted();
  if (admission === null) return indeterminate();
  const transaction = await beginExactRecovery(store);
  if (transaction === null) return indeterminate();

  let decision: SupervisorRegistrationExactPhaseDecisionV1;
  try {
    const decisionPeer = authenticatedPeer as unknown as AuthenticatedTransportPeerV1;
    decision = await decideSupervisorRegistrationExactPhaseV1(
      serializedRequest, decisionPeer, Object.freeze({
        mapAuthenticatedPeer: async () =>
          transaction.ports.mapAuthenticatedRecoveryPeer(authenticatedPeer),
        lookupExactCommittedResult: transaction.ports.lookupExactCommittedResult,
      }),
    );
  } catch {
    return rollbackIndeterminate(transaction);
  }
  return commitResponse(
    transaction,
    decision.decisionKind === 'exact-response' ? decision.response
      : fixedRegistrationTransportResponseV2('transaction-resolution-unknown-v2'),
  );
}

async function preclassifyRequest(serializedRequest: string): Promise<boolean | null> {
  try { await parseCanonicalRegistrationRequestV2(serializedRequest); return true; }
  catch (error) {
    return error instanceof ClosedJsonHashError || !(error instanceof TypeError) ? null : false;
  }
}

function consumePeer(
  peer: unknown,
  registry: unknown,
  operation: 'consumeRegistration' | 'consumeExactRecovery',
  label: string,
): boolean | null {
  if (typeof peer !== 'symbol') return false;
  try {
    const consume = captureCapabilityMethodV1(registry, [operation], label, operation);
    const admitted = consume(peer);
    return typeof admitted === 'boolean' ? admitted : null;
  } catch {
    return null;
  }
}

async function beginRegistration(
  store: SupervisorRegistrationTransactionStoreV1,
): Promise<BoundTransactionV1 | null> {
  try {
    const keys = ['checkoutRegistration'] as const;
    const begin = captureCapabilityMethodV1(store, keys, 'transaction store', keys[0]);
    const checkout = capabilityRecordV1(
      await begin(), ['open', 'discardMalformed'], 'registration checkout',
    );
    const open = captureCapabilityMethodV1(
      checkout, ['open', 'discardMalformed'], 'registration checkout', 'open',
    );
    const discard = captureCapabilityMethodV1(
      checkout, ['open', 'discardMalformed'], 'registration checkout', 'discardMalformed',
    );
    try { return bindTransaction(await open()); }
    catch {
      try { await discard(); } catch { /* fixed indeterminate remains the only result */ }
      return null;
    }
  } catch {
    return null;
  }
}

async function beginExactRecovery(
  store: SupervisorRegistrationRecoveryStoreV1,
): Promise<BoundRecoveryTransactionV1 | null> {
  try {
    const keys = ['checkoutExactRecovery'] as const;
    const begin = captureCapabilityMethodV1(store, keys, 'recovery store', keys[0]);
    const checkout = capabilityRecordV1(
      await begin(), ['open', 'discardMalformed'], 'recovery checkout',
    );
    const open = captureCapabilityMethodV1(
      checkout, ['open', 'discardMalformed'], 'recovery checkout', 'open',
    );
    const discard = captureCapabilityMethodV1(
      checkout, ['open', 'discardMalformed'], 'recovery checkout', 'discardMalformed',
    );
    try { return bindRecoveryTransaction(await open()); }
    catch {
      try { await discard(); } catch { /* fixed indeterminate remains the only result */ }
      return null;
    }
  } catch {
    return null;
  }
}

function bindTransaction(value: unknown): BoundTransactionV1 {
  const record = capabilityRecordV1(value, [
    'ports', 'stageCandidate', 'commit', 'rollback', 'quarantine',
  ], 'registration transaction');
  const stageCandidate = captureCapabilityMethodV1(
    record, ['ports', 'stageCandidate', 'commit', 'rollback', 'quarantine'],
    'registration transaction', 'stageCandidate',
  );
  const commit = captureCapabilityMethodV1(
    record, ['ports', 'stageCandidate', 'commit', 'rollback', 'quarantine'],
    'registration transaction', 'commit',
  );
  const rollback = captureCapabilityMethodV1(
    record, ['ports', 'stageCandidate', 'commit', 'rollback', 'quarantine'],
    'registration transaction', 'rollback',
  );
  const quarantine = captureCapabilityMethodV1(
    record, ['ports', 'stageCandidate', 'commit', 'rollback', 'quarantine'],
    'registration transaction', 'quarantine',
  );
  const bestEffortQuarantine = async () => { try { await quarantine(); } catch { /* closed */ } };
  let lastExactRead: unknown = null;
  let state: 'open' | 'operation' | 'staged' | 'terminal' = 'open';
  const invokePort = async <T>(operation: () => Promise<T>): Promise<T> => {
    const previous = state;
    if (previous !== 'open' && previous !== 'staged') {
      throw new TypeError('registration transaction is busy or closed');
    }
    state = 'operation';
    try { return await closeRegistrationAdapterOperationV1(operation); }
    finally { if (state === 'operation') state = previous; }
  };
  return Object.freeze({
    ports: bindPorts(record.ports, invokePort, (value) => { lastExactRead = value; }),
    stageCandidate: async (candidate: AppendDecisionV1['candidate']) => {
      if (state !== 'open') throw new TypeError('registration transaction cannot stage');
      state = 'operation';
      try { await stageCandidate(candidate); state = 'staged'; }
      catch (error) { state = 'open'; throw error; }
    },
    commit: async () => {
      if (state !== 'open' && state !== 'staged') {
        throw new TypeError('registration transaction cannot commit');
      }
      state = 'terminal';
      let resolution: unknown;
      try { resolution = await commit(); }
      catch (error) { await bestEffortQuarantine(); throw error; }
      if (resolution === 'committed') return 'committed';
      await bestEffortQuarantine();
      return 'unknown';
    },
    rollback: async () => {
      if (state !== 'open' && state !== 'staged') {
        throw new TypeError('registration transaction cannot roll back');
      }
      state = 'terminal';
      try { await rollback(); }
      catch (error) { await bestEffortQuarantine(); throw error; }
    },
    quarantine: bestEffortQuarantine,
    lastExactRead: () => lastExactRead,
  });
}

function bindRecoveryTransaction(value: unknown): BoundRecoveryTransactionV1 {
  const keys = ['ports', 'commit', 'rollback', 'quarantine'] as const;
  const record = capabilityRecordV1(value, keys, 'registration recovery transaction');
  const commit = captureCapabilityMethodV1(
    record, keys, 'registration recovery transaction', 'commit',
  );
  const rollback = captureCapabilityMethodV1(
    record, keys, 'registration recovery transaction', 'rollback',
  );
  const quarantine = captureCapabilityMethodV1(
    record, keys, 'registration recovery transaction', 'quarantine',
  );
  const bestEffortQuarantine = async () => { try { await quarantine(); } catch { /* closed */ } };
  let state: 'open' | 'operation' | 'terminal' = 'open';
  const invokePort = async <T>(operation: () => Promise<T>): Promise<T> => {
    if (state !== 'open') throw new TypeError('registration recovery is busy or closed');
    state = 'operation';
    try { return await closeRegistrationAdapterOperationV1(operation); }
    finally { if (state === 'operation') state = 'open'; }
  };
  return Object.freeze({
    ports: bindRecoveryPorts(record.ports, invokePort),
    commit: async () => {
      if (state !== 'open') throw new TypeError('registration recovery cannot commit');
      state = 'terminal';
      let resolution: unknown;
      try { resolution = await commit(); }
      catch (error) { await bestEffortQuarantine(); throw error; }
      if (resolution === 'committed') return 'committed';
      await bestEffortQuarantine();
      return 'unknown';
    },
    rollback: async () => {
      if (state !== 'open') throw new TypeError('registration recovery cannot roll back');
      state = 'terminal';
      try { await rollback(); }
      catch (error) { await bestEffortQuarantine(); throw error; }
    },
    quarantine: bestEffortQuarantine,
  });
}

type PortInvokerV1 = <T>(operation: () => Promise<T>) => Promise<T>;

function bindPorts(
  value: unknown,
  invoke: PortInvokerV1 = closeRegistrationAdapterOperationV1,
  observeExact: (value: unknown) => void = () => undefined,
): SupervisorRegistrationDecisionPortsV1 {
  const keys = [
    'mapAuthenticatedPeer', 'lookupExactCommittedResult', 'readActiveAuthorityHead',
    'readRequiredPredecessorReceipt', 'readRunState',
  ] as const;
  const record = capabilityRecordV1(value, keys, 'registration transaction ports');
  const map = captureCapabilityMethodV1(record, keys, 'registration transaction ports', keys[0]);
  const exact = captureCapabilityMethodV1(
    record, keys, 'registration transaction ports', keys[1],
  );
  const head = captureCapabilityMethodV1(record, keys, 'registration transaction ports', keys[2]);
  const receipt = captureCapabilityMethodV1(
    record, keys, 'registration transaction ports', keys[3],
  );
  const run = captureCapabilityMethodV1(record, keys, 'registration transaction ports', keys[4]);
  return Object.freeze({
    mapAuthenticatedPeer: (
      input: Parameters<SupervisorRegistrationDecisionPortsV1['mapAuthenticatedPeer']>[0],
    ) => invoke(() => map(input)),
    lookupExactCommittedResult: async (
      input: Parameters<SupervisorRegistrationDecisionPortsV1[
        'lookupExactCommittedResult'
      ]>[0],
    ) => {
      const value = await invoke(() => exact(input));
      observeExact(value);
      return value;
    },
    readActiveAuthorityHead: (
      input: Parameters<SupervisorRegistrationDecisionPortsV1['readActiveAuthorityHead']>[0],
    ) => invoke(() => head(input)),
    readRequiredPredecessorReceipt: (
      input: Parameters<SupervisorRegistrationDecisionPortsV1[
        'readRequiredPredecessorReceipt'
      ]>[0],
    ) => invoke(() => receipt(input)),
    readRunState: (
      input: Parameters<SupervisorRegistrationDecisionPortsV1['readRunState']>[0],
    ) => invoke(() => run(input)),
  }) as SupervisorRegistrationDecisionPortsV1;
}

function bindRecoveryPorts(
  value: unknown,
  invoke: PortInvokerV1 = closeRegistrationAdapterOperationV1,
): SupervisorRegistrationRecoveryPortsV1 {
  const keys = ['mapAuthenticatedRecoveryPeer', 'lookupExactCommittedResult'] as const;
  const record = capabilityRecordV1(value, keys, 'registration recovery ports');
  const map = captureCapabilityMethodV1(record, keys, 'registration recovery ports', keys[0]);
  const exact = captureCapabilityMethodV1(record, keys, 'registration recovery ports', keys[1]);
  return Object.freeze({
    mapAuthenticatedRecoveryPeer: (
      input: AuthenticatedExactRecoveryPeerV1,
    ) => invoke(() => map(input)),
    lookupExactCommittedResult: (
      input: Parameters<SupervisorRegistrationDecisionPortsV1[
        'lookupExactCommittedResult'
      ]>[0],
    ) => invoke(() => exact(input)),
  }) as SupervisorRegistrationRecoveryPortsV1;
}

async function commitResponse(
  transaction: Readonly<{
    commit(): Promise<SupervisorRegistrationCommitResolutionV1>;
  }>,
  response: RegistrationResponseV1,
): Promise<SupervisorRegistrationTransactionResultV1> {
  try {
    if (await transaction.commit() !== 'committed') return indeterminate();
    return result(response);
  } catch {
    return indeterminate();
  }
}

async function rollbackIndeterminate(
  transaction: Readonly<{ rollback(): Promise<void> }>,
): Promise<SupervisorRegistrationTransactionResultV1> {
  try { await transaction.rollback(); }
  catch { /* the only safe external result remains the fixed indeterminate response */ }
  return indeterminate();
}

function responseOf(decision: Exclude<SupervisorRegistrationDecisionV1, AppendDecisionV1>) {
  return decision.response;
}

function isAppendDecision(
  decision: SupervisorRegistrationDecisionV1,
): decision is AppendDecisionV1 {
  return decision.decisionKind === 'append-registration-candidate'
    || decision.decisionKind === 'append-changed-replay-candidate';
}

function notAdmitted(): SupervisorRegistrationTransactionResultV1 {
  return result(fixedRegistrationTransportResponseV2('registration-not-admitted-v2'));
}

function indeterminate(): SupervisorRegistrationTransactionResultV1 {
  return result(fixedRegistrationTransportResponseV2('transaction-resolution-unknown-v2'));
}

function result(response: RegistrationResponseV1): SupervisorRegistrationTransactionResultV1 {
  return deepFreeze({ authority: 'none', mutationAuthorized: false, response });
}

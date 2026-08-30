// SPDX-License-Identifier: MIT
import { deepFreeze } from './closed-json.js';
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
  type FixedRegistrationTransportResponseV2,
} from './registration-protocol-v2.js';
import {
  capabilityRecordV1,
  captureCapabilityMethodV1,
  closeRegistrationAdapterOperationV1,
  stagedResponseMatchesCandidateV1,
} from './registration-transaction-boundary-v1.js';
import {
  captureRegistrationPeerConsumerV1,
  consumeRegistrationPeerV1,
  preclassifyRegistrationRequestV1,
} from './registration-transaction-admission-v1.js';
import {
  captureExactRecoveryCheckoutV1,
  captureRegistrationCheckoutV1,
  openExactRecoveryCheckoutV1,
  openRegistrationCheckoutV1,
} from './registration-transaction-checkout-v1.js';
import type {
  AuthenticatedExactRecoveryPeerRegistryV1,
  AuthenticatedExactRecoveryPeerV1,
  AuthenticatedRegistrationPeerRegistryV1,
  SupervisorRegistrationRecoveryPortsV1,
  SupervisorRegistrationRecoveryStoreV1,
  SupervisorRegistrationTransactionStoreV1,
} from './registration-transaction-contract-v1.js';
import {
  commitSupervisorRegistrationAttemptV1,
  isSupervisorRegistrationRetryableAbortV1,
  rollbackSupervisorRegistrationAttemptV1,
  runBoundedSupervisorRegistrationAttemptsV1,
  type SupervisorRegistrationCommitResolutionV1,
  type SupervisorRegistrationRetryAttemptV1,
} from './registration-transaction-retry-v1.js';

type AppendDecisionV1 = Extract<SupervisorRegistrationDecisionV1, Readonly<{
  decisionKind: 'append-registration-candidate' | 'append-changed-replay-candidate';
}>>;
type ExactResponseDecisionV1 = Extract<SupervisorRegistrationExactPhaseDecisionV1,
  Readonly<{ decisionKind: 'exact-response' }>>;
export type {
  AuthenticatedExactRecoveryPeerRegistryV1,
  AuthenticatedExactRecoveryPeerV1,
  AuthenticatedRegistrationPeerRegistryV1,
  SupervisorRegistrationCommitResolutionV1,
  SupervisorRegistrationRecoveryCheckoutV1,
  SupervisorRegistrationRecoveryPortsV1,
  SupervisorRegistrationRecoveryStoreV1,
  SupervisorRegistrationRecoveryTransactionV1,
  SupervisorRegistrationTransactionCheckoutV1,
  SupervisorRegistrationTransactionStoreV1,
  SupervisorRegistrationTransactionV1,
} from './registration-transaction-contract-v1.js';

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
  retryRequested(): boolean;
}>;

type BoundRecoveryTransactionV1 = Readonly<{
  ports: SupervisorRegistrationRecoveryPortsV1;
  commit(): Promise<SupervisorRegistrationCommitResolutionV1>;
  rollback(): Promise<void>;
  quarantine(): Promise<void>;
  retryRequested(): boolean;
}>;

type TransactionAttemptResultV1 =
  | SupervisorRegistrationTransactionResultV1
  | SupervisorRegistrationRetryAttemptV1;

export async function executeSupervisorRegistrationTransactionV1(
  serializedRequest: string,
  authenticatedPeer: AuthenticatedTransportPeerV1,
  peerRegistry: AuthenticatedRegistrationPeerRegistryV1,
  store: SupervisorRegistrationTransactionStoreV1,
): Promise<SupervisorRegistrationTransactionResultV1> {
  const consumePeer = captureRegistrationPeerConsumerV1(
    peerRegistry, 'consumeRegistration', 'registration peer registry',
  );
  const checkoutRegistration = captureRegistrationCheckoutV1(store);
  const requestAdmission = await preclassifyRegistrationRequestV1(serializedRequest);
  if (requestAdmission === false) return notAdmitted();
  if (requestAdmission === null) return indeterminate();
  if (consumePeer === null || checkoutRegistration === null) return indeterminate();
  const admission = consumeRegistrationPeerV1(authenticatedPeer, consumePeer);
  if (admission === false) return notAdmitted();
  if (admission === null) return indeterminate();
  return runBoundedSupervisorRegistrationAttemptsV1(
    () => executeRegistrationAttempt(
      serializedRequest, authenticatedPeer, checkoutRegistration,
    ),
    indeterminate,
  );
}

async function executeRegistrationAttempt(
  serializedRequest: string,
  authenticatedPeer: AuthenticatedTransportPeerV1,
  checkoutRegistration: SupervisorRegistrationTransactionStoreV1['checkoutRegistration'],
): Promise<TransactionAttemptResultV1> {
  const transaction = await openRegistrationCheckoutV1(
    checkoutRegistration, bindTransaction,
  );
  if (transaction === null) return indeterminate();

  let decision: SupervisorRegistrationDecisionV1;
  try {
    decision = await decideSupervisorRegistrationV1(
      serializedRequest, authenticatedPeer, transaction.ports,
    );
  } catch (error) {
    return rollbackIndeterminate(
      transaction, isSupervisorRegistrationRetryableAbortV1(error),
    );
  }
  if (transaction.retryRequested()) return rollbackIndeterminate(transaction, true);
  if (!isAppendDecision(decision)) {
    return commitResponse(transaction, responseOf(decision));
  }

  try {
    await transaction.stageCandidate(decision.candidate);
  } catch (error) {
    return rollbackIndeterminate(
      transaction, isSupervisorRegistrationRetryableAbortV1(error),
    );
  }

  let staged: SupervisorRegistrationExactPhaseDecisionV1;
  try {
    staged = await decideSupervisorRegistrationExactPhaseV1(
      serializedRequest, authenticatedPeer, transaction.ports,
    );
  } catch (error) {
    return rollbackIndeterminate(
      transaction, isSupervisorRegistrationRetryableAbortV1(error),
    );
  }
  if (transaction.retryRequested()) return rollbackIndeterminate(transaction, true);
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
  const consumePeer = captureRegistrationPeerConsumerV1(
    peerRegistry, 'consumeExactRecovery', 'exact recovery peer registry',
  );
  const checkoutExactRecovery = captureExactRecoveryCheckoutV1(store);
  const requestAdmission = await preclassifyRegistrationRequestV1(serializedRequest);
  if (requestAdmission === false) return notAdmitted();
  if (requestAdmission === null) return indeterminate();
  if (consumePeer === null || checkoutExactRecovery === null) return indeterminate();
  const admission = consumeRegistrationPeerV1(authenticatedPeer, consumePeer);
  if (admission === false) return notAdmitted();
  if (admission === null) return indeterminate();
  return runBoundedSupervisorRegistrationAttemptsV1(
    () => executeRecoveryAttempt(
      serializedRequest, authenticatedPeer, checkoutExactRecovery,
    ),
    indeterminate,
  );
}

async function executeRecoveryAttempt(
  serializedRequest: string,
  authenticatedPeer: AuthenticatedExactRecoveryPeerV1,
  checkoutExactRecovery: SupervisorRegistrationRecoveryStoreV1['checkoutExactRecovery'],
): Promise<TransactionAttemptResultV1> {
  const transaction = await openExactRecoveryCheckoutV1(
    checkoutExactRecovery, bindRecoveryTransaction,
  );
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
  } catch (error) {
    return rollbackIndeterminate(
      transaction, isSupervisorRegistrationRetryableAbortV1(error),
    );
  }
  if (transaction.retryRequested()) return rollbackIndeterminate(transaction, true);
  return commitResponse(
    transaction,
    decision.decisionKind === 'exact-response' ? decision.response
      : fixedRegistrationTransportResponseV2('transaction-resolution-unknown-v2'),
  );
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
  const requiredRetryQuarantine = async (error: unknown) => {
    try { await quarantine(); }
    catch { return 'unknown' as const; }
    throw error;
  };
  let lastExactRead: unknown = null;
  let retryRequested = false;
  let state: 'open' | 'operation' | 'staged' | 'terminal' = 'open';
  const invokePort = async <T>(operation: () => Promise<T>): Promise<T> => {
    const previous = state;
    if (previous !== 'open' && previous !== 'staged') {
      throw new TypeError('registration transaction is busy or closed');
    }
    state = 'operation';
    try { return await closeRegistrationAdapterOperationV1(operation); }
    catch (error) {
      if (!isSupervisorRegistrationRetryableAbortV1(error)) throw error;
      retryRequested = true;
      return Object.freeze({ kind: 'indeterminate' }) as T;
    }
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
      catch (error) {
        if (isSupervisorRegistrationRetryableAbortV1(error)) {
          return requiredRetryQuarantine(error);
        }
        await bestEffortQuarantine();
        throw error;
      }
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
    retryRequested: () => retryRequested,
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
  const requiredRetryQuarantine = async (error: unknown) => {
    try { await quarantine(); }
    catch { return 'unknown' as const; }
    throw error;
  };
  let retryRequested = false;
  let state: 'open' | 'operation' | 'terminal' = 'open';
  const invokePort = async <T>(operation: () => Promise<T>): Promise<T> => {
    if (state !== 'open') throw new TypeError('registration recovery is busy or closed');
    state = 'operation';
    try { return await closeRegistrationAdapterOperationV1(operation); }
    catch (error) {
      if (!isSupervisorRegistrationRetryableAbortV1(error)) throw error;
      retryRequested = true;
      return Object.freeze({ kind: 'indeterminate' }) as T;
    }
    finally { if (state === 'operation') state = 'open'; }
  };
  return Object.freeze({
    ports: bindRecoveryPorts(record.ports, invokePort),
    commit: async () => {
      if (state !== 'open') throw new TypeError('registration recovery cannot commit');
      state = 'terminal';
      let resolution: unknown;
      try { resolution = await commit(); }
      catch (error) {
        if (isSupervisorRegistrationRetryableAbortV1(error)) {
          return requiredRetryQuarantine(error);
        }
        await bestEffortQuarantine();
        throw error;
      }
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
    retryRequested: () => retryRequested,
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
): Promise<TransactionAttemptResultV1> {
  return commitSupervisorRegistrationAttemptV1(
    transaction, response, result, indeterminate,
  );
}

async function rollbackIndeterminate(
  transaction: Readonly<{ rollback(): Promise<void> }>,
  retry = false,
): Promise<TransactionAttemptResultV1> {
  return rollbackSupervisorRegistrationAttemptV1(
    transaction, retry, indeterminate,
  );
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

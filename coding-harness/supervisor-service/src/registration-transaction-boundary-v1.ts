// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import {
  asRecord, cloneClosedRecord, parseCanonicalPrettyJson, sha256CanonicalValue,
} from './closed-json.js';
import { REGISTRATION_RESULT_MAX_BYTES_V2 } from './registration-protocol-v2.js';

export async function closeRegistrationAdapterOperationV1<T>(
  operation: () => Promise<T>,
): Promise<T> {
  const value = await operation();
  assertProxyFreeRecordGraph(value);
  return cloneClosedRecord(value, 'transaction-bound registration adapter result') as T;
}

export function captureCapabilityMethodV1(
  value: unknown,
  expected: readonly string[],
  label: string,
  key: string,
): (...args: readonly unknown[]) => any {
  const record = capabilityRecordV1(value, expected, label);
  const method = record[key];
  if (typeof method !== 'function' || isProxy(method)) {
    throw new TypeError(`${label}.${key} is invalid`);
  }
  return (...args: readonly unknown[]) => Reflect.apply(method, undefined, args);
}

export async function stagedResponseMatchesCandidateV1(
  responseBody: string,
  candidateValue: Readonly<Record<string, unknown>>,
  exactReadValue: unknown,
): Promise<boolean> {
  try {
    const result = parseCanonicalPrettyJson(
      responseBody, REGISTRATION_RESULT_MAX_BYTES_V2, 'staged registration result',
    );
    const envelope = parseCanonicalPrettyJson(
      String(result.serializedEventEnvelope),
      REGISTRATION_RESULT_MAX_BYTES_V2,
      'staged registration event envelope',
    );
    const event = asRecord(envelope.event, 'staged registration event');
    const candidate = asRecord(candidateValue, 'staged registration candidate');
    const request = asRecord(candidate.request, 'staged registration candidate request');
    const project = asRecord(candidate.project, 'staged registration candidate project');
    const exactRead = asRecord(exactReadValue, 'staged exact result');
    if (exactRead.kind !== 'found') return false;
    const row = asRecord(exactRead.row, 'staged exact result row');
    if (candidate.transactionScope !== 'same-serializable-transaction-required'
      || event.eventKind !== candidate.candidateKind
      || event.globalSequence !== candidate.expectedNextGlobalSequence
      || event.runSequence !== candidate.runSequence
      || event.priorControllerStateHeadDigest !== candidate.priorControllerStateHeadDigest
      || event.resourceTransition !== candidate.resourceTransition
      || event.runId !== request.runId
      || event.semanticRequestDigest !== request.semanticRequestDigest) return false;
    const eventProject = asRecord(event.project, 'staged registration event project');
    if (eventProject.projectAuthorityDigest !== project.projectAuthorityDigest
      || eventProject.principalId !== project.principalId) return false;
    if (row.serializedResponse !== responseBody
      || row.projectAuthorityDigest !== project.projectAuthorityDigest
      || row.semanticRequestDigest !== request.semanticRequestDigest
      || row.responseStatus !== result.responseStatus
      || row.responseContentType !== result.responseContentType
      || !rowProvenanceMatchesCandidate(row, event, candidate, request)) return false;
    const comparisons = await Promise.all([
      sameClosedValue(event.authorityHead, candidate.authorityHead),
      sameClosedValue(event.previousGlobal, candidate.previousGlobal),
      sameClosedValue(event.previousRun, candidate.previousRun),
      sameClosedValue(event.body, candidate.body),
    ]);
    return comparisons.every(Boolean);
  } catch {
    return false;
  }
}

function rowProvenanceMatchesCandidate(
  row: Record<string, unknown>,
  event: Record<string, unknown>,
  candidate: Record<string, unknown>,
  request: Record<string, unknown>,
): boolean {
  const expectedRun = asRecord(candidate.expectedRunState, 'staged expected run state');
  if (candidate.candidateKind === 'claim-registered-v2') {
    return expectedRun.kind === 'absent'
      && row.originalRegistrationRequestDigest === request.semanticRequestDigest
      && row.originalRegistrationEventDigest === event.eventDigest
      && row.originalRegistrationGlobalSequence === candidate.expectedNextGlobalSequence
      && row.changedReplayPriorControllerStateHeadDigest === null;
  }
  return candidate.candidateKind === 'capture-run-terminal-v2'
    && expectedRun.kind === 'registered'
    && row.originalRegistrationRequestDigest === expectedRun.originalRegistrationRequestDigest
    && row.originalRegistrationEventDigest === expectedRun.registrationEventDigest
    && row.originalRegistrationGlobalSequence === expectedRun.lastRunGlobalSequence
    && row.changedReplayPriorControllerStateHeadDigest
      === expectedRun.currentControllerStateHeadDigest;
}

export function capabilityRecordV1(
  value: unknown,
  expected: readonly string[],
  label: string,
): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value) || isProxy(value)
    || Object.getPrototypeOf(value) !== Object.prototype) {
    throw new TypeError(`${label} capability is invalid`);
  }
  const descriptors = Object.getOwnPropertyDescriptors(value);
  if (Reflect.ownKeys(descriptors).some((key) => typeof key !== 'string')
    || JSON.stringify(Object.keys(descriptors).sort())
    !== JSON.stringify([...expected].sort())) {
    throw new TypeError(`${label} capability keys are invalid`);
  }
  for (const key of expected) {
    const descriptor = descriptors[key];
    if (descriptor === undefined || !Object.hasOwn(descriptor, 'value')
      || descriptor.enumerable !== true) {
      throw new TypeError(`${label} capability property is invalid`);
    }
  }
  return value as Record<string, unknown>;
}

function assertProxyFreeRecordGraph(value: unknown): void {
  let nodes = 0;
  const seen = new Set<object>();
  const visit = (current: unknown, depth: number): void => {
    if (depth > 24 || ++nodes > 2_048) throw new TypeError('adapter result graph is unbounded');
    if (current === null || typeof current !== 'object') return;
    if (isProxy(current)) throw new TypeError('adapter result must not contain a Proxy');
    if (Array.isArray(current) || seen.has(current)
      || Object.getPrototypeOf(current) !== Object.prototype
      || Object.getOwnPropertySymbols(current).length !== 0) {
      throw new TypeError('adapter result must be an acyclic plain-record graph');
    }
    seen.add(current);
    for (const descriptor of Object.values(Object.getOwnPropertyDescriptors(current))) {
      if (!descriptor.enumerable || !Object.hasOwn(descriptor, 'value')) {
        throw new TypeError('adapter result must contain enumerable data properties only');
      }
      visit(descriptor.value, depth + 1);
    }
    seen.delete(current);
  };
  visit(value, 0);
}

async function sameClosedValue(left: unknown, right: unknown): Promise<boolean> {
  return await sha256CanonicalValue(left) === await sha256CanonicalValue(right);
}

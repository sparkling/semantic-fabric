// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import {
  decideSupervisorRegistrationV1,
  parseCanonicalRegistrationRequestV2,
  type AuthenticatedTransportPeerV1,
  type SupervisorRegistrationDecisionPortsV1,
} from '../src/index.js';
import {
  ACTIVE_HEAD, ACTIVE_SEMANTIC_HEAD, CANONICAL_REQUEST, DIGEST, PROJECT,
  READY_RECEIPT, READY_SEMANTIC_RECEIPT, REGISTERED_RUN,
  canonical, coherentlyMutatedRegistrationEnvelope, exactStoredResult,
  registrationEnvelope, sha256Text,
} from './registration-fixtures.js';

const PEER = Symbol('authenticated-test-peer') as AuthenticatedTransportPeerV1;
const MAPPED = Object.freeze({ kind: 'mapped' as const, project: PROJECT });
type Step = Readonly<{
  name: string; reply?: unknown; error?: Error; before?: () => void;
}>;
function scripted(steps: readonly Step[]) {
  const queue = [...steps];
  const calls: Array<{ name: string; input: unknown }> = [];
  const call = async <T>(name: string, input: unknown): Promise<T> => {
    calls.push({ name, input });
    const step = queue.shift();
    if (step?.name !== name) throw new Error(`unexpected ${name}; expected ${step?.name}`);
    step.before?.();
    if (step.error !== undefined) throw step.error;
    return step.reply as T;
  };
  const ports: SupervisorRegistrationDecisionPortsV1 = {
    mapAuthenticatedPeer: (peer) => call('map', peer),
    lookupExactCommittedResult: (input) => call('exact', input),
    readActiveAuthorityHead: (input) => call('head', input),
    readRequiredPredecessorReceipt: (input) => call('receipt', input),
    readRunState: (input) => call('run', input),
  };
  return { ports, calls, remaining: () => queue.length };
}

function fullSteps(run: unknown): Step[] {
  const { head, receipt } = stateReads(run);
  return [
    { name: 'map', reply: MAPPED },
    { name: 'exact', reply: { kind: 'absent' } },
    { name: 'head', reply: head },
    { name: 'receipt', reply: receipt },
    { name: 'run', reply: run },
  ];
}

function stateReads(run: unknown): { head: unknown; receipt: unknown } {
  const state = run as { kind?: unknown; lastRunGlobalSequence?: unknown;
    lastRunSequence?: unknown; lastRunEventDigest?: unknown };
  if (state?.kind === 'absent') return { head: ACTIVE_HEAD, receipt: READY_RECEIPT };
  if (state?.lastRunSequence === '0'
    && state.lastRunEventDigest === REGISTERED_RUN.registrationEventDigest) {
    return { head: ACTIVE_SEMANTIC_HEAD, receipt: READY_SEMANTIC_RECEIPT };
  }
  const expectedNextGlobalSequence =
    (BigInt(String(state.lastRunGlobalSequence)) + 1n).toString();
  const requiredPredecessor = {
    kind: 'semantic-event' as const, eventDigest: state.lastRunEventDigest,
  };
  return {
    head: { ...ACTIVE_SEMANTIC_HEAD, expectedNextGlobalSequence, requiredPredecessor },
    receipt: {
      kind: 'ready',
      previousGlobal: {
        ...requiredPredecessor,
        semanticReceiptDigest: sha256Text(`receipt:${expectedNextGlobalSequence}`),
      },
    },
  };
}

function fixedOutcome(decision: Awaited<ReturnType<typeof decideSupervisorRegistrationV1>>) {
  return decision.decisionKind === 'fixed-response' || decision.decisionKind === 'indeterminate'
    ? decision.response.outcomeCode : null;
}

function expectDeepFrozen(value: unknown): void {
  if (value === null || typeof value !== 'object') return;
  expect(Object.isFrozen(value)).toBe(true);
  for (const nested of Object.values(value as Record<string, unknown>)) {
    expectDeepFrozen(nested);
  }
}

describe('duplicate-first supervisor registration decision kernel V1', () => {
  it.each([201, 409] as const)(
    'returns byte-identical stored %s before current-head inspection', async (status) => {
      const stored = exactStoredResult(status);
      const { ports, calls, remaining } = scripted([
        { name: 'map', reply: MAPPED }, { name: 'exact', reply: stored },
      ]);

      const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

      expect(decision).toMatchObject({
        decisionKind: 'exact-response', authority: 'none', mutationAuthorized: false,
        response: {
          status,
          contentType: 'application/json; charset=utf-8',
          body: stored.row.serializedResponse,
        },
      });
      expect(calls.map(({ name }) => name)).toEqual(['map', 'exact']);
      expect(remaining()).toBe(0);
      expect(Object.isFrozen(decision)).toBe(true);
      expect(decision.decisionKind).toBe('exact-response');
      if (decision.decisionKind !== 'exact-response') throw new Error('exact response required');
      expect(Object.isFrozen(decision.response)).toBe(true);
    },
  );

  it('allows project-scoped exact recovery after principal rotation', async () => {
    const rotated = {
      kind: 'mapped',
      project: { ...PROJECT, principalId: 'project_client_rotated_20260830' },
    };
    const stored = exactStoredResult(201);
    const { ports, calls } = scripted([
      { name: 'map', reply: rotated }, { name: 'exact', reply: stored },
    ]);

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

    expect(decision.decisionKind).toBe('exact-response');
    expect(calls.map(({ name }) => name)).toEqual(['map', 'exact']);
    expect(calls[1]?.input).toEqual({
      projectAuthorityDigest: DIGEST.project,
      semanticRequestDigest: DIGEST.request,
    });
  });

  it('uses the authenticated project for lookup and rejects a request-selected project', async () => {
    const mappedProject = {
      ...PROJECT,
      projectAuthorityDigest: sha256Text('authenticated-project-b'),
    };
    const { ports, calls } = scripted([
      { name: 'map', reply: { kind: 'mapped', project: mappedProject } },
      { name: 'exact', reply: { kind: 'absent' } },
    ]);

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

    expect(fixedOutcome(decision)).toBe('registration-not-admitted-v2');
    expect(calls.map(({ name }) => name)).toEqual(['map', 'exact']);
    expect(calls[1]?.input).toEqual({
      projectAuthorityDigest: mappedProject.projectAuthorityDigest,
      semanticRequestDigest: DIGEST.request,
    });
  });

  it.each([
    ['project', (found: any) => { found.row.projectAuthorityDigest = sha256Text('foreign'); }],
    ['request digest', (found: any) => { found.row.semanticRequestDigest = sha256Text('other'); }],
    ['request bytes', (found: any) => { found.row.serializedRequest += ' '; }],
    ['request hash', (found: any) => { found.row.serializedRequestSha256 = sha256Text('bad'); }],
    ['response hash', (found: any) => { found.row.serializedResponseSha256 = sha256Text('bad'); }],
    ['content type', (found: any) => { found.row.responseContentType = 'application/json'; }],
    ['status', (found: any) => { found.row.responseStatus = 418; }],
    ['response bytes', (found: any) => { found.row.serializedResponse = '{"bad":true}'; }],
  ])('fails closed on corrupt exact-result %s', async (_label, mutate) => {
    const found = structuredClone(exactStoredResult(201));
    mutate(found);
    const { ports, calls } = scripted([
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: found },
    ]);

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

    expect(decision.decisionKind).toBe('indeterminate');
    expect(fixedOutcome(decision)).toBe('transaction-resolution-unknown-v2');
    expect(calls.map(({ name }) => name)).toEqual(['map', 'exact']);
  });

  it.each([
    ['arbitrary envelope', exactStoredResult(201, {
      serializedEventEnvelope: 'not-a-canonical-event-envelope\n',
    })],
    ['oversized result', exactStoredResult(201, {
      serializedEventEnvelope: 'x'.repeat(196_609),
    })],
    ['status/event mismatch', exactStoredResult(201, {
      serializedEventEnvelope: registrationEnvelope(409),
    })],
    ['request binding', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.semanticRequestDigest = sha256Text('foreign-semantic-request');
      }),
    })],
    ['authority head binding', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.authorityHead.headDigest = sha256Text('foreign-authority-head');
      }),
    })],
    ['principal binding', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.project.principalId = 'foreign_project_principal_20260830';
      }),
    })],
    ['claim binding', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.body.claimDigest = sha256Text('foreign-claim');
      }),
    })],
    ['prior-state binding', exactStoredResult(201, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(201, (event) => {
        event.priorControllerStateHeadDigest = sha256Text('foreign-prior-state');
      }),
    })],
    ['changed-replay adjacency', exactStoredResult(409, {
      serializedEventEnvelope: coherentlyMutatedRegistrationEnvelope(409, (event) => {
        event.runSequence = '2';
      }),
    })],
  ])('rejects a self-consistent exact row with invalid %s', async (_label, stored) => {
    const { ports, calls, remaining } = scripted([
      { name: 'map', reply: MAPPED }, { name: 'exact', reply: stored },
    ]);

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

    expect(decision.decisionKind).toBe('indeterminate');
    expect(fixedOutcome(decision)).toBe('transaction-resolution-unknown-v2');
    expect(calls.map(({ name }) => name)).toEqual(['map', 'exact']);
    expect(remaining()).toBe(0);
  });

  it('collapses malformed, unmapped, and stale requests to the same fixed 403', async () => {
    const malformed = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST.slice(0, -1), PEER, scripted([]).ports,
    );
    const unmappedPorts = scripted([{ name: 'map', reply: { kind: 'not-admitted' } }]);
    const unmapped = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, unmappedPorts.ports,
    );
    const stalePorts = scripted([
      { name: 'map', reply: MAPPED },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: { ...ACTIVE_HEAD, authorityHead: {
        ...ACTIVE_HEAD.authorityHead, headDigest: sha256Text('new-head'),
      } } },
    ]);
    const stale = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, stalePorts.ports);

    expect([malformed, unmapped, stale].map(fixedOutcome)).toEqual(
      Array(3).fill('registration-not-admitted-v2'),
    );
    expect(unmappedPorts.calls.map(({ name }) => name)).toEqual(['map']);
    expect(stalePorts.calls.map(({ name }) => name)).toEqual(['map', 'exact', 'head']);
    expect((malformed as any).response.body).toBe((unmapped as any).response.body);
    expect((unmapped as any).response.body).toBe((stale as any).response.body);
  });

  it('keeps a hashing-service failure distinct from malformed input', async () => {
    const digest = vi.spyOn(globalThis.crypto.subtle, 'digest')
      .mockRejectedValueOnce(new Error('hash unavailable'));
    try {
      const decision = await decideSupervisorRegistrationV1(
        CANONICAL_REQUEST, PEER, scripted([]).ports,
      );
      expect(decision.decisionKind).toBe('indeterminate');
      expect(fixedOutcome(decision)).toBe('transaction-resolution-unknown-v2');
    } finally {
      digest.mockRestore();
    }
  });

  it('accepts only an opaque Symbol peer identity before awaiting a mapper', async () => {
    const peer = { transportPeer: true } as unknown as AuthenticatedTransportPeerV1;
    const { ports, calls } = scripted([]);

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, peer, ports);

    expect(fixedOutcome(decision)).toBe('registration-not-admitted-v2');
    expect(calls).toEqual([]);

    for (const attack of [
      Object.freeze({}),
      new Proxy(Object.freeze({}), {}),
      Object.freeze({ auth: Object.freeze({ subject: 'project-a' }) }),
    ]) {
      const attackPorts = scripted([]);
      expect(fixedOutcome(await decideSupervisorRegistrationV1(
        CANONICAL_REQUEST, attack as unknown as AuthenticatedTransportPeerV1,
        attackPorts.ports,
      ))).toBe('registration-not-admitted-v2');
      expect(attackPorts.calls).toEqual([]);
    }
  });

  it('does not expose receipt readiness before mapping, exact miss, and active head', async () => {
    const { ports, calls } = scripted([
      { name: 'map', reply: MAPPED },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: ACTIVE_HEAD },
      { name: 'receipt', reply: { kind: 'pending' } },
      { name: 'run', reply: { kind: 'absent' } },
    ]);

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

    expect(fixedOutcome(decision)).toBe('registration-authority-pending-v2');
    expect(calls.map(({ name }) => name)).toEqual([
      'map', 'exact', 'head', 'receipt', 'run',
    ]);
  });

  it('treats disagreement between authenticated mapping and head storage as indeterminate', async () => {
    const inconsistentHead = {
      ...ACTIVE_HEAD,
      project: { ...PROJECT, authenticationPolicyDigest: sha256Text('corrupt-policy') },
    };
    const { ports, calls } = scripted([
      { name: 'map', reply: MAPPED },
      { name: 'exact', reply: { kind: 'absent' } },
      { name: 'head', reply: inconsistentHead },
    ]);

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

    expect(decision.decisionKind).toBe('indeterminate');
    expect(fixedOutcome(decision)).toBe('transaction-resolution-unknown-v2');
    expect(calls.map(({ name }) => name)).toEqual(['map', 'exact', 'head']);
  });

  it('returns a frozen nonauthorizing registration candidate only for an absent run', async () => {
    const { ports, calls } = scripted(fullSteps({ kind: 'absent' }));
    const request = await parseCanonicalRegistrationRequestV2(CANONICAL_REQUEST);

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

    expect(decision).toEqual({
      decisionKind: 'append-registration-candidate',
      authority: 'none',
      mutationAuthorized: false,
      candidate: {
        transactionScope: 'same-serializable-transaction-required',
        project: PROJECT,
        authorityHead: ACTIVE_HEAD.authorityHead,
        expectedNextGlobalSequence: '1',
        previousGlobal: READY_RECEIPT.previousGlobal,
        request,
        candidateKind: 'claim-registered-v2',
        expectedRunState: { kind: 'absent' },
        runSequence: '0',
        previousRun: { kind: 'run-genesis', eventDigest: null },
        priorControllerStateHeadDigest: DIGEST.priorController,
        resourceTransition: null,
        body: request.claim,
      },
    });
    expect(decision).not.toHaveProperty('responseStatus');
    expect(decision).not.toHaveProperty('signature');
    expect(calls.map(({ name }) => name)).toEqual(['map', 'exact', 'head', 'receipt', 'run']);
    expectDeepFrozen(decision);
    for (const forbidden of [
      'globalSequence', 'signature', 'serializedEventEnvelope', 'responseStatus',
      'signer', 'committed', 'authorized',
    ]) expect((decision as any).candidate).not.toHaveProperty(forbidden);
  });

  it('derives the first changed-replay candidate only from stored and trusted state', async () => {
    const { ports } = scripted(fullSteps(REGISTERED_RUN));
    const request = await parseCanonicalRegistrationRequestV2(CANONICAL_REQUEST);
    const outcomeEvidenceDigest = sha256Text(canonical({
      domain: 'semantic-fabric/programme-capture/supervisor-registration-changed-replay-evidence-v2',
      originalRegistrationRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
      originalRegistrationEventDigest: REGISTERED_RUN.registrationEventDigest,
      changedRegistrationRequestDigest: DIGEST.request,
      project: {
        projectAuthorityDigest: PROJECT.projectAuthorityDigest,
        principalId: PROJECT.principalId,
      },
      authorityHead: ACTIVE_HEAD.authorityHead,
    }));

    const decision = await decideSupervisorRegistrationV1(CANONICAL_REQUEST, PEER, ports);

    expect(decision).toEqual({
      decisionKind: 'append-changed-replay-candidate',
      authority: 'none',
      mutationAuthorized: false,
      candidate: {
        transactionScope: 'same-serializable-transaction-required',
        project: PROJECT,
        authorityHead: ACTIVE_SEMANTIC_HEAD.authorityHead,
        expectedNextGlobalSequence: '2',
        previousGlobal: READY_SEMANTIC_RECEIPT.previousGlobal,
        request,
        candidateKind: 'capture-run-terminal-v2',
        expectedRunState: REGISTERED_RUN,
        runSequence: '1',
        previousRun: { kind: 'run-event', eventDigest: REGISTERED_RUN.registrationEventDigest },
        priorControllerStateHeadDigest: REGISTERED_RUN.currentControllerStateHeadDigest,
        resourceTransition: null,
        body: {
          terminalStage: 'registration',
          outcomeCode: 'registration-changed-replay-v2',
          registrationEventDigest: REGISTERED_RUN.registrationEventDigest,
          outcomeEvidenceDigest,
          leaseEventDigest: null,
          leaseId: null,
          fence: null,
          resourceDisposition: null,
          attemptId: null,
          captureRecordDigest: null,
          outputEnvelopeDigest: null,
          cleanupEvidenceDigest: null,
        },
      },
    });
    expectDeepFrozen(decision);
  });

  it('maps changed-replay digest failure to indeterminate instead of rejecting the call', async () => {
    const nativeDigest = globalThis.crypto.subtle.digest.bind(globalThis.crypto.subtle);
    let calls = 0;
    const digest = vi.spyOn(globalThis.crypto.subtle, 'digest').mockImplementation(
      async (...input) => {
        calls += 1;
        if (calls === 4) throw new Error('changed-replay hash unavailable');
        return nativeDigest(...input);
      },
    );
    try {
      const decision = await decideSupervisorRegistrationV1(
        CANONICAL_REQUEST, PEER, scripted(fullSteps(REGISTERED_RUN)).ports,
      );
      expect(decision.decisionKind).toBe('indeterminate');
      expect(fixedOutcome(decision)).toBe('transaction-resolution-unknown-v2');
    } finally {
      digest.mockRestore();
    }
  });

  it.each([
    ['registered exact digest', {
      ...REGISTERED_RUN, originalRegistrationRequestDigest: DIGEST.request,
    }],
    ['closed first-change digest', {
      kind: 'advanced-or-closed',
      projectAuthorityDigest: DIGEST.project,
      runId: 'capture_run_20260829',
      originalRegistrationRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
      registrationEventDigest: REGISTERED_RUN.registrationEventDigest,
      lastRunEventDigest: sha256Text('first-changed-replay-event'),
      lastRunGlobalSequence: '2',
      currentControllerStateHeadDigest: REGISTERED_RUN.currentControllerStateHeadDigest,
      lastRunSequence: '1',
      firstChangedReplayRequestDigest: DIGEST.request,
    }],
  ])('treats missing exact bytes for %s as indeterminate', async (_label, run) => {
    const observed = scripted(fullSteps(run));
    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, observed.ports,
    );
    expect(decision.decisionKind).toBe('indeterminate');
    expect(fixedOutcome(decision)).toBe('transaction-resolution-unknown-v2');
    expect(observed.calls.map(({ name }) => name)).toEqual([
      'map', 'exact', 'head', 'receipt', 'run',
    ]);
    expect(observed.remaining()).toBe(0);
  });

  it.each([null, sha256Text('first-changed-request')])(
    'returns eventless fixed 409 for a later distinct request with first-change %s',
    async (firstChangedReplayRequestDigest) => {
    const run = {
      kind: 'advanced-or-closed',
      projectAuthorityDigest: DIGEST.project,
      runId: 'capture_run_20260829',
      originalRegistrationRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
      registrationEventDigest: REGISTERED_RUN.registrationEventDigest,
      lastRunEventDigest: firstChangedReplayRequestDigest === null
        ? sha256Text('closed-run-last-event') : sha256Text('first-changed-replay-event'),
      currentControllerStateHeadDigest: REGISTERED_RUN.currentControllerStateHeadDigest,
      lastRunSequence: firstChangedReplayRequestDigest === null ? '2' : '1',
      lastRunGlobalSequence: firstChangedReplayRequestDigest === null ? '3' : '2',
      firstChangedReplayRequestDigest,
    };

    const decision = await decideSupervisorRegistrationV1(
      CANONICAL_REQUEST, PEER, scripted(fullSteps(run)).ports,
    );

    expect(decision.decisionKind).toBe('fixed-response');
    expect(fixedOutcome(decision)).toBe('registration-closed-v2');
    expect(decision).not.toHaveProperty('candidate');
    },
  );

});

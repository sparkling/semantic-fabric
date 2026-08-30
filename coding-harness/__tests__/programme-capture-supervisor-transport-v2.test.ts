// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  buildProgrammeCaptureSupervisorPublicCommitmentV2,
  parseProgrammeCaptureSupervisorPublicCommitmentBlobV2,
  parseProgrammeCaptureSupervisorPublicCommitmentLeafBytesV2,
  parseProgrammeCaptureSupervisorPublicCommitmentV2,
  programmeCaptureSupervisorPublicCommitmentDigestV2,
  programmeCaptureSupervisorPublicCommitmentLeafBytesV2,
  programmeCaptureSupervisorTransparencyLogIdentityDigestV2,
  serializeProgrammeCaptureSupervisorPublicCommitmentV2,
  PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_MAX_BYTES_V2,
} from '../src/programme-capture-supervisor-public-commitment-v2.js';
import {
  buildProgrammeCaptureSupervisorTransportResponseV2,
  parseProgrammeCaptureSupervisorTransportResponseBlobV2,
  parseProgrammeCaptureSupervisorTransportResponseV2,
  serializeProgrammeCaptureSupervisorTransportResponseV2,
  PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_CONTENT_TYPE_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_MAX_BYTES_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_OUTCOMES_V2,
} from '../src/programme-capture-supervisor-transport-response-v2.js';
import {
  validAuthorityConfiguration,
  validRunHistory,
  signedEnvelope,
  withEventDigest,
} from './programme-capture-supervisor-run-event-v2-fixtures.js';
import {
  parseProgrammeCaptureSupervisorAuthorityConfigurationV2,
  serializeProgrammeCaptureSupervisorAuthorityConfigurationV2,
  PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
} from '../src/programme-capture-supervisor-authority-config-v2.js';
import { digestValue } from '../src/receipts.js';

describe('programme capture supervisor non-semantic transport response V2', () => {
  it('closes every response to one fixed status and recovery directive', () => {
    const expected = {
      'registration-not-admitted-v2': [403, 'new-authority-bound-request-required'],
      'registration-authority-pending-v2': [
        503, 'new-authority-bound-request-after-ready',
      ],
      'registration-closed-v2': [409, 'new-run-required'],
      'transaction-resolution-unknown-v2': [500, 'exact-result-lookup-only'],
    } as const;

    for (const outcomeCode of PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_OUTCOMES_V2) {
      const response = buildProgrammeCaptureSupervisorTransportResponseV2({ outcomeCode });
      const serialized = serializeProgrammeCaptureSupervisorTransportResponseV2(response);
      expect(parseProgrammeCaptureSupervisorTransportResponseBlobV2(serialized))
        .toEqual(response);
      expect([response.responseStatus, response.recoveryDirective])
        .toEqual(expected[outcomeCode]);
      expect(response.responseContentType)
        .toBe(PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_CONTENT_TYPE_V2);
      expect(Object.isFrozen(response)).toBe(true);
    }
  });

  it('discloses no project, principal, run, request, event, or authority-head identity', () => {
    const forbidden = [
      'project', 'principal', 'runId', 'requestDigest', 'eventDigest', 'authorityHead',
      'configurationDigest', 'configurationEpoch', 'headDigest', 'resultDigest',
      'serializedEventEnvelope',
    ];
    for (const outcomeCode of PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_OUTCOMES_V2) {
      const response = buildProgrammeCaptureSupervisorTransportResponseV2({ outcomeCode });
      expect(Object.keys(response)).toEqual([
        'schemaVersion', 'transactionKind', 'responseKind', 'outcomeCode',
        'responseStatus', 'responseContentType', 'recoveryDirective',
      ]);
      for (const key of forbidden) expect(JSON.stringify(response)).not.toContain(key);
    }
  });

  it('rejects unknown fields, mismatched mappings, proxies, and noncanonical bytes', () => {
    const response = buildProgrammeCaptureSupervisorTransportResponseV2({
      outcomeCode: 'registration-closed-v2',
    });
    const serialized = serializeProgrammeCaptureSupervisorTransportResponseV2(response);
    expect(() => buildProgrammeCaptureSupervisorTransportResponseV2({
      outcomeCode: 'registration-closed-v2', runId: 'capture_run_secret',
    })).toThrow(/invalid keys/);
    expect(() => buildProgrammeCaptureSupervisorTransportResponseV2(new Proxy({
      outcomeCode: 'registration-closed-v2',
    }, {}))).toThrow(/Proxy/);
    expect(() => parseProgrammeCaptureSupervisorTransportResponseV2({
      ...response, responseStatus: 503,
    })).toThrow(/MAPPING/);
    expect(() => parseProgrammeCaptureSupervisorTransportResponseBlobV2(
      serialized.slice(0, -1),
    )).toThrow(/CANONICAL/);
    expect(() => parseProgrammeCaptureSupervisorTransportResponseBlobV2(
      serialized.replace('{\n', '{\n  "schemaVersion": 2,\n'),
    )).toThrow(/duplicate/i);
    expect(() => parseProgrammeCaptureSupervisorTransportResponseBlobV2(
      'x'.repeat(PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_MAX_BYTES_V2 + 1),
    )).toThrow(/CANONICAL/);
  });

  it('pins the exact fixed bytes for every disclosure-free outcome', () => {
    expect(Object.fromEntries(
      PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_OUTCOMES_V2.map((outcomeCode) => [
        outcomeCode,
        sha256(serializeProgrammeCaptureSupervisorTransportResponseV2(
          buildProgrammeCaptureSupervisorTransportResponseV2({ outcomeCode }),
        )),
      ]),
    )).toEqual({
      'registration-not-admitted-v2':
        '6e9a9390950005c4e4b1a5b3435278b34d07171fb00e835be5ec4babcdb4cf49',
      'registration-authority-pending-v2':
        '3380a4e31017edae8206a0616e7e928e662208ba44ca891332e9de658166dae9',
      'registration-closed-v2':
        '11fbdd37a703d190be8d30db1582ffc7843f686238e6ee0fec88e9adf7fce0da',
      'transaction-resolution-unknown-v2':
        'e23e32a1abfdce0031973a46f6af37fb6099e9f75e4d7128b885992b0c4742f6',
    });
  });
});

describe('programme capture supervisor public commitment V2', () => {
  it('round-trips the closed privacy-minimized public commitment', () => {
    const configuration = validAuthorityConfiguration();
    const event = validRunHistory()[0];
    const commitment = buildProgrammeCaptureSupervisorPublicCommitmentV2({
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(configuration),
      serializedEventEnvelope: signedEnvelope(event),
    });
    const serialized = serializeProgrammeCaptureSupervisorPublicCommitmentV2(commitment);
    expect(parseProgrammeCaptureSupervisorPublicCommitmentBlobV2(serialized))
      .toEqual(commitment);
    expect(Object.keys(commitment)).toEqual([
      'schemaVersion', 'transactionKind', 'leafKind', 'logIdentityDigest', 'eventDigest',
    ]);
    expect(programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment))
      .toEqual(Buffer.from(canonicalOracle({
        domain: PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2,
        commitment,
      }), 'utf8'));
    expect(parseProgrammeCaptureSupervisorPublicCommitmentLeafBytesV2(
      programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment),
    )).toEqual(commitment);
    const leafBytes = programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment);
    expect(leafBytes.at(-1)).not.toBe(0x0a);
    expect(leafBytes.toString('utf8')).toContain(
      PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2,
    );
    for (const privateValue of [
      'capture_run_20260829', 'project_client_20260829', 'supervisor_service_20260829',
      'controlled_runner_20260829', 'claim-registered-v2',
    ]) expect(leafBytes.toString('utf8')).not.toContain(privateValue);
    expect(Object.isFrozen(commitment)).toBe(true);
  });

  it('derives identity and event only from matching canonical authority and envelope bytes', () => {
    const configuration = validAuthorityConfiguration();
    const event = validRunHistory()[0];
    const mismatched = withEventDigest({
      ...event,
      authorityHead: {
        ...event.authorityHead,
        configurationDigest: digest('different-authority-configuration'),
      },
    });
    expect(() => buildProgrammeCaptureSupervisorPublicCommitmentV2({
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(configuration),
      serializedEventEnvelope: signedEnvelope(mismatched),
    })).toThrow(/AUTHORITY_MISMATCH/);
  });

  it('derives one stable log identity from every log field and no unrelated field', () => {
    const configuration = validAuthorityConfiguration();
    const original = programmeCaptureSupervisorTransparencyLogIdentityDigestV2(configuration);
    const mutations: Array<(value: any) => void> = [
      (value) => { value.transparencyLog.principal.principalId = 'rotated_log_20260830'; },
      (value) => { value.transparencyLog.principal.keyEpoch = '2'; },
      (value) => { value.transparencyLog.principal.keyFingerprint = digest('rotated-log-key'); },
      (value) => { value.transparencyLog.principal.policyDigest = digest('rotated-log-policy'); },
      (value) => {
        value.transparencyLog.principal.administrationDigest = digest('rotated-log-admin');
      },
      (value) => { value.transparencyLog.endpointOrigin = 'https://rotated-log.example.org'; },
      (value) => { value.transparencyLog.tlsSpkiFingerprint = digest('rotated-log-tls'); },
      (value) => {
        value.transparencyLog.publicCommitmentPolicyDigest = digest('rotated-commitment-policy');
      },
    ];
    for (const mutate of mutations) {
      expect(programmeCaptureSupervisorTransparencyLogIdentityDigestV2(
        rehashAuthorityConfiguration(configuration, mutate),
      )).not.toBe(original);
    }
    expect(programmeCaptureSupervisorTransparencyLogIdentityDigestV2(
      rehashAuthorityConfiguration(configuration, (value) => {
        value.readinessPolicyDigest = digest('unrelated-readiness-policy');
      }),
    )).toBe(original);
  });

  it('publishes the same closed fields for all six private semantic event kinds', () => {
    const configuration = validAuthorityConfiguration();
    const events = [
      ...validRunHistory(),
      validRunHistory({ preStartTerminal: 'registration' }).at(-1)!,
    ];
    expect(new Set(events.map(({ eventKind }) => eventKind)).size).toBe(6);
    for (const event of events) {
      const commitment = buildProgrammeCaptureSupervisorPublicCommitmentV2({
        serializedAuthorityConfiguration:
          serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(configuration),
        serializedEventEnvelope: signedEnvelope(event),
      });
      expect(commitment.eventDigest).toBe(event.eventDigest);
      expect(Object.keys(commitment)).toEqual([
        'schemaVersion', 'transactionKind', 'leafKind', 'logIdentityDigest', 'eventDigest',
      ]);
      expect(programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment)
        .toString('utf8')).not.toContain(event.eventKind);
    }
  });

  it('rejects private fields, unknown fields, zero digests, proxies, and noncanonical bytes', () => {
    const input = {
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(
          validAuthorityConfiguration(),
        ),
      serializedEventEnvelope: signedEnvelope(validRunHistory()[0]),
    };
    const commitment = buildProgrammeCaptureSupervisorPublicCommitmentV2(input);
    const serialized = serializeProgrammeCaptureSupervisorPublicCommitmentV2(commitment);
    for (const forbidden of [
      'projectAuthorityDigest', 'runId', 'runnerId', 'sessionId', 'leaseId', 'fence',
      'semanticRequestDigest', 'globalSequence', 'runSequence', 'nonce', 'timestamp',
      'claimDigest', 'captureRecordDigest', 'outputEnvelopeDigest', 'credential',
    ]) {
      expect(() => buildProgrammeCaptureSupervisorPublicCommitmentV2({
        ...input, [forbidden]: 'secret',
      })).toThrow(/invalid keys/);
      expect(serialized).not.toContain(forbidden);
    }
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentV2({
      ...commitment, eventDigest: '0'.repeat(64),
    })).toThrow(/non-zero/);
    expect(() => buildProgrammeCaptureSupervisorPublicCommitmentV2(
      new Proxy(input, {}),
    )).toThrow(/Proxy/);
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentV2({
      ...commitment, leafKind: 'another-kind',
    })).toThrow(/IDENTITY/);
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentBlobV2(
      serialized.slice(0, -1),
    )).toThrow(/CANONICAL/);
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentBlobV2(
      serialized.replace('{\n', '{\n  "schemaVersion": 2,\n'),
    )).toThrow(/duplicate/i);
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentBlobV2(
      'x'.repeat(PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_MAX_BYTES_V2 + 1),
    )).toThrow(/CANONICAL/);

    const leafBytes = programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment);
    const leafText = leafBytes.toString('utf8');
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentLeafBytesV2(
      Buffer.concat([leafBytes, Buffer.from('\n')]),
    )).toThrow(/CANONICAL/);
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentLeafBytesV2(
      Buffer.from(`\ufeff${leafText}`),
    )).toThrow();
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentLeafBytesV2(
      Buffer.from([0xff]),
    )).toThrow(/UTF-8/);
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentLeafBytesV2(
      Buffer.from(canonicalOracle({
        domain: PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2,
        commitment,
        projectAuthorityDigest: digest('forbidden-project'),
      })),
    )).toThrow(/invalid keys/);
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentLeafBytesV2(
      Buffer.from(leafText.replace(
        `"domain":"${PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2}"`,
        `"domain":"${PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2}",`
          + `"domain":"${PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2}"`,
      )),
    )).toThrow(/duplicate/i);
    expect(() => parseProgrammeCaptureSupervisorPublicCommitmentLeafBytesV2(
      new Proxy(leafBytes, {}),
    )).toThrow(/Uint8Array/);
  });

  it('pins independent public-leaf byte and domain-separated digest oracles', () => {
    const configuration = validAuthorityConfiguration();
    const commitment = buildProgrammeCaptureSupervisorPublicCommitmentV2({
      serializedAuthorityConfiguration:
        serializeProgrammeCaptureSupervisorAuthorityConfigurationV2(configuration),
      serializedEventEnvelope: signedEnvelope(validRunHistory()[0]),
    });
    const independentDigest = sha256(canonicalOracle({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_PUBLIC_COMMITMENT_DOMAIN_V2,
      commitment,
    }));
    const leafBytes = programmeCaptureSupervisorPublicCommitmentLeafBytesV2(commitment);
    expect(programmeCaptureSupervisorPublicCommitmentDigestV2(commitment))
      .toBe(independentDigest);
    expect(programmeCaptureSupervisorPublicCommitmentDigestV2(commitment))
      .toBe(sha256(leafBytes));
    expect(commitment.logIdentityDigest).toBe(
      programmeCaptureSupervisorTransparencyLogIdentityDigestV2(configuration),
    );
    expect({
      logIdentityDigest: commitment.logIdentityDigest,
      commitmentDigest: independentDigest,
      leafBytesLength: leafBytes.byteLength,
      leafBytesBase64: leafBytes.toString('base64'),
      rfc9162LeafHash: sha256(Buffer.concat([Buffer.from([0]), leafBytes])),
    }).toEqual({
      logIdentityDigest: '457af0bd4bff644fe3b0bb18b9df0e54ff7db60456bd23c9903cd83d169f1125',
      commitmentDigest: '2377d2981ed8a0ba944a9b92a9055a1c6dea506e7ac03eec92a090318931c457',
      leafBytesLength: 377,
      leafBytesBase64:
        'eyJjb21taXRtZW50Ijp7ImV2ZW50RGlnZXN0IjoiNzQ2YmMyMDk2M2JjZjg0ODE3OTA1YzM4YjkzZWU2NzExMWVhMTYyMDk4YjFkYjRlYTM1ZjRmMTBjMWI0OWZmZiIsImxlYWZLaW5kIjoicHJvZ3JhbW1lLWNhcHR1cmUtZXZlbnQtY29tbWl0bWVudC12MiIsImxvZ0lkZW50aXR5RGlnZXN0IjoiNDU3YWYwYmQ0YmZmNjQ0ZmUzYjBiYjE4YjlkZjBlNTRmZjdkYjYwNDU2YmQyM2M5OTAzY2Q4M2QxNjlmMTEyNSIsInNjaGVtYVZlcnNpb24iOjIsInRyYW5zYWN0aW9uS2luZCI6InByb2dyYW1tZS1jYXB0dXJlLXYyIn0sImRvbWFpbiI6InNlbWFudGljLWZhYnJpYy9wcm9ncmFtbWUtY2FwdHVyZS9zdXBlcnZpc29yLXB1YmxpYy1ldmVudC1jb21taXRtZW50LXYyIn0=',
      rfc9162LeafHash: '6978e1c1bc97c77be617caa58c09f4ca42ec471627e06fb7e10b41f7a2a4cf43',
    });
  });
});

function digest(value: string): string {
  return sha256(value);
}

function sha256(value: string | Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

function canonicalOracle(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalOracle).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    const input = value as Record<string, unknown>;
    return `{${Object.keys(input).sort().map((key) =>
      `${JSON.stringify(key)}:${canonicalOracle(input[key])}`).join(',')}}`;
  }
  const encoded = JSON.stringify(value);
  if (encoded === undefined) throw new TypeError('KAT value is not JSON-serializable');
  return encoded;
}

function rehashAuthorityConfiguration(
  value: ReturnType<typeof validAuthorityConfiguration>,
  mutate: (configuration: any) => void,
) {
  const { configurationDigest: _ignored, ...body } = structuredClone(value);
  mutate(body);
  return parseProgrammeCaptureSupervisorAuthorityConfigurationV2({
    ...body,
    configurationDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_AUTHORITY_CONFIG_DIGEST_DOMAIN_V2,
      configuration: body,
    }),
  });
}

// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  createIssue8ProgrammeEnvelope,
  ISSUE_8_V4_POLICY_DIGEST,
  ISSUE_8_V4_REQUIRED_PROTECTED_PATHS,
  ISSUE_8_V4_POLICY_SOURCE_COMMIT,
  parseIssue8ProgrammeEnvelope,
} from '../src/issue-8-programme-envelope.js';
import { ReceiptChain, type Receipt } from '../src/receipts.js';

const golden = readFileSync(
  new URL('./fixtures/issue-8-programme-envelope-v4.json', import.meta.url),
  'utf8',
);

describe('schema-v4 programme envelope replay', () => {
  it('replays the historical envelope independently of live policy growth', () => {
    const envelope = parseIssue8ProgrammeEnvelope(golden);

    expect(envelope.envelopeDigest)
      .toBe('1aa77aca451eed9211bcd59390098289b7fd47778a876b9809518c44a1ed79f7');
    expect(envelope.programmeAcceptanceDigest)
      .toBe('4857f4cf0235af43db27fc1bff7afe491d37b1efe553c8d85c5eb9620b3d1731');
    expect(envelope.programmeAcceptance).toMatchObject({
      score: 40,
      status: 'REJECTED',
      receiptDigest: '29ea3b4a17875fb2820526c9e5fe9dca8dc9de1831704d3af7c205a260a975e2',
    });
  });

  it('pins the original v4 protected-path floor', () => {
    expect(ISSUE_8_V4_REQUIRED_PROTECTED_PATHS).toHaveLength(158);
    expect(new Set(ISSUE_8_V4_REQUIRED_PROTECTED_PATHS).size).toBe(158);
    expect(ISSUE_8_V4_POLICY_DIGEST)
      .toBe('df7afb055ab4d842b170e08a4efbeda4f7fd4edbf55d8c04ccd111e452275f35');
    expect(ISSUE_8_V4_POLICY_SOURCE_COMMIT)
      .toBe('47b9224a5fd4e8548c2a0c6c3c479e2e0abdc742');
  });

  it('rejects evidence missing an original v4 protected path', () => {
    const document = JSON.parse(golden) as Readonly<{
      receiptChain: { receipts: readonly Receipt[] };
      diagnosticBlob: string;
    }>;
    const receipt = document.receiptChain.receipts[0];
    const {
      sequence: _sequence,
      previousDigest: _previousDigest,
      digest: _digest,
      ...draft
    } = receipt;
    const protectedInputs = { ...draft.protectedInputs };
    delete protectedInputs['.mcp.json'];
    const replacement = new ReceiptChain().append({ ...draft, protectedInputs });
    const envelope = createIssue8ProgrammeEnvelope(replacement, document.diagnosticBlob);
    const policy = envelope.programmeAcceptance.dimensions.find(
      ({ id }) => id === 'policy-and-supply-chain-safety',
    );

    expect(policy).toMatchObject({ verifiedPoints: 0, hardGatePassed: false });
    expect(envelope.programmeAcceptance.score).toBe(20);
    expect(envelope.programmeAcceptance.status).toBe('REJECTED');
  });
});

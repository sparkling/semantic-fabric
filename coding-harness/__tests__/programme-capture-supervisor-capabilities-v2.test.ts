// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

const SOURCE_ROOT = fileURLToPath(new URL('../src/', import.meta.url));
const CONFIG_ENTRY = resolve(SOURCE_ROOT, 'programme-capture-supervisor-authority-config-v2.ts');
const TRANSITION_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-authority-transition-v2.ts',
);
const RUN_EVENT_CONTRACTS_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-run-event-contracts-v2.ts',
);
const RUN_EVENT_BODY_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-run-event-body-v2.ts',
);
const RUN_EVENT_CODEC_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-run-event-codec-v2.ts',
);
const RUN_EVENT_TRANSITION_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-run-event-transition-v2.ts',
);
const RUN_EVENT_VERIFIER_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-run-event-verifier-v2.ts',
);
const CLAIM_KEY_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-claim-key-v1.ts',
);
const REGISTRATION_REQUEST_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-registration-request-v2.ts',
);
const RUN_EVENT_BUILDER_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-run-event-builder-v2.ts',
);
const SERVICE_RESULT_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-service-result-v2.ts',
);
const SERVICE_CLIENT_ENTRY = resolve(
  SOURCE_ROOT, 'programme-capture-supervisor-service-client-v2.ts',
);
const V2_ENTRIES = Object.freeze([
  CONFIG_ENTRY, TRANSITION_ENTRY, RUN_EVENT_CONTRACTS_ENTRY, RUN_EVENT_BODY_ENTRY,
  RUN_EVENT_CODEC_ENTRY, RUN_EVENT_TRANSITION_ENTRY, RUN_EVENT_VERIFIER_ENTRY,
  CLAIM_KEY_ENTRY, REGISTRATION_REQUEST_ENTRY, RUN_EVENT_BUILDER_ENTRY, SERVICE_RESULT_ENTRY,
  SERVICE_CLIENT_ENTRY,
]);
const EXPECTED_IMPORTS = new Map<string, ReadonlyMap<string, string>>([
  [CONFIG_ENTRY, new Map([
    ['node:util/types', 'isProxy:isProxy'],
    ['./acceptance-task-v3.js', 'parseTaskOpaqueId:parseTaskOpaqueId'],
    [
      './contracts.js',
      'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,SHA256_PATTERN:SHA256_PATTERN,asClosedRecord:asClosedRecord,asDenseArray:asDenseArray,assertExactKeys:assertExactKeys,deepFreeze:deepFreeze,normalizePublicHttpsOrigin:normalizePublicHttpsOrigin',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
    ['./strict-json.js', 'parseJsonWithoutDuplicateKeys:parseJsonWithoutDuplicateKeys'],
  ])],
  [TRANSITION_ENTRY, new Map([
    ['node:util/types', 'isProxy:isProxy'],
    [
      './contracts.js',
      'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,SHA256_PATTERN:SHA256_PATTERN,asClosedRecord:asClosedRecord,assertExactKeys:assertExactKeys,deepFreeze:deepFreeze',
    ],
    [
      './programme-capture-supervisor-authority-config-v2.js',
      'parseProgrammeCaptureSupervisorAuthorityConfigurationV2:parseProgrammeCaptureSupervisorAuthorityConfigurationV2,programmeCaptureSupervisorAuthorityGenesisHeadDigestV2:programmeCaptureSupervisorAuthorityGenesisHeadDigestV2',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
    ['./strict-json.js', 'parseJsonWithoutDuplicateKeys:parseJsonWithoutDuplicateKeys'],
  ])],
  [RUN_EVENT_CONTRACTS_ENTRY, new Map([
    ['node:util/types', 'isProxy:isProxy'],
    ['./acceptance-task-v3.js', 'parseTaskOpaqueId:parseTaskOpaqueId'],
    [
      './contracts.js',
      'SHA256_PATTERN:SHA256_PATTERN,asClosedRecord:asClosedRecord,asDenseArray:asDenseArray',
    ],
  ])],
  [RUN_EVENT_BODY_ENTRY, new Map([
    ['./contracts.js', 'assertExactKeys:assertExactKeys,deepFreeze:deepFreeze'],
    [
      './programme-capture-supervisor-run-event-contracts-v2.js',
      'PROGRAMME_CAPTURE_SUPERVISOR_RESOURCE_CONFLICT_SET_DOMAIN_V2:PROGRAMME_CAPTURE_SUPERVISOR_RESOURCE_CONFLICT_SET_DOMAIN_V2,assertProgrammeCaptureSupervisorAttemptOutcomeDispositionsV2:assertProgrammeCaptureSupervisorAttemptOutcomeDispositionsV2,closedRunEventRecordV2:closedRunEventRecordV2,denseRunEventArrayV2:denseRunEventArrayV2,parseRunEventDigestV2:parseRunEventDigestV2,parseRunEventOpaqueIdV2:parseRunEventOpaqueIdV2,parseRunEventTimestampV2:parseRunEventTimestampV2,parseRunEventUint64V2:parseRunEventUint64V2',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
  ])],
  [RUN_EVENT_CODEC_ENTRY, new Map([
    ['@metaharness/harness', 'canonical:canonical'],
    [
      './contracts.js',
      'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,assertExactKeys:assertExactKeys,deepFreeze:deepFreeze',
    ],
    [
      './programme-capture-supervisor-run-event-body-v2.js',
      'parseProgrammeCaptureSupervisorAuthorityHeadRefV2:parseProgrammeCaptureSupervisorAuthorityHeadRefV2,parseProgrammeCaptureSupervisorPreviousGlobalV2:parseProgrammeCaptureSupervisorPreviousGlobalV2,parseProgrammeCaptureSupervisorPreviousRunV2:parseProgrammeCaptureSupervisorPreviousRunV2,parseProgrammeCaptureSupervisorResourceTransitionV2:parseProgrammeCaptureSupervisorResourceTransitionV2,parseProgrammeCaptureSupervisorRunEventBodyV2:parseProgrammeCaptureSupervisorRunEventBodyV2',
    ],
    [
      './programme-capture-supervisor-run-event-contracts-v2.js',
      'PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_KINDS_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_KINDS_V2,PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2,PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_SIGNING_DOMAIN_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_SIGNING_DOMAIN_V2,closedRunEventRecordV2:closedRunEventRecordV2,parseRunEventDigestV2:parseRunEventDigestV2,parseRunEventOpaqueIdV2:parseRunEventOpaqueIdV2,parseRunEventUint64V2:parseRunEventUint64V2',
    ],
    [
      './programme-capture-supervisor-crypto-v1.js',
      'parseProgrammeCaptureSupervisorEd25519SignatureV1:parseProgrammeCaptureSupervisorEd25519SignatureV1',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
    ['./strict-json.js', 'parseJsonWithoutDuplicateKeys:parseJsonWithoutDuplicateKeys'],
  ])],
  [RUN_EVENT_TRANSITION_ENTRY, new Map([
    ['./contracts.js', 'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,deepFreeze:deepFreeze'],
    [
      './programme-capture-supervisor-run-event-contracts-v2.js',
      'PROGRAMME_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_DOMAIN_V2:PROGRAMME_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_DOMAIN_V2,PROGRAMME_CAPTURE_SUPERVISOR_RUN_STATE_DIGEST_DOMAIN_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_STATE_DIGEST_DOMAIN_V2,denseRunEventArrayV2:denseRunEventArrayV2,parseRunEventDigestV2:parseRunEventDigestV2',
    ],
    [
      './programme-capture-supervisor-run-event-codec-v2.js',
      'parseProgrammeCaptureSupervisorRunEventV2:parseProgrammeCaptureSupervisorRunEventV2',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
  ])],
  [RUN_EVENT_VERIFIER_ENTRY, new Map([
    [
      './contracts.js',
      'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,assertExactKeys:assertExactKeys,deepFreeze:deepFreeze,snapshotUint8Array:snapshotUint8Array',
    ],
    [
      './programme-capture-supervisor-authority-config-v2.js',
      'parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2:parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2,parseProgrammeCaptureSupervisorAuthorityConfigurationV2:parseProgrammeCaptureSupervisorAuthorityConfigurationV2,programmeCaptureSupervisorAuthorityGenesisHeadDigestV2:programmeCaptureSupervisorAuthorityGenesisHeadDigestV2',
    ],
    [
      './programme-capture-supervisor-authority-transition-v2.js',
      'verifyProgrammeCaptureSupervisorAuthorityTransitionV2:verifyProgrammeCaptureSupervisorAuthorityTransitionV2',
    ],
    [
      './programme-capture-supervisor-run-event-body-v2.js',
      'parseProgrammeCaptureSupervisorPreviousGlobalV2:parseProgrammeCaptureSupervisorPreviousGlobalV2,parseProgrammeCaptureSupervisorPreviousRunV2:parseProgrammeCaptureSupervisorPreviousRunV2,parseProgrammeCaptureSupervisorPriorResourceStateV2:parseProgrammeCaptureSupervisorPriorResourceStateV2',
    ],
    [
      './programme-capture-supervisor-run-event-codec-v2.js',
      'parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2:parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2,programmeCaptureSupervisorRunEventSigningPayloadV2:programmeCaptureSupervisorRunEventSigningPayloadV2',
    ],
    [
      './programme-capture-supervisor-run-event-contracts-v2.js',
      'PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2,PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_VALIDATION_DIGEST_DOMAIN_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_VALIDATION_DIGEST_DOMAIN_V2,PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_DIGEST_DOMAIN_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_DIGEST_DOMAIN_V2,PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_VALIDATION_DIGEST_DOMAIN_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_VALIDATION_DIGEST_DOMAIN_V2,closedRunEventRecordV2:closedRunEventRecordV2,denseRunEventArrayV2:denseRunEventArrayV2,parseRunEventDigestV2:parseRunEventDigestV2,parseRunEventOpaqueIdV2:parseRunEventOpaqueIdV2,parseRunEventUint64V2:parseRunEventUint64V2',
    ],
    [
      './programme-capture-supervisor-crypto-v1.js',
      'programmeCaptureSupervisorUtf8Sha256V1:programmeCaptureSupervisorUtf8Sha256V1,verifyProgrammeCaptureSupervisorEd25519SignatureV1:verifyProgrammeCaptureSupervisorEd25519SignatureV1',
    ],
    [
      './programme-capture-supervisor-run-event-transition-v2.js',
      'deriveProgrammeCaptureSupervisorRunStateV2:deriveProgrammeCaptureSupervisorRunStateV2,programmeCaptureSupervisorControllerStateHeadDigestV2:programmeCaptureSupervisorControllerStateHeadDigestV2',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
  ])],
  [CLAIM_KEY_ENTRY, new Map([
    ['./acceptance-task-v3.js', 'parseTaskOpaqueId:parseTaskOpaqueId'],
    [
      './contracts.js',
      'SHA256_PATTERN:SHA256_PATTERN,asClosedRecord:asClosedRecord,assertExactKeys:assertExactKeys',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
  ])],
  [REGISTRATION_REQUEST_ENTRY, new Map([
    ['node:util/types', 'isProxy:isProxy'],
    [
      './contracts.js',
      'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,asClosedRecord:asClosedRecord,assertExactKeys:assertExactKeys,deepFreeze:deepFreeze',
    ],
    [
      './programme-capture-claim-key-v1.js',
      'programmeCaptureRunClaimKeyDigestV1:programmeCaptureRunClaimKeyDigestV1',
    ],
    [
      './programme-capture-supervisor-run-event-body-v2.js',
      'parseProgrammeCaptureSupervisorAuthorityHeadRefV2:parseProgrammeCaptureSupervisorAuthorityHeadRefV2',
    ],
    [
      './programme-capture-supervisor-run-event-contracts-v2.js',
      'parseRunEventDigestV2:parseRunEventDigestV2,parseRunEventOpaqueIdV2:parseRunEventOpaqueIdV2',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
    ['./strict-json.js', 'parseJsonWithoutDuplicateKeys:parseJsonWithoutDuplicateKeys'],
  ])],
  [RUN_EVENT_BUILDER_ENTRY, new Map([
    ['./contracts.js', 'assertExactKeys:assertExactKeys'],
    [
      './programme-capture-supervisor-run-event-contracts-v2.js',
      'PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,closedRunEventRecordV2:closedRunEventRecordV2',
    ],
    [
      './programme-capture-supervisor-run-event-codec-v2.js',
      'PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_NON_AUTHORITY_V2:PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_NON_AUTHORITY_V2,parseProgrammeCaptureSupervisorRunEventEnvelopeV2:parseProgrammeCaptureSupervisorRunEventEnvelopeV2,parseProgrammeCaptureSupervisorRunEventV2:parseProgrammeCaptureSupervisorRunEventV2',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
  ])],
  [SERVICE_RESULT_ENTRY, new Map([
    ['node:util/types', 'isProxy:isProxy'],
    [
      './contracts.js',
      'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,asClosedRecord:asClosedRecord,assertExactKeys:assertExactKeys,deepFreeze:deepFreeze',
    ],
    [
      './programme-capture-supervisor-run-event-codec-v2.js',
      'parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2:parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2',
    ],
    [
      './programme-capture-supervisor-run-event-contracts-v2.js',
      'parseRunEventDigestV2:parseRunEventDigestV2',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
    ['./strict-json.js', 'parseJsonWithoutDuplicateKeys:parseJsonWithoutDuplicateKeys'],
  ])],
  [SERVICE_CLIENT_ENTRY, new Map([
    [
      './contracts.js',
      'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,assertExactKeys:assertExactKeys,deepFreeze:deepFreeze',
    ],
    [
      './programme-capture-supervisor-registration-request-v2.js',
      'parseProgrammeCaptureSupervisorRegistrationRequestBlobV2:parseProgrammeCaptureSupervisorRegistrationRequestBlobV2,programmeCaptureSupervisorRegistrationChangedReplayEvidenceDigestV2:programmeCaptureSupervisorRegistrationChangedReplayEvidenceDigestV2',
    ],
    [
      './programme-capture-supervisor-run-event-codec-v2.js',
      'parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2:parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2',
    ],
    [
      './programme-capture-supervisor-run-event-contracts-v2.js',
      'closedRunEventRecordV2:closedRunEventRecordV2',
    ],
    [
      './programme-capture-supervisor-run-event-verifier-v2.js',
      'verifyProgrammeCaptureSupervisorRunEventEnvelopeV2:verifyProgrammeCaptureSupervisorRunEventEnvelopeV2',
    ],
    [
      './programme-capture-supervisor-service-result-v2.js',
      'parseProgrammeCaptureSupervisorServiceResultBlobV2:parseProgrammeCaptureSupervisorServiceResultBlobV2',
    ],
    ['./receipts.js', 'digestValue:digestValue'],
  ])],
]);
const FORBIDDEN_AMBIENT = new Set([
  'Bun', 'Deno', 'Function', 'Math', 'WebSocket', 'Worker', 'createRequire',
  'crypto', 'eval', 'fetch', 'generateKeyPair', 'global', 'globalThis', 'module',
  'performance', 'privateKey', 'process', 'require', 'setInterval', 'setTimeout', 'sign',
]);

describe('programme capture supervisor capability closure V2', () => {
  it('allows only closed parsing, normalization, freezing, and digest imports', () => {
    for (const path of V2_ENTRIES) {
      expect(() => verifyCapabilityClosure(path, readFileSync(path, 'utf8'))).not.toThrow();
    }
  });

  it('rejects transport, write, signing, process, and dynamic-loader mutants', () => {
    const mutants = [
      "import fs from 'node:fs'; void fs;",
      "import * as net from 'node:net'; void net;",
      "import { execFileSync } from 'node:child_process'; void execFileSync;",
      "import { sign } from 'node:crypto'; void sign;",
      "import { verify } from 'node:crypto'; void verify;",
      "await import('node:http');", "require('node:https');",
      "createRequire(import.meta.url)('node:fs');", "process.getBuiltinModule('fs');",
      "globalThis.fetch('https://example.invalid');", "Function('return fetch')();",
      "(async()=>{})['con' + 'structor']('return fetch')();",
      'Date.now();', 'Math.random();', 'crypto.getRandomValues(new Uint8Array(1));',
      "export { readFileSync } from 'node:fs';",
    ];
    for (const path of V2_ENTRIES) {
      const source = readFileSync(path, 'utf8');
      for (const mutant of mutants) {
        expect(() => verifyCapabilityClosure(path, `${source}\n${mutant}\n`)).toThrow();
      }
    }
  });
});

function verifyCapabilityClosure(path: string, source: string): void {
  const fail = (detail: string): never => {
    throw new Error(`HARNESS_CAPTURE_SUPERVISOR_CAPABILITY_CLOSURE_V2_INVALID:${detail}`);
  };
  const tree = ts.createSourceFile(
    path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS,
  );
  if (tree.parseDiagnostics.length) fail('syntax');
  const expected = EXPECTED_IMPORTS.get(path)!;
  const seen = new Set<string>();
  const visit = (node: ts.Node): void => {
    if (ts.isImportEqualsDeclaration(node)) fail('import equals');
    if (ts.isExportDeclaration(node) && node.moduleSpecifier && !node.isTypeOnly) fail('export');
    if (ts.isImportDeclaration(node)) {
      if (!ts.isStringLiteralLike(node.moduleSpecifier)) fail('computed import');
      const specifier = node.moduleSpecifier.text;
      const clause = node.importClause;
      if (clause?.isTypeOnly) return;
      if (!clause || clause.name || !clause.namedBindings
        || !ts.isNamedImports(clause.namedBindings) || seen.has(specifier)) {
        fail(`import ${specifier}`);
      }
      const runtime = clause.namedBindings.elements.filter((item) => !item.isTypeOnly);
      if (runtime.length === 0) return;
      const bindings = runtime.map((item) =>
        `${(item.propertyName ?? item.name).text}:${item.name.text}`).sort().join(',');
      if (bindings !== expected.get(specifier)) fail(`binding ${specifier}`);
      seen.add(specifier);
    }
    if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword
      || ts.isCallExpression(node.expression))) fail('dynamic call');
    if (ts.isPropertyAccessExpression(node) && node.name.text === 'constructor') {
      fail('constructor');
    }
    if (ts.isElementAccessExpression(node) && node.argumentExpression
      && staticString(node.argumentExpression) === 'constructor') fail('constructor');
    if (ts.isIdentifier(node) && FORBIDDEN_AMBIENT.has(node.text)) fail(`ambient ${node.text}`);
    if (ts.isPropertyAccessExpression(node) && ts.isIdentifier(node.expression)
      && node.expression.text === 'Date' && node.name.text === 'now') fail('clock');
    ts.forEachChild(node, visit);
  };
  visit(tree);
  if (seen.size !== expected.size) fail('missing import');
}

function staticString(node: ts.Expression): string | undefined {
  if (ts.isStringLiteralLike(node)) return node.text;
  if (ts.isParenthesizedExpression(node)) return staticString(node.expression);
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    const left = staticString(node.left);
    const right = staticString(node.right);
    return left === undefined || right === undefined ? undefined : left + right;
  }
  return undefined;
}

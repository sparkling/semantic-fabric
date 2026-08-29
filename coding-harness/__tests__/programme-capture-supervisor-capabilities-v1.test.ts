// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

const SOURCE_ROOT = fileURLToPath(new URL('../src/', import.meta.url));
const CLAIM_ENTRY = resolve(SOURCE_ROOT, 'programme-capture-supervisor-claim-v1.ts');
const CODEC_ENTRY = resolve(SOURCE_ROOT, 'programme-capture-supervisor-codec-v1.ts');
const CRYPTO_ENTRY = resolve(SOURCE_ROOT, 'programme-capture-supervisor-crypto-v1.ts');
const MERKLE_ENTRY = resolve(SOURCE_ROOT, 'programme-capture-supervisor-merkle-v1.ts');
const EXPECTED_IMPORTS = new Map<string, ReadonlyMap<string, string>>([
  [CLAIM_ENTRY, new Map([
    ['@metaharness/harness', 'canonical:canonical'],
    ['node:crypto', 'createHash:createHash'],
    ['./contracts.js', 'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,SHA256_PATTERN:SHA256_PATTERN,asClosedRecord:asClosedRecord,asInteger:asInteger,assertExactKeys:assertExactKeys,deepFreeze:deepFreeze'],
    ['./acceptance-task-v3.js', 'parseTaskOpaqueId:parseTaskOpaqueId'],
    ['./programme-capture-claim-io-v1.js', 'readProgrammeCaptureRunClaimV1:readProgrammeCaptureRunClaimV1'],
    ['./programme-capture-claim-record-v1.js', 'parseProgrammeCaptureRunClaimV1:parseProgrammeCaptureRunClaimV1,programmeCaptureRunClaimKeyDigestV1:programmeCaptureRunClaimKeyDigestV1,serializeProgrammeCaptureRunClaimV1:serializeProgrammeCaptureRunClaimV1'],
    ['./programme-capture-supervisor-crypto-v1.js', 'parseProgrammeCaptureSupervisorEd25519SignatureV1:parseProgrammeCaptureSupervisorEd25519SignatureV1,verifyProgrammeCaptureSupervisorEd25519SignatureV1:verifyProgrammeCaptureSupervisorEd25519SignatureV1'],
    ['./receipts.js', 'digestValue:digestValue'],
    ['./strict-json.js', 'parseJsonWithoutDuplicateKeys:parseJsonWithoutDuplicateKeys'],
  ])],
  [CODEC_ENTRY, new Map([
    ['./contracts.js', 'DEVELOPMENT_AUTHORITY:DEVELOPMENT_AUTHORITY,SHA256_PATTERN:SHA256_PATTERN,asClosedRecord:asClosedRecord,assertExactKeys:assertExactKeys,deepFreeze:deepFreeze'],
    ['./programme-capture-supervisor-claim-v1.js', 'PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_DIGEST_DOMAIN_V1:PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_DIGEST_DOMAIN_V1,parseProgrammeCaptureSupervisorClaimAcknowledgementV1:parseProgrammeCaptureSupervisorClaimAcknowledgementV1,parseProgrammeCaptureSupervisorClaimRequestV1:parseProgrammeCaptureSupervisorClaimRequestV1,verifyProgrammeCaptureSupervisorClaimAcknowledgementV1:verifyProgrammeCaptureSupervisorClaimAcknowledgementV1'],
    ['./receipts.js', 'digestValue:digestValue'],
    ['./strict-json.js', 'parseJsonWithoutDuplicateKeys:parseJsonWithoutDuplicateKeys'],
  ])],
  [CRYPTO_ENTRY, new Map([
    ['node:crypto', 'createHash:createHash,createPublicKey:createPublicKey,verify:verifyDetachedSignature'],
    ['node:util/types', 'isProxy:isProxy'],
    ['./contracts.js', 'SHA256_PATTERN:SHA256_PATTERN,asClosedRecord:asClosedRecord,assertExactKeys:assertExactKeys,snapshotUint8Array:snapshotUint8Array'],
  ])],
  [MERKLE_ENTRY, new Map([
    ['node:crypto', 'createHash:createHash'],
    ['node:util/types', 'isProxy:isProxy'],
    ['./contracts.js', 'SHA256_PATTERN:SHA256_PATTERN,asClosedRecord:asClosedRecord,asDenseArray:asDenseArray,assertExactKeys:assertExactKeys,snapshotUint8Array:snapshotUint8Array'],
  ])],
]);
const FORBIDDEN_AMBIENT = new Set([
  'Bun', 'Deno', 'Function', 'WebSocket', 'Worker', 'createRequire', 'eval', 'fetch',
  'generateKeyPair', 'global', 'globalThis', 'module', 'privateKey', 'process', 'require', 'sign',
]);

describe('programme capture supervisor capability closure V1', () => {
  it('allows only exact parser, digest, read, and detached-verification imports', () => {
    for (const path of [CLAIM_ENTRY, CODEC_ENTRY, CRYPTO_ENTRY, MERKLE_ENTRY]) {
      expect(() => verifyCapabilityClosure(path, readFileSync(path, 'utf8'))).not.toThrow();
    }
    const source = readFileSync(CLAIM_ENTRY, 'utf8');
    expect(() => assertVerifierOrder(source)).not.toThrow();
    expect(() => assertVerifierOrder(reorderVerifierRead(source))).toThrow(/ORDER_INVALID/);
  });

  it('rejects transport, write, signing, process, and dynamic-loader mutants', () => {
    const mutants = [
      "import fs from 'node:fs'; void fs;",
      "import * as net from 'node:net'; void net;",
      "import { execFileSync } from 'node:child_process'; void execFileSync;",
      "import { sign } from 'node:crypto'; void sign;", "await import('node:http');",
      "require('node:https');", "createRequire(import.meta.url)('node:fs');",
      "process.getBuiltinModule('fs');", "globalThis.fetch('https://example.invalid');",
      "Function('return fetch')();", "(async()=>{})['con' + 'structor']('return fetch')();",
      "export { readFileSync } from 'node:fs';",
    ];
    for (const path of [CLAIM_ENTRY, CODEC_ENTRY, CRYPTO_ENTRY, MERKLE_ENTRY]) {
      const source = readFileSync(path, 'utf8');
      for (const mutant of mutants) {
        expect(() => verifyCapabilityClosure(path, `${source}\n${mutant}\n`)).toThrow();
      }
    }
  });
});

function verifyCapabilityClosure(path: string, source: string): void {
  const fail = (detail: string): never => {
    throw new Error(`HARNESS_CAPTURE_SUPERVISOR_CAPABILITY_CLOSURE_INVALID:${detail}`);
  };
  const tree = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  if (tree.parseDiagnostics.length) fail('syntax');
  const expected = EXPECTED_IMPORTS.get(path)!;
  const seen = new Set<string>();
  const visit = (node: ts.Node): void => {
    if (ts.isImportEqualsDeclaration(node)) fail('import equals');
    if (ts.isExportDeclaration(node) && node.moduleSpecifier && !node.isTypeOnly) fail('export');
    if (ts.isImportDeclaration(node)) {
      if (!ts.isStringLiteralLike(node.moduleSpecifier)) fail('computed import');
      const specifier = node.moduleSpecifier.text, clause = node.importClause;
      if (clause?.isTypeOnly) return;
      if (!clause || clause.name || !clause.namedBindings
        || !ts.isNamedImports(clause.namedBindings) || seen.has(specifier)) fail(`import ${specifier}`);
      const runtime = clause.namedBindings.elements.filter((item) => !item.isTypeOnly);
      if (runtime.length === 0) return;
      const bindings = runtime.map((item) =>
        `${(item.propertyName ?? item.name).text}:${item.name.text}`).sort().join(',');
      if (bindings !== expected.get(specifier)) fail(`binding ${specifier}`);
      seen.add(specifier);
    }
    if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword
      || ts.isCallExpression(node.expression))) fail('dynamic call');
    if (ts.isPropertyAccessExpression(node) && node.name.text === 'constructor') fail('constructor');
    if (ts.isElementAccessExpression(node) && node.argumentExpression
      && staticString(node.argumentExpression) === 'constructor') fail('constructor');
    if (ts.isIdentifier(node) && FORBIDDEN_AMBIENT.has(node.text)) fail(`ambient ${node.text}`);
    ts.forEachChild(node, visit);
  };
  visit(tree);
  if (seen.size !== expected.size) fail('missing import');
}

function assertVerifierOrder(source: string): void {
  const tree = ts.createSourceFile(CLAIM_ENTRY, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const declaration = tree.statements.find((node): node is ts.FunctionDeclaration =>
    ts.isFunctionDeclaration(node)
      && node.name?.text === 'verifyProgrammeCaptureSupervisorClaimAcknowledgementV1');
  if (!declaration?.body) throw new Error('HARNESS_CAPTURE_SUPERVISOR_VERIFY_MISSING');
  const calls = new Map<string, number[]>();
  const visit = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
      const positions = calls.get(node.expression.text) ?? [];
      positions.push(node.getStart(tree)); calls.set(node.expression.text, positions);
    }
    ts.forEachChild(node, visit);
  };
  visit(declaration.body);
  const one = (name: string) => calls.get(name)?.[0] ?? Number.MAX_SAFE_INTEGER;
  const reads = calls.get('readRootedClaim') ?? [];
  const required = [
    'parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1',
    'verifyProgrammeCaptureSupervisorEd25519SignatureV1',
    'assertAcknowledgementBindings', 'assertSameClaim',
  ];
  if (reads.length !== 2 || required.some((name) => calls.get(name)?.length !== 1)
    || !(one('parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1')
    < one('verifyProgrammeCaptureSupervisorEd25519SignatureV1')
    && one('verifyProgrammeCaptureSupervisorEd25519SignatureV1') < reads[0]
    && reads[0] < one('assertAcknowledgementBindings')
    && one('assertAcknowledgementBindings') < reads[1]
    && reads[1] < one('assertSameClaim'))) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_VERIFY_ORDER_INVALID');
  }
}

function reorderVerifierRead(source: string): string {
  const start = source.indexOf('export async function verifyProgrammeCaptureSupervisor');
  const read = '  const before = await readRootedClaim(authority);\n';
  const marker = '  const authority = snapshotClaimAuthority(input.claimAuthority);\n';
  const readIndex = source.indexOf(read, start), insert = source.indexOf(marker, start) + marker.length;
  if (start < 0 || readIndex < 0 || insert < marker.length) throw new Error('mutation markers missing');
  const withoutRead = source.slice(0, readIndex) + source.slice(readIndex + read.length);
  return withoutRead.slice(0, insert) + read + withoutRead.slice(insert);
}

function staticString(node: ts.Expression): string | undefined {
  if (ts.isStringLiteralLike(node)) return node.text;
  if (ts.isParenthesizedExpression(node)) return staticString(node.expression);
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    const left = staticString(node.left), right = staticString(node.right);
    return left === undefined || right === undefined ? undefined : left + right;
  }
  return undefined;
}

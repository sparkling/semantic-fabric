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
]);
const FORBIDDEN_AMBIENT = new Set([
  'Bun', 'Date', 'Deno', 'Function', 'Math', 'WebSocket', 'Worker', 'createRequire',
  'crypto', 'eval', 'fetch', 'generateKeyPair', 'global', 'globalThis', 'module',
  'performance', 'privateKey', 'process', 'require', 'setInterval', 'setTimeout', 'sign',
]);

describe('programme capture supervisor capability closure V2', () => {
  it('allows only closed parsing, normalization, freezing, and digest imports', () => {
    for (const path of [CONFIG_ENTRY, TRANSITION_ENTRY]) {
      expect(() => verifyCapabilityClosure(path, readFileSync(path, 'utf8'))).not.toThrow();
    }
  });

  it('rejects transport, write, signing, process, and dynamic-loader mutants', () => {
    const mutants = [
      "import fs from 'node:fs'; void fs;",
      "import * as net from 'node:net'; void net;",
      "import { execFileSync } from 'node:child_process'; void execFileSync;",
      "import { sign } from 'node:crypto'; void sign;",
      "await import('node:http');", "require('node:https');",
      "createRequire(import.meta.url)('node:fs');", "process.getBuiltinModule('fs');",
      "globalThis.fetch('https://example.invalid');", "Function('return fetch')();",
      "(async()=>{})['con' + 'structor']('return fetch')();",
      'Date.now();', 'Math.random();', 'crypto.getRandomValues(new Uint8Array(1));',
      "export { readFileSync } from 'node:fs';",
    ];
    for (const path of [CONFIG_ENTRY, TRANSITION_ENTRY]) {
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

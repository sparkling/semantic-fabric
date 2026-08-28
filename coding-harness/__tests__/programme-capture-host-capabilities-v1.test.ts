// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';
import {
  collectProgrammeCaptureHostObservationV1,
  isFixedProgrammeCaptureHostObservationV1,
} from '../src/programme-capture-host-observation-v1.js';
import { snapshotUint8Array } from '../src/contracts.js';

const SOURCE_ROOT = fileURLToPath(new URL('../src/', import.meta.url));
const ENTRY = fileURLToPath(
  new URL('../src/programme-capture-host-preflight-v1.ts', import.meta.url),
);
const ALLOWED_EXTERNAL_IMPORTS = new Map<string, ReadonlySet<string>>([
  ['@metaharness/harness', new Set(['canonical', 'hash'])],
  ['node:crypto', new Set(['createHash'])],
  ['node:fs', new Set(['closeSync', 'constants', 'fstatSync', 'openSync', 'readSync'])],
  ['node:net', new Set(['isIP'])],
  ['node:path', new Set(['isAbsolute', 'posix', 'resolve'])],
]);
const FORBIDDEN_AMBIENT_REFERENCES = new Set([
  'Bun', 'Deno', 'EventSource', 'Function', 'SharedWorker', 'WebSocket', 'Worker',
  'XMLHttpRequest', 'eval', 'fetch', 'global', 'globalThis', 'navigator', 'require',
  'self', 'window',
]);

function fail(detail: string): never {
  throw new Error(`HARNESS_CAPTURE_HOST_CAPABILITY_CLOSURE_INVALID: ${detail}`);
}

function runtimeSpecifier(node: ts.Node): string | undefined {
  if (ts.isImportDeclaration(node)) {
    const clause = node.importClause;
    if (clause?.isTypeOnly) return undefined;
    if (clause?.namedBindings && ts.isNamedImports(clause.namedBindings)
      && !clause.name && clause.namedBindings.elements.every((item) => item.isTypeOnly)) {
      return undefined;
    }
    return ts.isStringLiteralLike(node.moduleSpecifier) ? node.moduleSpecifier.text : fail('import');
  }
  if (ts.isExportDeclaration(node) && node.moduleSpecifier) {
    if (node.isTypeOnly
      || (node.exportClause && ts.isNamedExports(node.exportClause)
        && node.exportClause.elements.every((item) => item.isTypeOnly))) return undefined;
    return ts.isStringLiteralLike(node.moduleSpecifier) ? node.moduleSpecifier.text : fail('export');
  }
  return undefined;
}

function checkExternalImport(node: ts.Node, specifier: string): void {
  const allowed = ALLOWED_EXTERNAL_IMPORTS.get(specifier);
  if (!allowed || !ts.isImportDeclaration(node)) fail(`external module ${specifier}`);
  const clause = node.importClause;
  if (!clause || clause.name || !clause.namedBindings
    || !ts.isNamedImports(clause.namedBindings)) fail(`import form ${specifier}`);
  const names = clause.namedBindings.elements
    .filter((item) => !item.isTypeOnly)
    .map((item) => (item.propertyName ?? item.name).text);
  if (names.length === 0 || clause.namedBindings.elements.some((item) => item.propertyName)
    || names.some((name) => !allowed.has(name))) {
    fail(`binding ${specifier}:${names.join(',')}`);
  }
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

function relativeTarget(containingPath: string, specifier: string): string {
  const target = resolve(dirname(containingPath), specifier.replace(/\.js$/, '.ts'));
  const fromRoot = relative(SOURCE_ROOT, target);
  if (fromRoot === '' || fromRoot.startsWith('..') || isAbsolute(fromRoot)) {
    fail(`relative escape ${specifier}`);
  }
  return target;
}

function assertAmbientCapabilityFree(node: ts.Node): void {
  if (ts.isImportEqualsDeclaration(node)) fail('import equals');
  if (ts.isCallExpression(node)) {
    if (node.expression.kind === ts.SyntaxKind.ImportKeyword) fail('dynamic import');
    if (ts.isCallExpression(node.expression)) fail('returned dynamic callable');
    if (ts.isIdentifier(node.expression) && node.expression.text === 'openSync') {
      const flags = node.arguments[1]?.getText();
      if (node.arguments.length !== 2 || node.arguments[0]?.getText() !== 'path'
        || flags !== 'constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0)') {
        fail('openSync call shape');
      }
    }
  }
  if (ts.isPropertyAccessExpression(node) && node.name.text === 'constructor') {
    fail('dynamic constructor');
  }
  if (ts.isElementAccessExpression(node) && node.argumentExpression
    && staticString(node.argumentExpression) === 'constructor') fail('dynamic constructor');
  if (ts.isPropertyAccessExpression(node) && ts.isIdentifier(node.expression)
    && node.expression.text === 'constants'
    && node.name.text !== 'O_RDONLY' && node.name.text !== 'O_NOFOLLOW') fail('write flag');
  if (ts.isIdentifier(node) && FORBIDDEN_AMBIENT_REFERENCES.has(node.text)) {
    fail(`ambient reference ${node.text}`);
  }
  if (ts.isIdentifier(node) && node.text === 'process') {
    const parent = node.parent;
    if (!ts.isPropertyAccessExpression(parent) || parent.expression !== node
      || (parent.name.text !== 'arch' && parent.name.text !== 'platform')) {
      fail('process authority');
    }
  }
}

function verifyRuntimeClosure(overrides: ReadonlyMap<string, string> = new Map()): Set<string> {
  const pending = [ENTRY], seen = new Set<string>();
  while (pending.length > 0) {
    const path = pending.pop()!;
    if (seen.has(path)) continue;
    seen.add(path);
    const source = overrides.get(path) ?? readFileSync(path, 'utf8');
    const tree = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    const visit = (node: ts.Node): void => {
      assertAmbientCapabilityFree(node);
      const specifier = runtimeSpecifier(node);
      if (specifier?.startsWith('.')) pending.push(relativeTarget(path, specifier));
      else if (specifier) checkExternalImport(node, specifier);
      ts.forEachChild(node, visit);
    };
    visit(tree);
  }
  return seen;
}

describe('programme capture host preflight capability closure V1', () => {
  it('allows only the fixed read/hash/parser runtime closure', () => {
    const closure = verifyRuntimeClosure();
    expect(closure.has(ENTRY)).toBe(true);
    expect(closure.size).toBeGreaterThan(8);
  });

  it('rejects transport, process, dynamic-loader, and ambient capability mutants', () => {
    const source = readFileSync(ENTRY, 'utf8');
    const mutants = [
      "const net = await import('node:net'); net.connect({ port: 1 });",
      "import net from 'node:net'; net.connect({ port: 1 });",
      "import { connect } from 'node:http2'; void connect;",
      "import * as dgram from 'node:dgram'; void dgram;",
      "import { Worker } from 'node:worker_threads'; void Worker;",
      "globalThis.fetch('https://example.invalid');",
      "fetch('https://example.invalid');",
      "require('node:child_process');",
      "createHash.constructor('return fetch')()('https://example.invalid');",
      "(async()=>{})['con' + 'structor']('return fetch')()('https://example.invalid');",
      "import cp = require('node:child_process'); void cp;",
      "openSync(path, constants.O_WRONLY | constants.O_CREAT | constants.O_TRUNC);",
    ];
    for (const mutant of mutants) {
      expect(() => verifyRuntimeClosure(new Map([[ENTRY, `${source}\n${mutant}\n`]])))
        .toThrow(/CAPABILITY_CLOSURE_INVALID/);
    }
  });

  it('keeps fixed-collector provenance under WeakSet prototype replacement', () => {
    const originalAdd = WeakSet.prototype.add, originalHas = WeakSet.prototype.has;
    try {
      WeakSet.prototype.add = function () { return this; };
      WeakSet.prototype.has = function () { return true; };
      const fixed = collectProgrammeCaptureHostObservationV1();
      expect(isFixedProgrammeCaptureHostObservationV1(fixed)).toBe(true);
      expect(isFixedProgrammeCaptureHostObservationV1(structuredClone(fixed))).toBe(false);
    } finally {
      WeakSet.prototype.add = originalAdd;
      WeakSet.prototype.has = originalHas;
    }
  });

  it('snapshots hostile, detached, and shared byte views without hook execution or aliasing', () => {
    let species = false, property = false, iterated = false;
    class HostileBytes extends Uint8Array {
      static get [Symbol.species](): Uint8ArrayConstructor { species = true; return Uint8Array; }
      get buffer(): ArrayBufferLike { property = true; return super.buffer; }
      *[Symbol.iterator](): Uint8ArrayIterator<number> {
        iterated = true; yield* super[Symbol.iterator]();
      }
    }
    const hostile = new HostileBytes(3);
    Uint8Array.prototype.set.call(hostile, [1, 2, 3]);
    const copied = snapshotUint8Array(hostile, 'hostile bytes', 3);
    expect([...copied]).toEqual([1, 2, 3]);
    expect({ species, property, iterated }).toEqual({
      species: false, property: false, iterated: false,
    });
    hostile[0] = 9;
    expect(copied[0]).toBe(1);

    const detached = new Uint8Array([4]);
    structuredClone(detached.buffer, { transfer: [detached.buffer] });
    expect(() => snapshotUint8Array(detached, 'detached bytes', 1)).toThrow(/byte bound/);

    const shared = new Uint8Array(new SharedArrayBuffer(2));
    shared.set([5, 6]);
    const sharedCopy = snapshotUint8Array(shared, 'shared bytes', 2);
    shared[0] = 7;
    expect([...sharedCopy]).toEqual([5, 6]);
  });
});

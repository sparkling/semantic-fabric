// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { readFileSync, realpathSync, statSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { TextDecoder } from 'node:util';

export const GATE_NAMES = Object.freeze([
  'rust', 'coding_harness', 'supervisor', 'acl_replay',
]);
const MAX_DIFF_BYTES = 1_048_576;
const MAX_MANIFEST_BYTES = 2_097_152;
const MAX_PATHS = 20_000;
const MAX_PATH_BYTES = 4_096;
const SHA = /^[0-9a-f]{40}$/;
const decoder = new TextDecoder('utf-8', { fatal: true });

const FULL_AUTHORITY_PATHS = new Set([
  '.gitignore',
  'coding-harness/.harness/controller-build.json',
  'coding-harness/.harness/manifest.json',
  'coding-harness/__tests__/controller-build.test.ts',
  'coding-harness/__tests__/m0-authority.test.ts',
  'coding-harness/__tests__/programme-v5-post-historical-paths.ts',
  'coding-harness/__tests__/select-ci-gates.test.ts',
  'coding-harness/scripts/select-ci-gates.mjs',
  'coding-harness/src/config.ts',
  'coding-harness/src/controller-build.ts',
]);
const RUST_AND_HARNESS_PATHS = new Set([
  'Cargo.lock', 'Cargo.toml', 'README.md', 'rust-toolchain.toml',
  'docs/capability-matrix.json', 'docs/capability-matrix.md',
  'tests/capabilities/catalog-v1.json', 'tests/capabilities/schema-v1.json',
]);
const HARNESS_ROOT_PATHS = new Set([
  '.mcp.json', 'AGENTS.md', 'BENCHMARKS.md', 'CLAUDE.md', 'COMPARISON.md',
  'harness-plan.json', 'repo-profile.json',
]);
const RUST_HARNESS_PREFIXES = [
  '.cargo/',
  'crates/sf-bench/src/performance/',
  'crates/sf-conformance/src/binary_artifact_receipt/',
  'crates/sf-conformance/src/execution_receipt/',
  'crates/sf-conformance/src/regression_receipt/',
  'crates/sf-conformance/src/rust_closure_receipt/',
  'tests/capabilities/', 'tests/sparql/', 'tests/w3c/rdb2rdf/',
];

export function allGates() {
  return Object.freeze(Object.fromEntries(GATE_NAMES.map((gate) => [gate, true])));
}

function noGates() {
  return Object.fromEntries(GATE_NAMES.map((gate) => [gate, false]));
}

export function selectForPaths(paths, protectedPaths = []) {
  try {
    const changed = checkedPathList(paths, false);
    const protectedSet = new Set(checkedPathList(protectedPaths, true));
    if (changed.length === 0) return allGates();
    const selected = noGates();
    for (const path of changed) {
      const classification = classify(path, protectedSet);
      if (classification === null) return allGates();
      for (const gate of classification) selected[gate] = true;
    }
    if (selected.acl_replay) selected.supervisor = true;
    if (selected.supervisor) selected.coding_harness = true;
    return Object.freeze(selected);
  } catch {
    return allGates();
  }
}

function classify(path, protectedPaths) {
  if (path.startsWith('.github/') || FULL_AUTHORITY_PATHS.has(path)) return GATE_NAMES;
  if (path.startsWith('coding-harness/supervisor-service/')) {
    return ['coding_harness', 'supervisor', 'acl_replay'];
  }
  if (path.startsWith('coding-harness/')) return ['coding_harness'];
  if (RUST_AND_HARNESS_PATHS.has(path)
    || (path.startsWith('crates/') && path.endsWith('/Cargo.toml'))
    || RUST_HARNESS_PREFIXES.some((prefix) => path.startsWith(prefix))) {
    return ['rust', 'coding_harness'];
  }
  if (path.startsWith('crates/') || path.startsWith('tests/')
    || path.startsWith('benches/') || path.startsWith('examples/')) {
    return protectedPaths.has(path) ? ['rust', 'coding_harness'] : ['rust'];
  }
  if (path.startsWith('docs/')) return ['coding_harness'];
  if (path.startsWith('.agents/') || path.startsWith('.claude/')
    || path.startsWith('.claude-flow/') || path.startsWith('.harness/')) {
    return ['coding_harness'];
  }
  if (HARNESS_ROOT_PATHS.has(path) || path.startsWith('LICENSE')) return ['coding_harness'];
  if (path.startsWith('scratch/') && path.endsWith('Cargo.toml')) {
    return ['rust', 'coding_harness'];
  }
  return protectedPaths.has(path) ? ['coding_harness'] : null;
}

function checkedPathList(value, allowEmpty) {
  if (!Array.isArray(value) || value.length > MAX_PATHS || (!allowEmpty && value.length === 0)) {
    throw new TypeError('CI_SELECTOR_PATH_LIST_INVALID');
  }
  const seen = new Set();
  for (const path of value) {
    if (typeof path !== 'string' || Buffer.byteLength(path, 'utf8') > MAX_PATH_BYTES
      || path === '' || path.startsWith('/') || path.includes('\\')
      || /[\u0000-\u001f\u007f]/u.test(path) || path.normalize('NFC') !== path) {
      throw new TypeError('CI_SELECTOR_PATH_INVALID');
    }
    const parts = path.split('/');
    if (parts.some((part) => part === '' || part === '.' || part === '..') || seen.has(path)) {
      throw new TypeError('CI_SELECTOR_PATH_INVALID');
    }
    seen.add(path);
  }
  return [...seen];
}

export function parseDiffOutput(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.length > MAX_DIFF_BYTES) {
    throw new TypeError('CI_SELECTOR_DIFF_INVALID');
  }
  const text = decoder.decode(bytes);
  if (!text.endsWith('\0')) throw new TypeError('CI_SELECTOR_DIFF_INVALID');
  return checkedPathList(text.slice(0, -1).split('\0'), false);
}

export function readChangedPaths({ repository, baseSha, headSha }) {
  if (typeof repository !== 'string' || !isAbsolute(repository)
    || realpathSync(repository) !== resolve(repository) || !statSync(repository).isDirectory()
    || !SHA.test(baseSha) || !SHA.test(headSha) || baseSha === headSha) {
    throw new TypeError('CI_SELECTOR_GIT_INPUT_INVALID');
  }
  for (const commit of [baseSha, headSha]) {
    const exists = git(repository, ['cat-file', '-e', `${commit}^{commit}`], 1_024);
    if (exists.status !== 0) throw new Error('CI_SELECTOR_COMMIT_INVALID');
  }
  const ancestor = git(repository, ['merge-base', '--is-ancestor', baseSha, headSha], 1_024);
  if (ancestor.status !== 0) throw new Error('CI_SELECTOR_ANCESTRY_INVALID');
  const diff = git(repository, [
    'diff', '--no-renames', '--name-only', '-z', baseSha, headSha, '--',
  ], MAX_DIFF_BYTES + 1);
  if (diff.status !== 0 || diff.error || !Buffer.isBuffer(diff.stdout)) {
    throw new Error('CI_SELECTOR_DIFF_FAILED');
  }
  return parseDiffOutput(diff.stdout);
}

function git(repository, args, maxBuffer) {
  return spawnSync('git', ['-C', repository, ...args], {
    encoding: null, maxBuffer, timeout: 30_000, windowsHide: true,
  });
}

export function readProtectedPaths(manifestPath) {
  const bytes = readFileSync(manifestPath);
  if (bytes.length === 0 || bytes.length > MAX_MANIFEST_BYTES) {
    throw new TypeError('CI_SELECTOR_MANIFEST_INVALID');
  }
  const parsed = JSON.parse(decoder.decode(bytes));
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new TypeError('CI_SELECTOR_MANIFEST_INVALID');
  }
  return checkedPathList(parsed.protectedPaths, false);
}

export function selectFromGit({ eventName, repository, baseSha, headSha, manifestPath }) {
  if (eventName !== 'pull_request') return allGates();
  try {
    const protectedPaths = readProtectedPaths(manifestPath);
    return selectForPaths(readChangedPaths({ repository, baseSha, headSha }), protectedPaths);
  } catch {
    return allGates();
  }
}

export function formatOutputs(gates) {
  if (gates === null || typeof gates !== 'object'
    || GATE_NAMES.some((gate) => typeof gates[gate] !== 'boolean')) {
    throw new TypeError('CI_SELECTOR_OUTPUT_INVALID');
  }
  return `${GATE_NAMES.map((gate) => `${gate}=${String(gates[gate])}`).join('\n')}\n`;
}

function parseArguments(argv) {
  const allowed = new Set(['--event', '--repository', '--base', '--head', '--manifest']);
  if (argv.length % 2 !== 0) throw new TypeError('CI_SELECTOR_ARGUMENT_INVALID');
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!allowed.has(key) || values.has(key) || typeof value !== 'string' || value === '') {
      throw new TypeError('CI_SELECTOR_ARGUMENT_INVALID');
    }
    values.set(key, value);
  }
  return {
    eventName: values.get('--event'), repository: values.get('--repository'),
    baseSha: values.get('--base'), headSha: values.get('--head'),
    manifestPath: values.get('--manifest'),
  };
}

function main() {
  let gates = allGates();
  try { gates = selectFromGit(parseArguments(process.argv.slice(2))); }
  catch { /* A malformed invocation runs every gate. */ }
  process.stdout.write(formatOutputs(gates));
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) main();

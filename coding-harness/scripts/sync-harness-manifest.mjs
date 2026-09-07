// SPDX-License-Identifier: MIT

import { chmodSync, readFileSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SECURE_HARNESS_CONFIG } from '../dist/config.js';
import { parseHarnessManifest } from '../dist/manifest.js';
import { parseJsonWithoutDuplicateKeys } from '../dist/strict-json.js';

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifestPath = resolve(harnessRoot, '.harness/manifest.json');
const args = process.argv.slice(2);
if (args.length > 1 || (args.length === 1 && args[0] !== '--check')) {
  throw new Error('usage: sync-harness-manifest.mjs [--check]');
}

const current = readFileSync(manifestPath, 'utf8');
const input = parseJsonWithoutDuplicateKeys(current, 'canonical harness manifest');
if (input === null || typeof input !== 'object' || Array.isArray(input)) {
  throw new Error('HARNESS_MANIFEST_OBJECT_REQUIRED');
}
const synchronized = {
  ...input,
  protectedPaths: [...SECURE_HARNESS_CONFIG.requiredProtectedPaths],
};
parseHarnessManifest(synchronized, SECURE_HARNESS_CONFIG);
const serialized = compactProtectedPaths(synchronized);
const lineCount = serialized.endsWith('\n')
  ? serialized.split('\n').length - 1 : serialized.split('\n').length;
if (lineCount >= 500) throw new Error(`HARNESS_MANIFEST_LINE_LIMIT_EXCEEDED:${lineCount}`);

if (args[0] === '--check') {
  if (serialized !== current) throw new Error('HARNESS_MANIFEST_NOT_SYNCHRONIZED');
} else if (serialized !== current) {
  const temporary = `${manifestPath}.tmp-${String(process.pid)}`;
  try {
    writeFileSync(temporary, serialized, { encoding: 'utf8', flag: 'wx', mode: 0o600 });
    renameSync(temporary, manifestPath);
  } catch (error) {
    try { unlinkSync(temporary); } catch {}
    throw error;
  }
}

// A no-content-change rebuild must also repair the generated file's mode.
if (args[0] !== '--check') chmodSync(manifestPath, 0o644);

function compactProtectedPaths(value) {
  const pretty = JSON.stringify(value, null, 2);
  const startMarker = '  "protectedPaths": [\n';
  const endMarker = '\n  ],\n  "acceptanceTasks":';
  const start = pretty.indexOf(startMarker);
  const end = pretty.indexOf(endMarker, start + startMarker.length);
  if (start < 0 || end < 0) throw new Error('HARNESS_MANIFEST_FORMAT_UNEXPECTED');
  const paths = value.protectedPaths;
  if (!Array.isArray(paths) || paths.some((path) => typeof path !== 'string')) {
    throw new Error('HARNESS_MANIFEST_PROTECTED_PATHS_INVALID');
  }
  const packed = packStrings(paths, 220);
  return `${pretty.slice(0, start + startMarker.length)}${packed}${pretty.slice(end)}\n`;
}

function packStrings(values, maximumWidth) {
  const lines = [];
  let line = '    ';
  values.forEach((value, index) => {
    const token = `${JSON.stringify(value)}${index + 1 === values.length ? '' : ','}`;
    const candidate = line.trim().length === 0 ? `${line}${token}` : `${line} ${token}`;
    if (candidate.length > maximumWidth && line.trim().length > 0) {
      lines.push(line);
      line = `    ${token}`;
    } else {
      line = candidate;
    }
  });
  if (line.trim().length > 0) lines.push(line);
  return lines.join('\n');
}

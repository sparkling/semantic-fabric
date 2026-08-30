// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync,
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
  renameSync,
  writeFileSync,
} from 'node:fs';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';

const BUILD_MANIFEST = 'coding-harness/.harness/controller-build.json';
const HARNESS_MANIFEST = 'coding-harness/.harness/manifest.json';
const LOCKFILE = 'coding-harness/package-lock.json';
const RUNTIME_ENTRY = 'coding-harness/dist/issue-8-program.js';
const projectRoot = canonicalDirectory(resolve('.'), 'HARNESS_BUILD_PROJECT_INVALID');
const repositoryRoot = canonicalDirectory(resolve(projectRoot, '..'), 'HARNESS_BUILD_REPOSITORY_INVALID');

const harnessManifest = parseJson(readRepositoryFile(HARNESS_MANIFEST), HARNESS_MANIFEST);
if (!Array.isArray(harnessManifest.protectedPaths)) {
  throw new Error('HARNESS_BUILD_PROTECTED_PATHS_INVALID');
}
const outputPaths = harnessManifest.protectedPaths
  .filter((path) => typeof path === 'string'
    && path.startsWith('coding-harness/src/')
    && (path.endsWith('.ts') || path.endsWith('.cts')))
  .map((path) => path.replace('coding-harness/src/', 'coding-harness/dist/')
    .replace(/\.cts$/, '.cjs').replace(/\.ts$/, '.js'))
  .sort();
const actualOutputs = walkFiles(resolve(projectRoot, 'dist'))
  .map(repositoryPath)
  .filter((path) => path.endsWith('.js') || path.endsWith('.cjs'))
  .sort();
assertExactPaths(actualOutputs, outputPaths, 'HARNESS_BUILD_OUTPUT_SET_MISMATCH');
if (!outputPaths.includes(RUNTIME_ENTRY)) throw new Error('HARNESS_BUILD_RUNTIME_ENTRY_MISSING');

const lockfile = parseJson(readRepositoryFile(LOCKFILE), LOCKFILE);
if (lockfile.lockfileVersion !== 3 || lockfile.packages === null
  || typeof lockfile.packages !== 'object' || Array.isArray(lockfile.packages)) {
  throw new Error('HARNESS_BUILD_LOCKFILE_INVALID');
}
const packageRoots = Object.entries(lockfile.packages)
  .filter(([path, value]) => path.startsWith('node_modules/')
    && value !== null && typeof value === 'object'
    && value.dev !== true && value.optional !== true)
  .map(([path]) => `coding-harness/${path}`)
  .sort();
if (packageRoots.length === 0) throw new Error('HARNESS_BUILD_PRODUCTION_PACKAGES_MISSING');
const productionPaths = packageRoots.flatMap((path) =>
  walkFiles(repositoryFile(path, true)).map(repositoryPath)).sort();

const outputs = digestFiles(outputPaths);
const productionFiles = digestFiles(productionPaths);
const body = {
  schemaVersion: 1,
  authority: 'development-only-no-promotion',
  runtimeEntry: RUNTIME_ENTRY,
  harnessManifestDigest: sha256(readRepositoryFile(HARNESS_MANIFEST)),
  lockfileDigest: sha256(readRepositoryFile(LOCKFILE)),
  outputs,
  productionFiles,
};
const build = { ...body, runtimeTreeDigest: sha256(JSON.stringify(body)) };
const target = repositoryTarget(BUILD_MANIFEST);
const temporary = `${target}.tmp-${String(process.pid)}`;
writeFileSync(temporary, `${JSON.stringify(build, null, 2)}\n`, {
  encoding: 'utf8', flag: 'wx', mode: 0o600,
});
renameSync(temporary, target);
chmodSync(target, 0o644);

function digestFiles(paths) {
  return Object.fromEntries(paths.map((path) => [path, sha256(readRepositoryFile(path))]));
}

function walkFiles(root) {
  const files = [];
  const visit = (directory) => {
    const stat = lstatSync(directory);
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(directory) !== directory
      || (stat.mode & 0o022) !== 0) throw new Error('HARNESS_BUILD_RUNTIME_TREE_UNTRUSTED');
    for (const entry of readdirSync(directory, { withFileTypes: true })
      .sort((left, right) => left.name.localeCompare(right.name))) {
      const path = join(directory, entry.name);
      const child = lstatSync(path);
      if (entry.isDirectory() && child.isDirectory() && !child.isSymbolicLink()) visit(path);
      else if (entry.isFile() && child.isFile() && !child.isSymbolicLink()
        && child.nlink === 1 && realpathSync(path) === path && (child.mode & 0o022) === 0) files.push(path);
      else throw new Error('HARNESS_BUILD_RUNTIME_TREE_UNTRUSTED');
    }
  };
  visit(root);
  return files;
}

function readRepositoryFile(path) {
  return readFileSync(repositoryFile(path, false));
}

function repositoryFile(path, directory) {
  if (typeof path !== 'string' || path.includes('\0') || isAbsolute(path)) {
    throw new Error('HARNESS_BUILD_PATH_INVALID');
  }
  const absolute = resolve(repositoryRoot, path);
  const delta = relative(repositoryRoot, absolute);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error('HARNESS_BUILD_PATH_INVALID');
  }
  if (directory) return canonicalDirectory(absolute, 'HARNESS_BUILD_DIRECTORY_INVALID');
  const stat = lstatSync(absolute);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1
    || realpathSync(absolute) !== absolute || (stat.mode & 0o022) !== 0) {
    throw new Error('HARNESS_BUILD_FILE_INVALID');
  }
  return absolute;
}

function repositoryPath(absolute) {
  const path = relative(repositoryRoot, absolute).split(sep).join('/');
  if (path === '' || path.startsWith('../')) throw new Error('HARNESS_BUILD_PATH_INVALID');
  return path;
}

function repositoryTarget(path) {
  if (typeof path !== 'string' || path.includes('\0') || isAbsolute(path)) {
    throw new Error('HARNESS_BUILD_PATH_INVALID');
  }
  const absolute = resolve(repositoryRoot, path);
  const delta = relative(repositoryRoot, absolute);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error('HARNESS_BUILD_PATH_INVALID');
  }
  return absolute;
}

function canonicalDirectory(value, error) {
  const path = realpathSync(value);
  const stat = lstatSync(path);
  if (!isAbsolute(value) || resolve(value) !== value || path !== value
    || !stat.isDirectory() || stat.isSymbolicLink()) throw new Error(error);
  return path;
}

function assertExactPaths(actual, expected, error) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(error);
}

function parseJson(bytes, path) {
  try { return JSON.parse(bytes.toString('utf8')); }
  catch { throw new Error(`HARNESS_BUILD_JSON_INVALID:${path}`); }
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

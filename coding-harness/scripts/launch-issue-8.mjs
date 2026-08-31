// SPDX-License-Identifier: MIT
import { spawnSync } from 'node:child_process'; import { createHash } from 'node:crypto';
import { chmodSync, closeSync, constants, fstatSync, lstatSync, mkdirSync, mkdtempSync,
  openSync, readSync, readdirSync, realpathSync, writeFileSync, writeSync } from 'node:fs';
import { rm } from 'node:fs/promises';
import { registerHooks } from 'node:module';
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const NODE = realpathSync(process.execPath); const GIT = realpathSync('/usr/bin/git');
const BUILD_PATH = 'coding-harness/.harness/controller-build.json';
const MANIFEST_PATH = 'coding-harness/.harness/manifest.json';
const LOCK_PATH = 'coding-harness/package-lock.json'; const PACKAGE_PATH = 'coding-harness/package.json'; const RUNTIME_RESOURCES = Object.freeze(['coding-harness/config/programme-v5-ruflo-schema-v2-memory-bridge.js.gz', 'coding-harness/config/programme-v5-ruflo-schema-v2-memory-initializer.js.gz', 'coding-harness/config/programme-v5-ruflo-schema-v2-overlay.json']);
const RUNTIME_ENTRY = 'coding-harness/dist/issue-8-program.js';
const GIT_OBJECT = /^[a-f0-9]{40,64}$/; const DIGEST = /^[a-f0-9]{64}$/;
const DEFAULT_TASK_PATH = 'coding-harness/config/issue-8-acceptance.json'; const TASK_PATH = /^coding-harness\/config\/[a-z0-9]+(?:[-_][a-z0-9]+)*-acceptance\.json$/;
const MAX_FILE_BYTES = 100_000_000; let privateRuntime = null;
let controllerStore = null; let controllerStoreDigest = null; let controllerStoreCommit = null;
try {
  validateProcess();
  const invocation = parseInvocation(process.argv.slice(2));
  const nodeDigest = trustedExecutable(NODE);
  const gitDigest = trustedExecutable(GIT);
  controllerStore = invocation.controllerStore;
  controllerStoreCommit = invocation.controllerCommit;
  controllerStoreDigest = validateControllerStore(controllerStore, invocation.controllerCommit);
  const head = gitText(controllerStore, ['rev-parse', '--verify', 'HEAD']);
  const commit = gitText(controllerStore, ['rev-parse', '--verify', `${invocation.controllerCommit}^{commit}`]);
  if (head !== commit || commit !== invocation.controllerCommit) {
    throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_COMMIT_MISMATCH');
  }
  const buildBytes = gitBytes(controllerStore, commit, BUILD_PATH);
  const manifestBytes = gitBytes(controllerStore, commit, MANIFEST_PATH);
  const lockBytes = gitBytes(controllerStore, commit, LOCK_PATH);
  const packageBytes = gitBytes(controllerStore, commit, PACKAGE_PATH);
  const build = parseBuildManifest(buildBytes);
  if (build.harnessManifestDigest !== sha256(manifestBytes)
    || build.lockfileDigest !== sha256(lockBytes)) {
    throw new Error('HARNESS_BOOTSTRAP_BUILD_INPUT_MISMATCH');
  }
  const runtimeParent = privateDirectory(process.env.XDG_RUNTIME_DIR, 'RUNTIME_PARENT');
  privateRuntime = mkdtempSync(join(runtimeParent, 'semantic-fabric-controller-'));
  chmodSync(privateRuntime, 0o700);
  writeCommittedFile(privateRuntime, PACKAGE_PATH, packageBytes);
  for (const [path, expected] of Object.entries(build.outputs)) copyCurrentFile(invocation.repositoryRoot, privateRuntime, path, expected);
  for (const [path, expected] of Object.entries(build.productionFiles)) RUNTIME_RESOURCES.includes(path) ? writeCommittedFile(privateRuntime, path, committedBuildFile(controllerStore, commit, path, expected)) : copyCurrentFile(invocation.repositoryRoot, privateRuntime, path, expected);
  hardenRuntime(privateRuntime);
  installResolutionBoundary(privateRuntime);
  const entry = safePath(privateRuntime, build.runtimeEntry);
  const module = await import(pathToFileURL(entry).href);
  if (typeof module.trustedControllerMain !== 'function') throw new Error('HARNESS_BOOTSTRAP_ENTRY_INVALID');
  const outcome = await module.trustedControllerMain(process.argv.slice(2), Object.freeze({
    schemaVersion: 3,
    source: 'verified-packed-private-runtime',
    controllerCommit: commit,
    taskPath: invocation.taskPath,
    controllerStoreDigest,
    buildManifestDigest: sha256(buildBytes),
    runtimeTreeDigest: build.runtimeTreeDigest,
    nodeDigest,
    gitDigest,
  }));
  if (outcome === null || typeof outcome !== 'object'
    || !['pass', 'fail', 'gated', 'cancelled'].includes(outcome.status)
    || typeof outcome.seal !== 'function') {
    throw new Error('HARNESS_BOOTSTRAP_OUTCOME_INVALID');
  }
  await cleanupPrivateState();
  const sealed = await outcome.seal();
  const sealedDigests = [sealed.receiptDigest, sealed.programmeAcceptanceDigest, sealed.envelopeDigest]; const reasonValid = sealed.status === 'pass' ? outcome.reason === null : typeof outcome.reason === 'string' && safeReason(new Error(outcome.reason)) === outcome.reason;
  if (sealed.status !== outcome.status || !['pass', 'fail', 'gated', 'cancelled'].includes(sealed.status)
    || !reasonValid || !sealedDigests.every((digest) => DIGEST.test(digest))) {
    throw new Error('HARNESS_BOOTSTRAP_SEALED_OUTCOME_INVALID');
  }
  process.stdout.write(`${JSON.stringify({ status: sealed.status, reason: outcome.reason, receiptDigest: sealed.receiptDigest, programmeAcceptanceDigest: sealed.programmeAcceptanceDigest, envelopeDigest: sealed.envelopeDigest })}\n`);
  process.exitCode = sealed.status === 'pass' ? 0 : 1;
} catch (error) {
  try {
    await cleanupPrivateState();
  } catch (cleanupError) {
    error = new AggregateError([error, cleanupError], 'HARNESS_BOOTSTRAP_CLEANUP_FAILED');
  }
  process.stderr.write(`${JSON.stringify({ status: 'error', reason: safeReason(error) ?? 'HARNESS_BOOTSTRAP_FAILED' })}\n`); process.exitCode = 1;
}
async function cleanupRuntime() {
  if (privateRuntime === null) return;
  const target = privateRuntime;
  privateRuntime = null;
  privateDirectory(target, 'PRIVATE_RUNTIME_CHANGED');
  makeRuntimeRemovable(target);
  await rm(target, { recursive: true, force: true });
}
async function cleanupPrivateState() {
  const failures = [];
  try { await cleanupRuntime(); } catch (error) { failures.push(error); }
  if (controllerStore !== null) {
    const target = controllerStore;
    controllerStore = null;
    try {
      if (validateControllerStore(target, controllerStoreCommit) !== controllerStoreDigest) {
        throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_CHANGED');
      }
    } catch (error) { failures.push(error); }
    try {
      makeRuntimeRemovable(target);
      await rm(target, { recursive: true, force: true });
    } catch (error) { failures.push(error); }
  }
  if (failures.length > 0) throw new AggregateError(failures, 'HARNESS_BOOTSTRAP_PRIVATE_CLEANUP_FAILED');
}
function validateProcess() {
  if (process.execPath !== NODE || realpathSync(process.execPath) !== NODE) {
    throw new Error('HARNESS_BOOTSTRAP_NODE_PATH_INVALID');
  }
  const expectedArgs = ['--no-addons', '--disable-proto=throw', '--input-type=module'];
  if (JSON.stringify(process.execArgv) !== JSON.stringify(expectedArgs)) {
    throw new Error('HARNESS_BOOTSTRAP_NODE_FLAGS_INVALID');
  }
  const keys = Object.keys(process.env).sort();
  const expectedKeys = ['DBUS_SESSION_BUS_ADDRESS', 'LANG', 'XDG_RUNTIME_DIR'];
  if (JSON.stringify(keys) !== JSON.stringify(expectedKeys)
    || process.env.LANG !== 'C.UTF-8'
    || process.env.DBUS_SESSION_BUS_ADDRESS !== `unix:path=${process.env.XDG_RUNTIME_DIR}/bus`
    || process.umask() !== 0o077) {
    throw new Error('HARNESS_BOOTSTRAP_ENVIRONMENT_INVALID');
  }
}
function parseInvocation(argv) {
  const required = [
    'repository', 'controller-store', 'controller-commit', 'run-id', 'swarm-id',
    'coordination-task-id', 'hive-id', 'consensus-id',
  ];
  const allowed = [...required, 'task-path'];
  if (![required.length * 2, allowed.length * 2].includes(argv.length)) throw new Error('HARNESS_BOOTSTRAP_ARGUMENTS_INVALID');
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const name = flag?.startsWith('--') ? flag.slice(2) : '';
    const value = argv[index + 1];
    if (!allowed.includes(name) || values.has(name) || typeof value !== 'string'
      || value.length === 0 || value.includes('\0')) {
      throw new Error('HARNESS_BOOTSTRAP_ARGUMENTS_INVALID');
    }
    values.set(name, value);
  }
  if (required.some((name) => !values.has(name))) throw new Error('HARNESS_BOOTSTRAP_ARGUMENTS_INVALID');
  const repositoryRoot = canonicalDirectory(values.get('repository'), 'REPOSITORY_INVALID');
  const controllerStore = privateDirectory(values.get('controller-store'), 'CONTROLLER_STORE_INVALID');
  const runtimeParent = privateDirectory(process.env.XDG_RUNTIME_DIR, 'RUNTIME_PARENT');
  if (dirname(controllerStore) !== runtimeParent
    || !basename(controllerStore).startsWith('semantic-fabric-controller-store-')
    || pathsOverlap(repositoryRoot, controllerStore)) {
    throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_INVALID');
  }
  const controllerCommit = values.get('controller-commit');
  if (!GIT_OBJECT.test(controllerCommit)) throw new Error('HARNESS_BOOTSTRAP_COMMIT_INVALID');
  const taskPath = values.get('task-path') ?? DEFAULT_TASK_PATH;
  if (!TASK_PATH.test(taskPath)) throw new Error('HARNESS_BOOTSTRAP_ARGUMENTS_INVALID');
  return Object.freeze({ repositoryRoot, controllerStore, controllerCommit, taskPath });
}
function validateControllerStore(root, commit) {
  privateDirectory(root, 'CONTROLLER_STORE_INVALID');
  const files = [];
  const directories = [];
  const visit = (directory, prefix = '') => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      const relativePath = prefix === '' ? entry.name : `${prefix}/${entry.name}`;
      const stat = lstatSync(path, { bigint: true });
      if (stat.isSymbolicLink() || stat.uid !== BigInt(process.getuid()) || (stat.mode & 0o222n) !== 0n) {
        throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_UNTRUSTED');
      }
      if (entry.isDirectory()) { directories.push(relativePath); visit(path, relativePath); }
      else if (entry.isFile() && stat.nlink === 1n) files.push(relativePath);
      else throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_UNTRUSTED');
    }
  };
  visit(root);
  const expectedDirectories = ['objects', 'objects/info', 'objects/pack', 'refs', 'refs/heads', 'refs/tags'];
  const pack = files.filter((path) => /^objects\/pack\/pack-[a-f0-9]+\.pack$/.test(path));
  const index = files.filter((path) => /^objects\/pack\/pack-[a-f0-9]+\.idx$/.test(path));
  const fixed = files.filter((path) => !pack.includes(path) && !index.includes(path)).sort();
  if (JSON.stringify(directories.sort()) !== JSON.stringify(expectedDirectories)
    || pack.length !== 1 || index.length !== 1
    || pack[0].slice(0, -5) !== index[0].slice(0, -4)
    || JSON.stringify(fixed) !== JSON.stringify(['HEAD', 'config', 'refs/heads/controller'])) {
    throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_LAYOUT_INVALID');
  }
  const format = commit.length === 40
    ? '[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = true\n'
    : '[core]\n\trepositoryformatversion = 1\n\tfilemode = true\n\tbare = true\n[extensions]\n\tobjectformat = sha256\n';
  if (smallText(join(root, 'config')) !== format
    || smallText(join(root, 'HEAD')) !== 'ref: refs/heads/controller\n'
    || smallText(join(root, 'refs/heads/controller')) !== `${commit}\n`) {
    throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_METADATA_INVALID');
  }
  gitText(root, ['fsck', '--strict', '--full', '--no-reflogs', commit]);
  if (gitText(root, ['rev-parse', '--verify', 'HEAD']) !== commit) {
    throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_HEAD_INVALID');
  }
  return sha256(Buffer.from(JSON.stringify({ commit, files: Object.fromEntries(
    files.sort().map((path) => [path, fileDigest(join(root, path), 10_000_000_000)]),
  ) }), 'utf8'));
}
function smallText(path) {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.size < 1 || stat.size > 4096)
    throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_METADATA_INVALID');
  const buffer = Buffer.allocUnsafe(stat.size);
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    let offset = 0;
    while (offset < buffer.length) {
      const count = readSync(descriptor, buffer, offset, buffer.length - offset, offset);
      if (count === 0) throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_METADATA_CHANGED');
      offset += count;
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameFile(before, after)) throw new Error('HARNESS_BOOTSTRAP_CONTROLLER_STORE_METADATA_CHANGED');
    return buffer.toString('utf8');
  } finally { closeSync(descriptor); }
}
function parseBuildManifest(bytes) {
  let input;
  try { input = JSON.parse(bytes.toString('utf8')); }
  catch { throw new Error('HARNESS_BOOTSTRAP_BUILD_JSON_INVALID'); }
  exactKeys(input, [
    'schemaVersion', 'authority', 'runtimeEntry', 'harnessManifestDigest',
    'lockfileDigest', 'outputs', 'productionFiles', 'runtimeTreeDigest',
  ], 'BUILD');
  if (input.schemaVersion !== 1 || input.authority !== 'development-only-no-promotion'
    || input.runtimeEntry !== RUNTIME_ENTRY) {
    throw new Error('HARNESS_BOOTSTRAP_BUILD_INVALID');
  }
  const body = {
    schemaVersion: 1,
    authority: input.authority,
    runtimeEntry: runtimePath(input.runtimeEntry, 'output'),
    harnessManifestDigest: digest(input.harnessManifestDigest),
    lockfileDigest: digest(input.lockfileDigest),
    outputs: digestMap(input.outputs, 'output'),
    productionFiles: digestMap(input.productionFiles, 'production'),
  };
  const runtimeTreeDigest = digest(input.runtimeTreeDigest);
  if (!(RUNTIME_ENTRY in body.outputs) || RUNTIME_RESOURCES.some((path) => !(path in body.productionFiles))
    || sha256(Buffer.from(JSON.stringify(body), 'utf8')) !== runtimeTreeDigest) {
    throw new Error('HARNESS_BOOTSTRAP_BUILD_TREE_INVALID');
  }
  return Object.freeze({ ...body, runtimeTreeDigest });
}
function digestMap(value, kind) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('HARNESS_BOOTSTRAP_BUILD_MAP_INVALID');
  }
  const entries = Object.entries(value);
  if (entries.length === 0 || entries.length > 5_000) {
    throw new Error('HARNESS_BOOTSTRAP_BUILD_MAP_INVALID');
  }
  const output = {};
  let previous = '';
  for (const [path, valueDigest] of entries) {
    const normalized = runtimePath(path, kind);
    if (previous !== '' && normalized <= previous) {
      throw new Error('HARNESS_BOOTSTRAP_BUILD_PATH_ORDER_INVALID');
    }
    output[normalized] = digest(valueDigest);
    previous = normalized;
  }
  return Object.freeze(output);
}
function runtimePath(value, kind) {
  if (typeof value !== 'string' || value.length === 0 || value.length > 500
    || value.includes('\0') || value.includes('\\') || value.startsWith('/')
    || value.split('/').some((part) => part === '' || part === '.' || part === '..')) {
    throw new Error('HARNESS_BOOTSTRAP_RUNTIME_PATH_INVALID');
  }
  const valid = kind === 'output'
    ? value.startsWith('coding-harness/dist/') && value.endsWith('.js')
    : value.startsWith('coding-harness/node_modules/') || RUNTIME_RESOURCES.includes(value);
  if (!valid) throw new Error('HARNESS_BOOTSTRAP_RUNTIME_PATH_INVALID');
  return value;
}
function copyCurrentFile(repositoryRoot, runtimeRoot, path, expected) {
  const source = safePath(repositoryRoot, path);
  const pathStat = lstatSync(source, { bigint: true });
  if (!pathStat.isFile() || pathStat.isSymbolicLink() || pathStat.nlink !== 1n
    || realpathSync(source) !== source || pathStat.size < 1n
    || pathStat.size > BigInt(MAX_FILE_BYTES) || (pathStat.mode & 0o022n) !== 0n) {
    throw new Error('HARNESS_BOOTSTRAP_RUNTIME_SOURCE_INVALID');
  }
  const target = safeTarget(runtimeRoot, path);
  mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
  const input = openSync(source, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  const output = openSync(target, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL, 0o600);
  try {
    const before = fstatSync(input, { bigint: true });
    const hash = createHash('sha256');
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < before.size) {
      const count = readSync(input, buffer, 0, Math.min(buffer.length, Number(before.size - offset)), Number(offset));
      if (count === 0) break;
      writeAll(output, buffer, count, Number(offset));
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    const after = fstatSync(input, { bigint: true });
    if (offset !== before.size || !sameFile(before, after) || hash.digest('hex') !== expected) {
      throw new Error('HARNESS_BOOTSTRAP_RUNTIME_SOURCE_CHANGED');
    }
  } finally {
    closeSync(input);
    closeSync(output);
  }
  chmodSync(target, (Number(pathStat.mode) & 0o111) === 0 ? 0o400 : 0o500);
}
function committedBuildFile(root, commit, path, expected) { const bytes = gitBytes(root, commit, path); if (sha256(bytes) !== expected) throw new Error('HARNESS_BOOTSTRAP_BUILD_INPUT_MISMATCH'); return bytes; }
function writeCommittedFile(runtimeRoot, path, bytes) {
  if (bytes.length < 1 || bytes.length > 1_000_000) {
    throw new Error('HARNESS_BOOTSTRAP_COMMITTED_FILE_INVALID');
  }
  const target = safeTarget(runtimeRoot, path);
  mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
  writeFileSync(target, bytes, { flag: 'wx', mode: 0o400 });
}
function installResolutionBoundary(runtimeRoot) {
  registerHooks({
    resolve(specifier, context, nextResolve) {
      const resolved = nextResolve(specifier, context);
      if (resolved.url.startsWith('node:')) return resolved;
      if (!resolved.url.startsWith('file:')) {
        throw new Error('HARNESS_BOOTSTRAP_MODULE_ORIGIN_FORBIDDEN');
      }
      const path = fileURLToPath(resolved.url);
      const delta = relative(runtimeRoot, path);
      if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
        throw new Error('HARNESS_BOOTSTRAP_MODULE_ESCAPE');
      }
      return resolved;
    },
  });
}
function hardenRuntime(root) {
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) visit(path);
      else if (!entry.isFile()) throw new Error('HARNESS_BOOTSTRAP_PRIVATE_RUNTIME_INVALID');
    }
    chmodSync(directory, 0o500);
  };
  visit(root);
}
function makeRuntimeRemovable(root) {
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) visit(path);
      else if (!entry.isFile()) throw new Error('HARNESS_BOOTSTRAP_PRIVATE_RUNTIME_INVALID');
    }
    chmodSync(directory, 0o700);
  };
  visit(root);
}
function gitBytes(root, commit, path) {
  const result = spawnSync(GIT, [
    '-c', 'core.hooksPath=/dev/null', '-c', 'core.fsmonitor=false',
    '-c', 'core.pager=cat', 'show', `${commit}:${path}`,
  ], {
    cwd: root,
    env: gitEnvironment(),
    encoding: null,
    maxBuffer: 20_000_000,
    timeout: 30_000,
  });
  if (result.error || result.status !== 0 || !Buffer.isBuffer(result.stdout)
    || result.stdout.length < 1 || result.stdout.length > 10_000_000) {
    throw new Error('HARNESS_BOOTSTRAP_GIT_BLOB_FAILED');
  }
  return result.stdout;
}
function gitText(root, args) {
  const result = spawnSync(GIT, [
    '-c', 'core.hooksPath=/dev/null', '-c', 'core.fsmonitor=false',
    '-c', 'core.pager=cat', ...args,
  ], { cwd: root, env: gitEnvironment(), encoding: 'utf8', maxBuffer: 4096, timeout: 30_000 });
  if (result.error || result.status !== 0 || typeof result.stdout !== 'string') {
    throw new Error('HARNESS_BOOTSTRAP_GIT_COMMAND_FAILED');
  }
  return result.stdout.trim();
}
function gitEnvironment() {
  return {
    PATH: '/usr/bin:/bin', HOME: '/nonexistent', LANG: 'C', LC_ALL: 'C',
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_ATTR_NOSYSTEM: '1',
    GIT_NO_REPLACE_OBJECTS: '1',
    GIT_NO_LAZY_FETCH: '1',
    GIT_TERMINAL_PROMPT: '0', GIT_ASKPASS: '/bin/false', GIT_PAGER: 'cat', PAGER: 'cat',
  };
}
function trustedExecutable(path, expected) {
  const stat = lstatSync(path, { bigint: true });
  if (!stat.isFile() || stat.isSymbolicLink() || stat.uid !== 0n || stat.nlink !== 1n
    || (stat.mode & 0o111n) === 0n || (stat.mode & 0o022n) !== 0n || realpathSync(path) !== path) {
    throw new Error('HARNESS_BOOTSTRAP_EXECUTABLE_UNTRUSTED');
  }
  const actual = fileDigest(path, 200_000_000);
  if (expected !== undefined && actual !== expected) {
    throw new Error('HARNESS_BOOTSTRAP_EXECUTABLE_MISMATCH');
  }
  return actual;
}
function fileDigest(path, maximum) {
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    if (before.size < 1n || before.size > BigInt(maximum)) throw new Error('HARNESS_BOOTSTRAP_FILE_SIZE_INVALID');
    const hash = createHash('sha256');
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < before.size) {
      const count = readSync(descriptor, buffer, 0, Math.min(buffer.length, Number(before.size - offset)), Number(offset));
      if (count === 0) break;
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (offset !== before.size || !sameFile(before, after)) throw new Error('HARNESS_BOOTSTRAP_FILE_CHANGED');
    return hash.digest('hex');
  } finally { closeSync(descriptor); }
}
function privateDirectory(value, label) {
  const path = canonicalDirectory(value, label);
  const stat = lstatSync(path);
  if (stat.uid !== process.getuid() || (stat.mode & 0o077) !== 0) {
    throw new Error(`HARNESS_BOOTSTRAP_${label}`);
  }
  return path;
}
function canonicalDirectory(value, label) {
  if (typeof value !== 'string' || !isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error(`HARNESS_BOOTSTRAP_${label}`);
  }
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error(`HARNESS_BOOTSTRAP_${label}`);
  }
  return value;
}
function safePath(root, path) {
  const absolute = resolve(root, path);
  const delta = relative(root, absolute);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error('HARNESS_BOOTSTRAP_PATH_ESCAPE');
  }
  return absolute;
}
function safeTarget(root, path) {
  if (typeof path !== 'string' || path.includes('\0') || path.includes('\\')) {
    throw new Error('HARNESS_BOOTSTRAP_PATH_INVALID');
  }
  return safePath(root, path);
}
function pathsOverlap(left, right) {
  const delta = relative(left, right);
  const inverse = relative(right, left);
  return delta === '' || (!delta.startsWith(`..${sep}`) && delta !== '..' && !isAbsolute(delta))
    || (!inverse.startsWith(`..${sep}`) && inverse !== '..' && !isAbsolute(inverse));
}
function exactKeys(value, keys, label) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)
    || JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...keys].sort())) {
    throw new Error(`HARNESS_BOOTSTRAP_${label}_KEYS_INVALID`);
  }
}
function digest(value) {
  if (typeof value !== 'string' || !DIGEST.test(value) || value === '0'.repeat(64)) {
    throw new Error('HARNESS_BOOTSTRAP_DIGEST_INVALID');
  }
  return value;
}
function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
function sameFile(left, right) {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.size === right.size && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}
function writeAll(descriptor, buffer, length, position) {
  let written = 0;
  while (written < length) {
    written += writeSync(descriptor, buffer, written, length - written, position + written);
  }
}
function safeReason(error) {
    const allowed = new Set(['HARNESS_ACCEPTANCE_GATE_FAILED','HARNESS_BOOTSTRAP_ARGUMENTS_INVALID','HARNESS_BOOTSTRAP_BUILD_INPUT_MISMATCH','HARNESS_BOOTSTRAP_BUILD_INVALID','HARNESS_BOOTSTRAP_BUILD_JSON_INVALID','HARNESS_BOOTSTRAP_BUILD_MAP_INVALID','HARNESS_BOOTSTRAP_BUILD_PATH_ORDER_INVALID','HARNESS_BOOTSTRAP_BUILD_TREE_INVALID','HARNESS_BOOTSTRAP_CLEANUP_FAILED','HARNESS_BOOTSTRAP_COMMITTED_FILE_INVALID','HARNESS_BOOTSTRAP_COMMIT_INVALID','HARNESS_BOOTSTRAP_CONTROLLER_COMMIT_MISMATCH','HARNESS_BOOTSTRAP_CONTROLLER_STORE_CHANGED','HARNESS_BOOTSTRAP_CONTROLLER_STORE_HEAD_INVALID','HARNESS_BOOTSTRAP_CONTROLLER_STORE_INVALID','HARNESS_BOOTSTRAP_CONTROLLER_STORE_LAYOUT_INVALID','HARNESS_BOOTSTRAP_CONTROLLER_STORE_METADATA_CHANGED','HARNESS_BOOTSTRAP_CONTROLLER_STORE_METADATA_INVALID','HARNESS_BOOTSTRAP_CONTROLLER_STORE_UNTRUSTED','HARNESS_BOOTSTRAP_DIGEST_INVALID','HARNESS_BOOTSTRAP_ENTRY_INVALID','HARNESS_BOOTSTRAP_ENVIRONMENT_INVALID','HARNESS_BOOTSTRAP_EXECUTABLE_MISMATCH','HARNESS_BOOTSTRAP_EXECUTABLE_UNTRUSTED','HARNESS_BOOTSTRAP_FAILED','HARNESS_BOOTSTRAP_FILE_CHANGED','HARNESS_BOOTSTRAP_FILE_SIZE_INVALID','HARNESS_BOOTSTRAP_GIT_BLOB_FAILED','HARNESS_BOOTSTRAP_GIT_COMMAND_FAILED','HARNESS_BOOTSTRAP_MODULE_ESCAPE','HARNESS_BOOTSTRAP_MODULE_ORIGIN_FORBIDDEN','HARNESS_BOOTSTRAP_NODE_FLAGS_INVALID','HARNESS_BOOTSTRAP_NODE_PATH_INVALID','HARNESS_BOOTSTRAP_OUTCOME_INVALID','HARNESS_BOOTSTRAP_PATH_ESCAPE','HARNESS_BOOTSTRAP_PATH_INVALID','HARNESS_BOOTSTRAP_PRIVATE_CLEANUP_FAILED','HARNESS_BOOTSTRAP_PRIVATE_RUNTIME_INVALID','HARNESS_BOOTSTRAP_RUNTIME_PATH_INVALID','HARNESS_BOOTSTRAP_RUNTIME_SOURCE_CHANGED','HARNESS_BOOTSTRAP_RUNTIME_SOURCE_INVALID','HARNESS_BOOTSTRAP_SEALED_OUTCOME_INVALID','HARNESS_CLAUDE_SUBSCRIPTION_UNAVAILABLE','HARNESS_CLEANUP_FAILED','HARNESS_CODEX_SUBSCRIPTION_UNAVAILABLE','HARNESS_FROZEN_LOCK_DIGEST_MISMATCH','HARNESS_ISSUE_8_EXECUTION_AND_SCRATCH_CLEANUP_FAILED','HARNESS_ISSUE_8_PROGRAMME_ACCEPTANCE_REJECTED','HARNESS_ISSUE_8_TRANSACTION_FAILED','HARNESS_NATIVE_ARCHITECTURE_RESPONSE_INVALID','HARNESS_NATIVE_CIRCUIT_OPEN','HARNESS_NATIVE_HOST_FAILED','HARNESS_NATIVE_HOST_TIMEOUT','HARNESS_NATIVE_INVOCATION_CANCELLED','HARNESS_NATIVE_ORIGIN_POLICY_DENIED','HARNESS_NATIVE_ORIGIN_UNUSED','HARNESS_NATIVE_PATCH_INVALID','HARNESS_NATIVE_PATCH_RESPONSE_INVALID','HARNESS_NATIVE_RETRY_BUDGET_EXHAUSTED','HARNESS_NATIVE_REVIEW_RESPONSE_INVALID','HARNESS_NATIVE_STRUCTURED_ENVELOPE_INVALID','HARNESS_NATIVE_STRUCTURED_OUTPUT_INVALID','HARNESS_NATIVE_STRUCTURED_OUTPUT_MISSING','HARNESS_PATCH_ADMISSION_INVALID','HARNESS_PATCH_APPLICATION_FAILED','HARNESS_PATCH_EMPTY','HARNESS_PATCH_INVALID','HARNESS_PATCH_PATH_NOT_DECLARED','HARNESS_PATCH_TOO_LARGE','HARNESS_REPAIR_BUDGET_EXHAUSTED','HARNESS_RUNTIME_EVIDENCE_FAILED','HARNESS_TRANSACTION_FAILED','HARNESS_VERIFIER_INDEPENDENT_INFRASTRUCTURE_FAILED','HARNESS_VERIFIER_PUBLIC_INFRASTRUCTURE_FAILED','HARNESS_VERIFIER_REGRESSION_INFRASTRUCTURE_FAILED','HARNESS_RUST_CLOSURE_CONTENT_MISMATCH','HARNESS_RUST_REGISTRY_CONTENT_MISMATCH','HARNESS_RUST_REGISTRY_IO_FAILED']); const seen = new Set(); let count = 0;
  const visit = (value) => { if (!(value instanceof Error) || seen.has(value) || count++ >= 64) return null; seen.add(value); const match = Buffer.byteLength(value.message, 'utf8') <= 4096 ? /^(HARNESS_[A-Z0-9_]+)(?=[^A-Z0-9_]|$)/.exec(value.message) : null; const own = match !== null && allowed.has(match[1]) ? match[1] : null; if (!(value instanceof AggregateError) && own !== null) return own; for (const nested of value instanceof AggregateError ? [...value.errors, value.cause] : [value.cause]) { const found = visit(nested); if (found !== null) return found; } return own; };
  try { return visit(error); } catch { return null; }
}

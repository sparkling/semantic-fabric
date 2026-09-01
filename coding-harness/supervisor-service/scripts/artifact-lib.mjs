// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
  realpathSync,
} from 'node:fs';
import { isAbsolute, relative, resolve, sep } from 'node:path';

export const SERVICE_KIND = 'programme-capture-supervisor-service-v1';
export const SERVICE_AUTHORITY = 'nonoperational-proposed-adr-0042';
export const BUNDLE_PATH = 'dist/supervisor-service.mjs';
export const SOURCE_INPUT_PATHS = Object.freeze([
  'src/closed-json.ts',
  'src/index.ts',
  'src/readiness.ts',
  'src/registration-decision-v1.ts',
  'src/registration-event-envelope-v2.ts',
  'src/registration-protocol-v2.ts',
]);
export const BUILD_INPUT_PATHS = Object.freeze([
  'migrations/0001-registration-state-v1.sql',
  'migrations/0002-registration-rls-v1.sql',
  'migrations/catalog-contract-v1.json',
  'migrations/manifest-v1.json',
  'migrations/provisioning-contract-v1.json',
  'package.json',
  'tsconfig.json',
  'vitest.config.ts',
  'scripts/artifact-lib.mjs',
  'scripts/build.mjs',
  'scripts/deny-publish.mjs',
  'scripts/verify-artifact.mjs',
  'src/registration-ports-v1.ts',
  'src/registration-postgresql-authority-configuration-v1.ts',
  'src/registration-postgresql-authority-seed-v1.ts',
  'src/registration-postgresql-canonical-v1.ts',
  'src/registration-postgresql-catalogue-contract-v1.ts',
  'src/registration-postgresql-catalogue-core-v1.ts',
  'src/registration-postgresql-catalogue-query-v1.ts',
  'src/registration-postgresql-catalogue-scanner-v1.ts',
  'src/registration-postgresql-catalogue-security-v1.ts',
  'src/registration-postgresql-catalogue-shape-v1.ts',
  'src/registration-postgresql-catalogue-templates-v1.ts',
  'src/registration-postgresql-catalogue-values-v1.ts',
  'src/registration-postgresql-locked-snapshots-v1.ts',
  'src/registration-postgresql-materializer-contract-v1.ts',
  'src/registration-postgresql-materializer-rows-v1.ts',
  'src/registration-postgresql-materializer-v1.ts',
  'src/registration-postgresql-migration-command-catalogue-v1.ts',
  'src/registration-postgresql-migration-command-ddl-v1.ts',
  'src/registration-postgresql-migration-insert-contract-v1.ts',
  'src/registration-postgresql-migration-json-v1.ts',
  'src/registration-postgresql-migration-lifecycle-v1.ts',
  'src/registration-postgresql-migration-manifest-v1.ts',
  'src/registration-postgresql-migration-plan-v1.ts',
  'src/registration-postgresql-migration-reader-v1.ts',
  'src/registration-postgresql-migration-sql-policy-v1.ts',
  'src/registration-postgresql-migration-sql-scanner-v1.ts',
  'src/registration-postgresql-migration-store-contract-v1.ts',
  'src/registration-postgresql-migration-terminal-results-v1.ts',
  'src/registration-postgresql-provisioning-contract-v1.ts',
  'src/registration-postgresql-row-codecs-v1.ts',
  'src/registration-transaction-admission-v1.ts',
  'src/registration-transaction-boundary-v1.ts',
  'src/registration-transaction-checkout-v1.ts',
  'src/registration-transaction-contract-v1.ts',
  'src/registration-transaction-retry-v1.ts',
  'src/registration-transaction-v1.ts',
]);
const MAX_SEALED_FILE_BYTES = 16 * 1024 * 1024;

export function canonicalRoot(value) {
  const absolute = resolve(value);
  const stat = lstatSync(absolute);
  const canonical = realpathSync(absolute);
  if (!stat.isDirectory() || stat.isSymbolicLink() || canonical !== absolute) {
    throw new Error('SUPERVISOR_SERVICE_ROOT_UNTRUSTED');
  }
  return absolute;
}

export function readSafeFile(root, path, label) {
  const absolute = safePath(root, path);
  const pathBefore = lstatSync(absolute, { bigint: true });
  if (!trustedFile(pathBefore) || realpathSync(absolute) !== absolute) {
    throw new Error(`SUPERVISOR_SERVICE_${label}_UNTRUSTED`);
  }
  const descriptor = openSync(absolute, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    if (!sameFile(pathBefore, before) || before.size > BigInt(MAX_SEALED_FILE_BYTES)) {
      throw new Error(`SUPERVISOR_SERVICE_${label}_CHANGED`);
    }
    const bytes = Buffer.alloc(Number(before.size));
    let offset = 0;
    while (offset < bytes.length) {
      const count = readSync(descriptor, bytes, offset, bytes.length - offset, offset);
      if (count === 0) throw new Error(`SUPERVISOR_SERVICE_${label}_CHANGED`);
      offset += count;
    }
    const after = fstatSync(descriptor, { bigint: true });
    const pathAfter = lstatSync(absolute, { bigint: true });
    if (!sameFile(before, after) || !sameFile(after, pathAfter)
      || realpathSync(absolute) !== absolute) {
      throw new Error(`SUPERVISOR_SERVICE_${label}_CHANGED`);
    }
    return bytes;
  } finally {
    closeSync(descriptor);
  }
}

export function safePath(root, path) {
  if (typeof path !== 'string' || path.length === 0 || path.includes('\0')
    || isAbsolute(path) || path.includes('\\')) {
    throw new Error('SUPERVISOR_SERVICE_PATH_INVALID');
  }
  const absolute = resolve(root, path);
  const delta = relative(root, absolute);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error('SUPERVISOR_SERVICE_PATH_INVALID');
  }
  return absolute;
}

export function parseJson(bytes, label) {
  let parsed;
  try {
    const text = bytes.toString('utf8');
    parsed = JSON.parse(text);
    if (`${JSON.stringify(parsed, null, 2)}\n` !== text) {
      throw new Error('noncanonical JSON');
    }
  } catch {
    throw new Error(`SUPERVISOR_SERVICE_${label}_JSON_INVALID`);
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error(`SUPERVISOR_SERVICE_${label}_SHAPE_INVALID`);
  }
  return parsed;
}

export function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

export function digestMap(root, paths, label) {
  return Object.fromEntries(paths.map((path) => [
    path, sha256(readSafeFile(root, path, label)),
  ]));
}

export function artifactDigest(body) {
  return sha256(Buffer.from(canonical(body), 'utf8'));
}

export function assertExactKeys(value, expected, label) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)
    || JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...expected].sort())) {
    throw new Error(`SUPERVISOR_SERVICE_${label}_KEYS_INVALID`);
  }
}

export function assertDigest(value, label) {
  if (typeof value !== 'string' || !/^[a-f0-9]{64}$/.test(value)) {
    throw new Error(`SUPERVISOR_SERVICE_${label}_DIGEST_INVALID`);
  }
}

export function validateServiceManifest(manifest) {
  assertExactKeys(manifest, [
    'schemaVersion', 'serviceKind', 'authority', 'sourceEntrypoint', 'bundlePath',
    'operational', 'externallyReachable', 'registrationMutationsEnabled',
    'databaseAccessEnabled', 'signerAccessEnabled', 'publicationAccessEnabled',
    'parentRuntimeDependencyAllowed',
  ], 'MANIFEST');
  if (manifest.schemaVersion !== 1 || manifest.serviceKind !== SERVICE_KIND
    || manifest.authority !== SERVICE_AUTHORITY
    || manifest.sourceEntrypoint !== 'src/index.ts' || manifest.bundlePath !== BUNDLE_PATH
    || manifest.operational !== false || manifest.externallyReachable !== false
    || manifest.registrationMutationsEnabled !== false
    || manifest.databaseAccessEnabled !== false || manifest.signerAccessEnabled !== false
    || manifest.publicationAccessEnabled !== false
    || manifest.parentRuntimeDependencyAllowed !== false) {
    throw new Error('SUPERVISOR_SERVICE_MANIFEST_CONTRACT_INVALID');
  }
}

export function validatePackageContract(packageJson) {
  assertExactKeys(packageJson, [
    'name', 'version', 'description', 'private', 'license', 'type', 'scripts',
    'dependencies', 'devDependencies', 'engines',
  ], 'PACKAGE');
  assertExactKeys(packageJson.scripts, [
    'build', 'test', 'verify', 'prepublishOnly',
  ], 'PACKAGE_SCRIPTS');
  assertExactKeys(packageJson.dependencies, [], 'PACKAGE_DEPENDENCIES');
  assertExactKeys(packageJson.devDependencies, [
    '@types/node', 'esbuild', 'typescript', 'vite', 'vitest',
  ], 'PACKAGE_DEV_DEPENDENCIES');
  assertExactKeys(packageJson.engines, ['node'], 'PACKAGE_ENGINES');
  if (packageJson.name !== 'semantic-fabric-supervisor-service'
    || packageJson.version !== '0.1.0'
    || packageJson.description !== 'Private, independently deployable ADR-0042 supervisor service'
    || packageJson.private !== true || packageJson.license !== 'MIT'
    || packageJson.type !== 'module'
    || packageJson.scripts.build !== 'tsc --noEmit && node scripts/build.mjs'
    || packageJson.scripts.test !== 'vitest run'
    || packageJson.scripts.verify
      !== 'npm run build && npm test && node scripts/verify-artifact.mjs'
    || packageJson.scripts.prepublishOnly !== 'node scripts/deny-publish.mjs'
    || packageJson.devDependencies['@types/node'] !== '20.19.43'
    || packageJson.devDependencies.esbuild !== '0.28.2'
    || packageJson.devDependencies.typescript !== '5.9.3'
    || packageJson.devDependencies.vite !== '6.4.3'
    || packageJson.devDependencies.vitest !== '3.2.7'
    || packageJson.engines.node !== '>=20.0.0') {
    throw new Error('SUPERVISOR_SERVICE_PACKAGE_CONTRACT_INVALID');
  }
}

export function validateLockfile(lock, packageJson) {
  assertExactKeys(lock, ['name', 'version', 'lockfileVersion', 'requires', 'packages'], 'LOCKFILE');
  if (lock.name !== packageJson.name || lock.version !== packageJson.version
    || lock.lockfileVersion !== 3 || lock.requires !== true
    || lock.packages === null || typeof lock.packages !== 'object'
    || Array.isArray(lock.packages)) {
    throw new Error('SUPERVISOR_SERVICE_LOCKFILE_INVALID');
  }
  if (JSON.stringify(lock).includes('file:') || JSON.stringify(lock).includes('workspace:')) {
    throw new Error('SUPERVISOR_SERVICE_LOCKFILE_LOCAL_DEPENDENCY');
  }
  for (const [path, entry] of Object.entries(lock.packages)) {
    if (entry === null || typeof entry !== 'object' || Array.isArray(entry) || entry.link === true) {
      throw new Error(`SUPERVISOR_SERVICE_LOCKFILE_ENTRY_INVALID:${path}`);
    }
    if (entry.resolved !== undefined) {
      let resolved;
      try { resolved = new URL(entry.resolved); }
      catch { throw new Error(`SUPERVISOR_SERVICE_LOCKFILE_URL_INVALID:${path}`); }
      if (resolved.protocol !== 'https:' || resolved.hostname !== 'registry.npmjs.org'
        || resolved.port !== '' || resolved.username !== '' || resolved.password !== ''
        || typeof entry.integrity !== 'string'
        || !/^sha512-[A-Za-z0-9+/]+={0,2}$/.test(entry.integrity)) {
        throw new Error(`SUPERVISOR_SERVICE_LOCKFILE_PROVENANCE_INVALID:${path}`);
      }
    }
  }
}

function trustedFile(stat) {
  return stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1n
    && (stat.mode & 0o022n) === 0n;
}

function sameFile(left, right) {
  return trustedFile(left) && trustedFile(right)
    && left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.nlink === right.nlink && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function canonical(value) {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  return `{${Object.keys(value).sort().map((key) =>
    `${JSON.stringify(key)}:${canonical(value[key])}`).join(',')}}`;
}

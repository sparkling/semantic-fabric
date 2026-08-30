// SPDX-License-Identifier: MIT

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  artifactDigest,
  assertDigest,
  assertExactKeys,
  BUNDLE_PATH,
  BUILD_INPUT_PATHS,
  canonicalRoot,
  digestMap,
  parseJson,
  readSafeFile,
  SERVICE_AUTHORITY,
  SERVICE_KIND,
  sha256,
  SOURCE_INPUT_PATHS,
  validatePackageContract,
  validateServiceManifest,
  validateLockfile,
} from './artifact-lib.mjs';

const root = canonicalRoot(resolve(dirname(fileURLToPath(import.meta.url)), '..'));
const artifactBytes = readSafeFile(root, '.service/artifact.json', 'ARTIFACT');
const artifact = parseJson(artifactBytes, 'ARTIFACT');
const manifestBytes = readSafeFile(root, '.service/manifest.json', 'MANIFEST');
const packageBytes = readSafeFile(root, 'package.json', 'PACKAGE');
const lockBytes = readSafeFile(root, 'package-lock.json', 'LOCKFILE');
const manifest = parseJson(manifestBytes, 'MANIFEST');
const packageJson = parseJson(packageBytes, 'PACKAGE');
const lockfile = parseJson(lockBytes, 'LOCKFILE');
validateServiceManifest(manifest);
validatePackageContract(packageJson);
validateLockfile(lockfile, packageJson);

assertExactKeys(artifact, [
  'schemaVersion', 'serviceKind', 'authority', 'serviceManifestDigest',
  'packageLockDigest', 'sourceInputs', 'buildInputs', 'bundle',
  'externalImports', 'runtimePackages', 'artifactDigest',
], 'ARTIFACT');
assertExactKeys(artifact.bundle, ['path', 'bytes', 'sha256'], 'ARTIFACT_BUNDLE');
if (artifact.schemaVersion !== 1 || artifact.serviceKind !== SERVICE_KIND
  || artifact.authority !== SERVICE_AUTHORITY || artifact.bundle.path !== BUNDLE_PATH
  || !Number.isSafeInteger(artifact.bundle.bytes) || artifact.bundle.bytes <= 0
  || !Array.isArray(artifact.externalImports) || artifact.externalImports.length !== 0
  || !Array.isArray(artifact.runtimePackages) || artifact.runtimePackages.length !== 0) {
  throw new Error('SUPERVISOR_SERVICE_ARTIFACT_IDENTITY_INVALID');
}
for (const [value, label] of [
  [artifact.serviceManifestDigest, 'MANIFEST'],
  [artifact.packageLockDigest, 'LOCKFILE'],
  [artifact.bundle.sha256, 'BUNDLE'],
  [artifact.artifactDigest, 'ARTIFACT'],
]) assertDigest(value, label);

const expectedSources = digestMap(root, SOURCE_INPUT_PATHS, 'SOURCE');
const expectedBuildInputs = digestMap(root, BUILD_INPUT_PATHS, 'BUILD_INPUT');
const bundle = readSafeFile(root, BUNDLE_PATH, 'BUNDLE');
const body = {
  schemaVersion: 1,
  serviceKind: SERVICE_KIND,
  authority: SERVICE_AUTHORITY,
  serviceManifestDigest: sha256(manifestBytes),
  packageLockDigest: sha256(lockBytes),
  sourceInputs: expectedSources,
  buildInputs: expectedBuildInputs,
  bundle: { path: BUNDLE_PATH, bytes: bundle.byteLength, sha256: sha256(bundle) },
  externalImports: [],
  runtimePackages: Object.keys(packageJson.dependencies).sort(),
};
if (JSON.stringify(artifact.sourceInputs) !== JSON.stringify(expectedSources)
  || JSON.stringify(artifact.buildInputs) !== JSON.stringify(expectedBuildInputs)
  || JSON.stringify(artifact.bundle) !== JSON.stringify(body.bundle)
  || artifact.serviceManifestDigest !== body.serviceManifestDigest
  || artifact.packageLockDigest !== body.packageLockDigest
  || JSON.stringify(artifact.runtimePackages) !== JSON.stringify(body.runtimePackages)
  || artifact.artifactDigest !== artifactDigest(body)) {
  throw new Error('SUPERVISOR_SERVICE_ARTIFACT_BINDING_MISMATCH');
}
const bundleText = bundle.toString('utf8');
if (/sourceMappingURL|workspace:|file:/.test(bundleText)
  || /\b(?:import|export)\s+(?:[^'\"]+\s+from\s+)?['\"]/.test(bundleText)
  || /\bimport\s*\(/.test(bundleText)) {
  throw new Error('SUPERVISOR_SERVICE_BUNDLE_IMPORT_CLOSURE_INVALID');
}

process.stdout.write(`${JSON.stringify({
  verified: true,
  artifactDigest: artifact.artifactDigest,
  bundleDigest: artifact.bundle.sha256,
})}\n`);

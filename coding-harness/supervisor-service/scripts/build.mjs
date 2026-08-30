// SPDX-License-Identifier: MIT

import { build } from 'esbuild';
import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  artifactDigest,
  BUNDLE_PATH,
  BUILD_INPUT_PATHS,
  canonicalRoot,
  digestMap,
  parseJson,
  readSafeFile,
  safePath,
  SERVICE_AUTHORITY,
  SERVICE_KIND,
  sha256,
  SOURCE_INPUT_PATHS,
  validatePackageContract,
  validateServiceManifest,
  validateLockfile,
} from './artifact-lib.mjs';

const root = canonicalRoot(resolve(dirname(fileURLToPath(import.meta.url)), '..'));
const manifest = parseJson(readSafeFile(root, '.service/manifest.json', 'MANIFEST'), 'MANIFEST');
const packageJson = parseJson(readSafeFile(root, 'package.json', 'PACKAGE'), 'PACKAGE');
const lockfile = parseJson(readSafeFile(root, 'package-lock.json', 'LOCKFILE'), 'LOCKFILE');
validateServiceManifest(manifest);
validatePackageContract(packageJson);
validateLockfile(lockfile, packageJson);

const dist = safePath(root, 'dist');
if (existsSync(dist)) {
  const stat = lstatSync(dist);
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    throw new Error('SUPERVISOR_SERVICE_DIST_UNTRUSTED');
  }
  rmSync(dist, { recursive: true, force: true });
}
mkdirSync(dist, { mode: 0o755 });

const result = await build({
  absWorkingDir: root,
  entryPoints: ['src/index.ts'],
  outfile: BUNDLE_PATH,
  bundle: true,
  format: 'esm',
  platform: 'node',
  target: 'node20',
  treeShaking: true,
  legalComments: 'none',
  sourcemap: false,
  metafile: true,
  write: false,
  logLevel: 'silent',
});

const sourceInputs = Object.keys(result.metafile.inputs).sort();
if (JSON.stringify(sourceInputs) !== JSON.stringify(SOURCE_INPUT_PATHS)) {
  throw new Error('SUPERVISOR_SERVICE_SOURCE_CLOSURE_INVALID');
}
const output = result.metafile.outputs[BUNDLE_PATH];
if (output === undefined || Object.keys(result.metafile.outputs).length !== 1) {
  throw new Error('SUPERVISOR_SERVICE_OUTPUT_CLOSURE_INVALID');
}
const externalImports = output.imports.filter(({ external }) => external)
  .map(({ path }) => path).sort();
if (externalImports.length !== 0) {
  throw new Error('SUPERVISOR_SERVICE_EXTERNAL_IMPORT_INVALID');
}

if (result.outputFiles.length !== 1
  || resolve(result.outputFiles[0].path) !== safePath(root, BUNDLE_PATH)) {
  throw new Error('SUPERVISOR_SERVICE_OUTPUT_BYTES_INVALID');
}
const bundleTarget = safePath(root, BUNDLE_PATH);
const bundleTemporary = `${bundleTarget}.tmp-${String(process.pid)}`;
writeFileSync(bundleTemporary, result.outputFiles[0].contents, {
  flag: 'wx', mode: 0o600,
});
renameSync(bundleTemporary, bundleTarget);
chmodSync(bundleTarget, 0o444);
const bundle = readSafeFile(root, BUNDLE_PATH, 'BUNDLE');
const body = {
  schemaVersion: 1,
  serviceKind: SERVICE_KIND,
  authority: SERVICE_AUTHORITY,
  serviceManifestDigest: sha256(readSafeFile(root, '.service/manifest.json', 'MANIFEST')),
  packageLockDigest: sha256(readSafeFile(root, 'package-lock.json', 'LOCKFILE')),
  sourceInputs: digestMap(root, SOURCE_INPUT_PATHS, 'SOURCE'),
  buildInputs: digestMap(root, BUILD_INPUT_PATHS, 'BUILD_INPUT'),
  bundle: {
    path: BUNDLE_PATH,
    bytes: bundle.byteLength,
    sha256: sha256(bundle),
  },
  externalImports,
  runtimePackages: Object.keys(packageJson.dependencies).sort(),
};
const artifact = { ...body, artifactDigest: artifactDigest(body) };
const target = safePath(root, '.service/artifact.json');
const temporary = `${target}.tmp-${String(process.pid)}`;
writeFileSync(temporary, `${JSON.stringify(artifact, null, 2)}\n`, {
  encoding: 'utf8', flag: 'wx', mode: 0o600,
});
renameSync(temporary, target);
chmodSync(target, 0o444);

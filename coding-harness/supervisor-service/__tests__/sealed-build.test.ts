// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync, cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
import { describe, expect, it } from 'vitest';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const bundlePath = resolve(root, 'dist/supervisor-service.mjs');
const artifactPath = resolve(root, '.service/artifact.json');

function runScript(path: string, packageRoot = root) {
  return spawnSync(process.execPath, [resolve(packageRoot, path)], {
    cwd: packageRoot,
    encoding: 'utf8',
  });
}

function copySealedPackage(): string {
  const target = mkdtempSync(resolve(tmpdir(), 'sf-supervisor-seal-'));
  for (const path of [
    '.service', 'dist', 'migrations', 'scripts', 'src', 'package.json', 'package-lock.json',
    'tsconfig.json', 'vitest.config.ts',
  ]) {
    cpSync(resolve(root, path), resolve(target, path), { recursive: true });
  }
  return target;
}

describe('sealed supervisor-service artifact', () => {
  it('builds one dependency-free bundle bound to exact source and lock bytes', async () => {
    const build = runScript('scripts/build.mjs');
    expect(build.status, build.stderr).toBe(0);
    const verify = runScript('scripts/verify-artifact.mjs');
    expect(verify.status, verify.stderr).toBe(0);

    const artifact = JSON.parse(readFileSync(artifactPath, 'utf8')) as {
      schemaVersion: number;
      serviceKind: string;
      authority: string;
      bundle: { path: string; bytes: number; sha256: string };
      sourceInputs: Record<string, string>;
      buildInputs: Record<string, string>;
      externalImports: string[];
      runtimePackages: string[];
      artifactDigest: string;
    };
    const bundle = readFileSync(bundlePath);

    expect(artifact.schemaVersion).toBe(1);
    expect(artifact.serviceKind).toBe('programme-capture-supervisor-service-v1');
    expect(artifact.authority).toBe('nonoperational-proposed-adr-0042');
    expect(artifact.bundle).toEqual({
      path: 'dist/supervisor-service.mjs',
      bytes: 49_106,
      sha256: '90e21e7c0e3a45b66da55f0e8cf9c0a23b3fb82e805223922d81096e097f7c3a',
    });
    expect(bundle.byteLength).toBe(artifact.bundle.bytes);
    expect(createHash('sha256').update(bundle).digest('hex')).toBe(artifact.bundle.sha256);
    expect(artifact.sourceInputs).toEqual({
      'src/closed-json.ts': expect.stringMatching(/^[a-f0-9]{64}$/),
      'src/index.ts': expect.stringMatching(/^[a-f0-9]{64}$/),
      'src/readiness.ts': expect.stringMatching(/^[a-f0-9]{64}$/),
      'src/registration-decision-v1.ts': expect.stringMatching(/^[a-f0-9]{64}$/),
      'src/registration-event-envelope-v2.ts': expect.stringMatching(/^[a-f0-9]{64}$/),
      'src/registration-protocol-v2.ts': expect.stringMatching(/^[a-f0-9]{64}$/),
    });
    const privateBuildInputs = [
      'migrations/0001-registration-state-v1.sql',
      'migrations/0002-registration-rls-v1.sql',
      'migrations/catalog-contract-v1.json',
      'migrations/manifest-v1.json',
      'migrations/provisioning-contract-v1.json',
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
      'src/registration-postgresql-migration-json-v1.ts',
      'src/registration-postgresql-migration-manifest-v1.ts',
      'src/registration-postgresql-migration-plan-v1.ts',
      'src/registration-postgresql-migration-reader-v1.ts',
      'src/registration-postgresql-migration-sql-policy-v1.ts',
      'src/registration-postgresql-migration-sql-scanner-v1.ts',
      'src/registration-postgresql-provisioning-contract-v1.ts',
      'src/registration-postgresql-row-codecs-v1.ts',
      'src/registration-transaction-admission-v1.ts',
      'src/registration-transaction-boundary-v1.ts',
      'src/registration-transaction-checkout-v1.ts',
      'src/registration-transaction-contract-v1.ts',
      'src/registration-transaction-retry-v1.ts',
      'src/registration-transaction-v1.ts',
    ];
    expect(Object.keys(artifact.buildInputs).sort()).toEqual([
      'package.json', 'scripts/artifact-lib.mjs', 'scripts/build.mjs',
      'scripts/deny-publish.mjs', 'scripts/verify-artifact.mjs',
      ...privateBuildInputs, 'tsconfig.json', 'vitest.config.ts',
    ].sort());
    for (const path of privateBuildInputs) {
      expect(artifact.buildInputs[path]).toMatch(/^[a-f0-9]{64}$/);
      expect(artifact.sourceInputs).not.toHaveProperty(path);
    }
    expect(artifact.buildInputs['vitest.config.ts']).toMatch(/^[a-f0-9]{64}$/);
    expect(artifact.externalImports).toEqual([]);
    expect(artifact.runtimePackages).toEqual([]);
    expect(artifact.artifactDigest).toMatch(/^[a-f0-9]{64}$/);
    expect(bundle.toString('utf8')).not.toMatch(/sourceMappingURL|\.\.\/|file:|workspace:/);

    const built = await import(`${pathToFileURL(bundlePath).href}?digest=${artifact.bundle.sha256}`);
    expect(Object.keys(built).sort()).toEqual([
      'decideSupervisorRegistrationV1',
      'fixedRegistrationTransportResponseV2',
      'parseCanonicalRegistrationRequestV2',
      'registrationChangedReplayEvidenceDigestV2',
      'supervisorServiceReadinessV1',
    ]);
    expect(built).not.toHaveProperty('executeSupervisorRegistrationTransactionV1');
    expect(built).not.toHaveProperty('recoverExactSupervisorRegistrationV1');
    expect(built).not.toHaveProperty('preparePostgresRegistrationMaterializationV1');
    expect(built).not.toHaveProperty('finalizePostgresRegistrationMaterializationV1');
    expect(built.supervisorServiceReadinessV1()).toMatchObject({
      operational: false,
      authority: 'none',
    });
  });

  it('rejects exact-bundle mutation and unsafe file mode', () => {
    expect(runScript('scripts/build.mjs').status).toBe(0);
    const original = readFileSync(bundlePath);
    try {
      chmodSync(bundlePath, 0o644);
      writeFileSync(bundlePath, Buffer.concat([original, Buffer.from('\n')]));
      expect(runScript('scripts/verify-artifact.mjs').status).not.toBe(0);
      writeFileSync(bundlePath, original);
      chmodSync(bundlePath, 0o664);
      expect(runScript('scripts/verify-artifact.mjs').status).not.toBe(0);
    } finally {
      writeFileSync(bundlePath, original);
      chmodSync(bundlePath, 0o444);
    }
    expect(runScript('scripts/verify-artifact.mjs').status).toBe(0);
  });

  it('rejects capability escalation and package drift before digest replay', () => {
    expect(runScript('scripts/build.mjs').status).toBe(0);
    const fixture = copySealedPackage();
    const fixtureManifest = resolve(fixture, '.service/manifest.json');
    const fixturePackage = resolve(fixture, 'package.json');
    const manifestBytes = readFileSync(fixtureManifest);
    const packageBytes = readFileSync(fixturePackage);
    try {
      for (const key of [
        'operational', 'externallyReachable', 'registrationMutationsEnabled',
        'databaseAccessEnabled', 'signerAccessEnabled', 'publicationAccessEnabled',
        'parentRuntimeDependencyAllowed',
      ]) {
        const attack = JSON.parse(manifestBytes.toString('utf8')) as Record<string, unknown>;
        attack[key] = true;
        writeFileSync(fixtureManifest, `${JSON.stringify(attack, null, 2)}\n`);
        const verified = runScript('scripts/verify-artifact.mjs', fixture);
        expect(verified.status, key).not.toBe(0);
        expect(verified.stderr, key).toContain('SUPERVISOR_SERVICE_MANIFEST_CONTRACT_INVALID');
      }
      const extra = JSON.parse(manifestBytes.toString('utf8')) as Record<string, unknown>;
      extra.unreviewedCapability = false;
      writeFileSync(fixtureManifest, `${JSON.stringify(extra, null, 2)}\n`);
      expect(runScript('scripts/verify-artifact.mjs', fixture).stderr)
        .toContain('SUPERVISOR_SERVICE_MANIFEST_KEYS_INVALID');
      writeFileSync(fixtureManifest, manifestBytes);

      for (const mutate of [
        (value: any) => { value.private = false; },
        (value: any) => { value.dependencies.pg = '8.23.0'; },
        (value: any) => { value.scripts.verify = 'true'; },
        (value: any) => { value.exports = './dist/supervisor-service.mjs'; },
      ]) {
        const attack = JSON.parse(packageBytes.toString('utf8'));
        mutate(attack);
        writeFileSync(fixturePackage, `${JSON.stringify(attack, null, 2)}\n`);
        const verified = runScript('scripts/verify-artifact.mjs', fixture);
        expect(verified.status).not.toBe(0);
        expect(verified.stderr)
          .toMatch(/SUPERVISOR_SERVICE_PACKAGE_[A-Z_]*(?:CONTRACT|KEYS)_INVALID/);
      }
      writeFileSync(fixturePackage, packageBytes);
      for (const path of [
        'migrations/0001-registration-state-v1.sql',
        'migrations/0002-registration-rls-v1.sql',
        'migrations/catalog-contract-v1.json',
        'migrations/manifest-v1.json',
        'migrations/provisioning-contract-v1.json',
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
        'src/registration-postgresql-migration-json-v1.ts',
        'src/registration-postgresql-migration-manifest-v1.ts',
        'src/registration-postgresql-migration-plan-v1.ts',
        'src/registration-postgresql-migration-reader-v1.ts',
        'src/registration-postgresql-migration-sql-policy-v1.ts',
        'src/registration-postgresql-migration-sql-scanner-v1.ts',
        'src/registration-postgresql-provisioning-contract-v1.ts',
        'src/registration-transaction-admission-v1.ts',
        'src/registration-transaction-checkout-v1.ts',
        'src/registration-transaction-contract-v1.ts',
        'src/registration-transaction-retry-v1.ts',
      ]) {
        const input = resolve(fixture, path);
        const bytes = readFileSync(input);
        writeFileSync(input, Buffer.concat([bytes, Buffer.from('\n')]));
        expect(runScript('scripts/verify-artifact.mjs', fixture).stderr, path)
          .toContain('SUPERVISOR_SERVICE_ARTIFACT_BINDING_MISMATCH');
        writeFileSync(input, bytes);
      }
    } finally {
      rmSync(fixture, { recursive: true, force: true });
    }
    expect(runScript('scripts/verify-artifact.mjs').status).toBe(0);
  });
});

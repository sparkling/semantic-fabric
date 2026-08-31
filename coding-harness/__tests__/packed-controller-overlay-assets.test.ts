// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync,
  rmSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { afterEach, describe, expect, it } from 'vitest';

interface Version {
  readonly name: 'issue8' | 'v5' | 'v6';
  readonly entry: string;
  readonly launcher: URL;
  readonly reviewExport?: string;
}

interface Fixture {
  readonly repository: string;
  readonly runtime: string;
  readonly store: string;
  readonly commit: string;
  readonly event: string;
  readonly assets: ReadonlyMap<string, Buffer>;
}

const GIT = '/usr/bin/git';
const NODE = '/usr/bin/node';
const PRIMARY_ENTRY = 'coding-harness/dist/issue-8-program.js';
const ASSET_PATHS = Object.freeze([
  'coding-harness/config/programme-v5-ruflo-schema-v2-overlay.json',
  'coding-harness/config/programme-v5-ruflo-schema-v2-memory-bridge.js.gz',
  'coding-harness/config/programme-v5-ruflo-schema-v2-memory-initializer.js.gz',
] as const);
const VERSIONS: readonly Version[] = [
  {
    name: 'issue8', entry: PRIMARY_ENTRY,
    launcher: new URL('../scripts/launch-issue-8.mjs', import.meta.url),
  },
  {
    name: 'v5', entry: 'coding-harness/dist/programme-v5-program.js',
    launcher: new URL('../scripts/launch-programme-v5.mjs', import.meta.url),
    reviewExport: 'prepareReviewableProgrammeV5Policy',
  },
  {
    name: 'v6', entry: 'coding-harness/dist/programme-v6-program.js',
    launcher: new URL('../scripts/launch-programme-v6.mjs', import.meta.url),
    reviewExport: 'prepareReviewableProgrammeV6Policy',
  },
];
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('packed controller Ruflo overlay asset closure', () => {
  it.each(VERSIONS)(
    'materializes exact committed overlay assets in the $name private runtime',
    (version) => {
      const fixture = controllerFixture(version);
      for (const path of ASSET_PATHS) {
        writeFileSync(join(fixture.repository, path), 'ambient-mismatch\n');
        chmodSync(join(fixture.repository, path), 0o600);
      }

      const result = runLauncher(fixture, version);

      expect(result.error).toBeUndefined();
      expect(result.signal).toBeNull();
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(JSON.parse(readFileSync(fixture.event, 'utf8'))).toEqual({
        assets: Object.fromEntries([...fixture.assets].map(([path, bytes]) => [
          path, { digest: sha256(bytes), mode: 0o400 },
        ])),
      });
      expect(existsSync(fixture.store)).toBe(false);
    },
    15_000,
  );

  it.each(VERSIONS)('rejects a missing $name runtime resource binding', (version) => {
    const fixture = controllerFixture(version, ASSET_PATHS[0]);
    const result = runLauncher(fixture, version);
    expect(result.error).toBeUndefined();
    expect(result.signal).toBeNull();
    expect(result.status).toBe(1);
    expect(result.stdout).toBe('');
    expect(JSON.parse(result.stderr)).toEqual({
      status: 'error', reason: 'HARNESS_BOOTSTRAP_BUILD_TREE_INVALID',
    });
    expect(existsSync(fixture.store)).toBe(false);
  });
});

function controllerFixture(version: Version, omittedResource?: string): Fixture {
  const repository = temporary('packed-overlay-repository-');
  const runtime = temporary('packed-overlay-runtime-');
  const template = join(runtime, 'empty-template');
  mkdirSync(template, { mode: 0o700 });
  git(repository, ['init', '--quiet', '--template=' + template]);
  git(repository, ['config', 'user.name', 'Harness Test']);
  git(repository, ['config', 'user.email', 'harness@example.invalid']);
  const manifest = Buffer.from('{"schemaVersion":1}\n');
  const lockfile = Buffer.from('{"lockfileVersion":3}\n');
  const packageJson = Buffer.from('{"name":"fixture","private":true,"type":"module"}\n');
  const assets = new Map<string, Buffer>([
    [ASSET_PATHS[0], Buffer.from(`${JSON.stringify({ schemaVersion: 1, files: [] }, null, 2)}\n`)],
    [ASSET_PATHS[1], gzipSync(Buffer.from('historical bridge\n'), { level: 9 })],
    [ASSET_PATHS[2], gzipSync(Buffer.from('historical initializer\n'), { level: 9 })],
  ]);
  const files = new Map<string, Buffer>([
    ['coding-harness/.harness/manifest.json', manifest],
    ['coding-harness/package-lock.json', lockfile],
    ['coding-harness/package.json', packageJson],
    [PRIMARY_ENTRY, Buffer.from('export const legacyEntry=true;\n')],
    [version.entry, Buffer.from(controllerModule(version, assets))],
    ['coding-harness/node_modules/fixture/package.json',
      Buffer.from('{"name":"fixture","version":"1.0.0"}\n')],
    ...assets,
  ]);
  for (const [path, bytes] of files) write(repository, path, bytes);
  const outputs = Object.fromEntries([PRIMARY_ENTRY, version.entry].sort()
    .map((path) => [path, sha256(files.get(path)!)]));
  const productionFiles = Object.fromEntries([
    'coding-harness/node_modules/fixture/package.json', ...ASSET_PATHS,
  ].filter((path) => path !== omittedResource).sort()
    .map((path) => [path, sha256(files.get(path)!)]));
  const buildBody = {
    schemaVersion: 1, authority: 'development-only-no-promotion',
    runtimeEntry: PRIMARY_ENTRY, harnessManifestDigest: sha256(manifest),
    lockfileDigest: sha256(lockfile), outputs, productionFiles,
  };
  write(repository, 'coding-harness/.harness/controller-build.json', Buffer.from(
    `${JSON.stringify({
      ...buildBody, runtimeTreeDigest: sha256(Buffer.from(JSON.stringify(buildBody))),
    }, null, 2)}\n`,
  ));
  git(repository, ['add', '--', '.']);
  git(repository, ['commit', '--quiet', '-m', 'controller']);
  const commit = gitText(repository, ['rev-parse', 'HEAD']);
  const store = mkdtempSync(join(runtime, 'semantic-fabric-controller-store-'));
  git(repository, ['init', '--quiet', '--bare', '--template=' + template, store]);
  const pack = git(repository, [
    'pack-objects', '--stdout', '--revs', '--no-reuse-object', '--no-reuse-delta',
  ], Buffer.from(`${commit}\n`));
  git(repository, [
    '-c', 'pack.writeReverseIndex=false', '--git-dir=' + store,
    'index-pack', '--strict', '--stdin',
  ], pack);
  git(repository, ['--git-dir=' + store, 'update-ref', 'refs/heads/controller', commit]);
  git(repository, ['--git-dir=' + store, 'symbolic-ref', 'HEAD', 'refs/heads/controller']);
  harden(store);
  return {
    repository, runtime, store, commit,
    event: join(repository, `${version.name}-assets.json`), assets,
  };
}

function controllerModule(version: Version, assets: ReadonlyMap<string, Buffer>): string {
  const expected = Object.fromEntries([...assets].map(([path, bytes]) => [
    basename(path), { path, hex: bytes.toString('hex'), digest: sha256(bytes) },
  ]));
  const common = [
    "import{appendFileSync,readFileSync,statSync}from'node:fs';",
    "import{dirname,join}from'node:path';",
    "import{fileURLToPath}from'node:url';",
    `const EXPECTED=${JSON.stringify(expected)};`,
    'function flag(argv,name){const i=argv.indexOf(name);return i<0?undefined:argv[i+1]}',
    'function record(argv){const config=join(dirname(fileURLToPath(import.meta.url)),"..","config");',
    'const assets=Object.fromEntries(Object.entries(EXPECTED).map(([name,spec])=>{',
    'const path=join(config,name),bytes=readFileSync(path);',
    'if(bytes.toString("hex")!==spec.hex)throw new Error("OVERLAY_ASSET_MISMATCH");',
    'return[spec.path,{digest:spec.digest,mode:statSync(path).mode&511}]}));',
    `appendFileSync(join(flag(argv,"--repository"),"${version.name}-assets.json"),JSON.stringify({assets}));`,
    '}',
  ];
  if (version.name === 'issue8') return [...common,
    'export async function trustedControllerMain(argv){record(argv);return{status:"pass",reason:null,',
    'seal:async()=>({status:"pass",receiptDigest:"1".repeat(64),',
    'programmeAcceptanceDigest:"2".repeat(64),envelopeDigest:"3".repeat(64)})}}',
    '',
  ].join('\n');
  const policyBlob = policy(version);
  return [...common,
    'function review(argv){record(argv);',
    `return{policyBlob:${JSON.stringify(policyBlob)},policyFingerprint:${JSON.stringify(sha256(Buffer.from(policyBlob)))}}}`,
    `export async function ${version.reviewExport}(argv){return review(argv)}`,
    '',
  ].join('\n');
}

function policy(version: Version): string {
  if (version.name === 'v5') return canonicalJson({ alpha: 1 });
  const baseGateContract = { schemaVersion: 1 };
  const basePolicy = {
    schemaVersion: 1,
    policyId: 'semantic-fabric-programme-v5-policy-v1',
    authority: 'development-only-no-promotion',
    gateContract: baseGateContract,
  };
  return canonicalJson({
    schemaVersion: 2,
    policyId: 'semantic-fabric-programme-v6-policy-v2',
    authority: 'development-only-no-promotion',
    basePolicy,
    basePolicyFingerprint: sha256(Buffer.from(canonicalJson(basePolicy))),
    gateContract: {
      schemaVersion: 2,
      contractId: 'semantic-fabric-programme-gate-contract-v2',
      authority: 'development-only-no-promotion',
      envelope: { schemaVersion: 6 },
      baseGateContract,
      baseGateContractDigest: sha256(Buffer.from(canonicalJson(baseGateContract))),
    },
  });
}

function canonicalJson(value: unknown): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  const record = value as Record<string, unknown>;
  return `{${Object.keys(record).sort().map((key) =>
    `${JSON.stringify(key)}:${canonicalJson(record[key])}`).join(',')}}`;
}

function runLauncher(fixture: Fixture, version: Version) {
  const previousUmask = process.umask(0o077);
  try {
    const args = [
      '--no-addons', '--disable-proto=throw', '--input-type=module', '-',
      '--repository', fixture.repository, '--controller-store', fixture.store,
      '--controller-commit', fixture.commit, '--run-id', `programme_${version.name}_run`,
      '--swarm-id', `programme_${version.name}_swarm`,
      '--coordination-task-id', `programme_${version.name}_task`,
      '--hive-id', 'hierarchical', '--consensus-id', 'raft',
      ...(version.name === 'issue8' ? [] : ['--policy-review', 'prepare-only']),
    ];
    return spawnSync(NODE, args, {
      input: readFileSync(version.launcher), encoding: 'utf8', maxBuffer: 2_000_000,
      timeout: 10_000, killSignal: 'SIGKILL',
      env: {
        XDG_RUNTIME_DIR: fixture.runtime,
        DBUS_SESSION_BUS_ADDRESS: `unix:path=${fixture.runtime}/bus`, LANG: 'C.UTF-8',
      },
    });
  } finally { process.umask(previousUmask); }
}

function write(root: string, path: string, bytes: Buffer): void {
  const target = join(root, path);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, bytes);
  chmodSync(target, 0o600);
}

function temporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  chmodSync(root, 0o700);
  roots.push(root);
  return root;
}

function harden(directory: string): void {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) harden(path);
    else chmodSync(path, 0o400);
  }
  chmodSync(directory, 0o500);
}

function git(cwd: string, args: readonly string[], input?: Buffer): Buffer {
  const result = spawnSync(GIT, args, {
    cwd, input, maxBuffer: 20_000_000,
    env: {
      PATH: '/usr/bin:/bin', HOME: '/nonexistent', LANG: 'C', LC_ALL: 'C',
      GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_ATTR_NOSYSTEM: '1', GIT_NO_REPLACE_OBJECTS: '1', GIT_NO_LAZY_FETCH: '1',
    },
  });
  if (result.status !== 0) throw new Error(result.stderr.toString('utf8'));
  return result.stdout;
}

function gitText(cwd: string, args: readonly string[]): string {
  return git(cwd, args).toString('utf8').trim();
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

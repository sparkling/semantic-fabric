import { spawnSync } from 'node:child_process';
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { runPostgresQualificationContainer } from
  '../src/postgres-observation-qualification-docker.js';
import { buildPostgresQualificationProbe } from
  '../src/postgres-observation-qualification-build.js';
import {
  POSTGRES_QUALIFICATION_BUILDER_EXPECTATION,
  expectedQualificationBuilderImageIdentity,
  expectedQualificationImageIdentity,
  parsePostgresQualificationProtocol,
  protocolImage,
} from '../src/postgres-observation-qualification-protocol.js';
import {
  runQualificationProcess,
  type QualificationProcessExecutor,
  type QualificationProcessResult,
} from '../src/postgres-observation-qualification-process.js';
import {
  collectQualificationProvenance,
  parsePostgresQualificationInventory,
  verifyQualificationSourceProvenance,
} from
  '../src/postgres-observation-qualification-provenance.js';

const REPOSITORY_ROOT = resolve(import.meta.dirname, '..', '..');
const PROTOCOL_TEXT = readFileSync(
  resolve(REPOSITORY_ROOT, 'tests/postgresql/observation-qualification-protocol-v1.json'),
  'utf8',
);
const protocol = parsePostgresQualificationProtocol(PROTOCOL_TEXT);
const temporaryRoots: string[] = [];

afterEach(() => {
  while (temporaryRoots.length > 0) rmSync(temporaryRoots.pop()!, { recursive: true, force: true });
});

describe('PostgreSQL qualification protocol and process boundary', () => {
  it('accepts only the closed immutable two-patch protocol', () => {
    expect(protocol.images.map((image) => image.patch)).toEqual(['16.9', '16.15']);
    expect(protocol.phases.slice(-2)).toEqual(['legacy-comparison', 'identity-build']);
    expect(() => parsePostgresQualificationProtocol(
      PROTOCOL_TEXT.replace('"authority": "observation-profile-only"',
        '"authority": "observation-profile-only",\n  "authority": "observation-profile-only"'),
    )).toThrow(/duplicate JSON key/);
    expect(() => parsePostgresQualificationProtocol(
      PROTOCOL_TEXT.replace('"requiredRuns": 2', '"requiredRuns": 1'),
    )).toThrow(/REQUIRED_RUNS_INVALID/);
    expect(() => parsePostgresQualificationProtocol(
      PROTOCOL_TEXT.replace('"patch": "16.9"', '"patch": "16.10"'),
    )).toThrow(/IMAGE_1_PATCH_INVALID/);
    expect(() => parsePostgresQualificationProtocol(
      PROTOCOL_TEXT.replace(protocol.builder.configDigest, `sha256:${'0'.repeat(64)}`),
    )).toThrow(/BUILDER_CONFIG_DIGEST_INVALID/);
  });

  it('bounds native process output and time without a shell', async () => {
    const success = await runQualificationProcess({
      file: '/usr/bin/printf', args: ['ok'], cwd: '/tmp', environment: { LANG: 'C' },
      timeoutMs: 1_000, maxOutputBytes: 16,
    });
    expect(success).toMatchObject({ status: 0, timedOut: false, outputLimitExceeded: false });
    expect(success.stdout.toString()).toBe('ok');

    const overflow = await runQualificationProcess({
      file: '/usr/bin/yes', args: [], cwd: '/tmp', environment: { LANG: 'C' },
      timeoutMs: 1_000, maxOutputBytes: 64,
    });
    expect(overflow.outputLimitExceeded).toBe(true);
    expect(overflow.stdout).toHaveLength(64);

    const timeout = await runQualificationProcess({
      file: '/usr/bin/sleep', args: ['2'], cwd: '/tmp', environment: { LANG: 'C' },
      timeoutMs: 10, maxOutputBytes: 16,
    });
    expect(timeout.timedOut).toBe(true);
  });

  it('accepts only the sorted, self-bound, seven-category source inventory', () => {
    const bytes = readFileSync(resolve(
      REPOSITORY_ROOT, 'tests/postgresql/observation-qualification-inputs-v1.tsv',
    ));
    const records = parsePostgresQualificationInventory(bytes);
    expect(records).toHaveLength(101);
    expect(new Set(records.map((entry) => entry.category))).toEqual(new Set([
      'adr', 'profile', 'queries', 'tests', 'fixture', 'runner', 'protocol',
    ]));
    expect(records).toContainEqual({
      category: 'protocol',
      path: 'tests/postgresql/observation-qualification-inputs-v1.tsv',
    });
    const lines = bytes.toString('utf8').trimEnd().split('\n');
    [lines[1], lines[2]] = [lines[2], lines[1]];
    expect(() => parsePostgresQualificationInventory(
      Buffer.from(`${lines.join('\n')}\n`),
    )).toThrow(/INVENTORY_ORDER_INVALID/);
    expect(() => parsePostgresQualificationInventory(
      Buffer.from(`${bytes.toString('utf8')}${lines[1]}\n`),
    )).toThrow(/INVENTORY_DUPLICATE_PATH/);
    expect(() => parsePostgresQualificationInventory(Buffer.from(
      bytes.toString('utf8').replace(
        'protocol\ttests/postgresql/observation-qualification-inputs-v1.tsv\n', '',
      ),
    ))).toThrow(/INVENTORY_SELF_BINDING_MISSING/);
  });

  it('rejects historical provenance after a current qualification input changes', async () => {
    const root = mkdtempSync(join(tmpdir(), 'semantic-fabric-pgq-provenance-'));
    temporaryRoots.push(root);
    const inventoryPath = 'tests/postgresql/observation-qualification-inputs-v1.tsv';
    const records = [
      ['adr', 'docs/adr.md'], ['fixture', 'tests/fixture.sql'], ['profile', 'src/profile.rs'],
      ['protocol', inventoryPath], ['queries', 'src/query.rs'], ['runner', 'Cargo.lock'],
      ['tests', 'tests/profile.test'],
    ] as const;
    for (const [, path] of records) {
      if (path === inventoryPath) continue;
      mkdirSync(dirname(resolve(root, path)), { recursive: true });
      writeFileSync(resolve(root, path), `${path}\n`);
    }
    mkdirSync(dirname(resolve(root, inventoryPath)), { recursive: true });
    writeFileSync(resolve(root, inventoryPath), `category\tpath\n${records
      .map(([category, path]) => `${category}\t${path}`).join('\n')}\n`);
    writeFileSync(resolve(root, '.gitignore'), '/target/\n');
    mkdirSync(resolve(root, 'target'), { recursive: true });
    writeFileSync(resolve(root, 'target/probe'), 'probe\n', { mode: 0o700 });
    git(root, ['init', '--initial-branch=main']);
    git(root, ['config', 'user.name', 'Qualification Test']);
    git(root, ['config', 'user.email', 'qualification@example.invalid']);
    git(root, ['add', '.']);
    git(root, ['commit', '-m', 'source']);
    const provenance = await collectQualificationProvenance({
      repositoryRoot: root,
      artifactPath: 'target/probe',
      rustcVv: Buffer.from([
        'rustc 1.96.0 (ac68faa20 2026-05-25)', 'binary: rustc',
        `commit-hash: ${'a'.repeat(40)}`, 'commit-date: 2026-05-25',
        'host: x86_64-unknown-linux-gnu', 'release: 1.96.0', 'LLVM version: 22.1.2', '',
      ].join('\n')),
      cargoVersion: Buffer.from('cargo 1.96.0 (30a34c682 2026-05-25)\n'),
      builderImage: expectedQualificationBuilderImageIdentity(
        POSTGRES_QUALIFICATION_BUILDER_EXPECTATION,
      ),
      probeStdoutSha256: 'a'.repeat(64),
      image: expectedQualificationImageIdentity(protocol.images[0]),
    });
    writeFileSync(resolve(root, 'receipt-pair.json'), '[]\n');
    git(root, ['add', 'receipt-pair.json']);
    git(root, ['commit', '-m', 'publish receipt']);
    await expect(verifyQualificationSourceProvenance(root, provenance)).resolves.toBeUndefined();
    writeFileSync(resolve(root, 'src/profile.rs'), 'drift\n');
    git(root, ['add', 'src/profile.rs']);
    git(root, ['commit', '-m', 'drift input']);
    await expect(verifyQualificationSourceProvenance(root, provenance))
      .rejects.toThrow(/CURRENT_PROFILE_INPUTS_MISMATCH/);
  });

  it('builds in an exact isolated image and cleans warnings fail closed', async () => {
    const root = qualificationRoot();
    const requests: Parameters<QualificationProcessExecutor['run']>[0][] = [];
    const creates = new Map<string, readonly string[]>();
    const token = 'fedcba9876543210fedcba9876543210';
    let warnOnCreate = false;
    const executor: QualificationProcessExecutor = { run: async (request) => {
      requests.push(request);
      const args = request.args;
      if (args[0] === 'image') return okLine(JSON.stringify({
        Id: protocol.builder.configDigest, Os: 'linux', Architecture: 'amd64',
        RepoDigests: [protocol.builder.reference],
      }));
      if (args[0] === 'create') {
        creates.set(args[2], args);
        return result({ status: 0, stdout: Buffer.from(`${'a'.repeat(64)}\n`),
          stderr: warnOnCreate ? Buffer.from('warning\n') : Buffer.alloc(0) });
      }
      if (args[0] === 'start') return okLine(args[1]);
      if (args[0] === 'wait') {
        if (args[1].includes('-build-')) {
          const targetMount = creates.get(args[1])!.find((arg) => arg.endsWith('dst=/target'))!;
          const target = targetMount.match(/src=([^,]+)/u)![1];
          const artifact = resolve(target, 'x86_64-unknown-linux-gnu/release',
            'postgres-observation-qualification');
          mkdirSync(dirname(artifact), { recursive: true });
          writeFileSync(artifact, 'container-built\n', { mode: 0o700 });
          chmodSync(artifact, 0o700);
        }
        return okLine('0');
      }
      if (args[0] === 'logs') {
        if (args[1].includes('-rustc-')) return okLine('rustc 1.96.0 exact');
        if (args[1].includes('-cargo-')) return okLine('cargo 1.96.0 exact');
        return ok();
      }
      if (args[0] === 'container' && args[1] === 'inspect') return okLine(token);
      if (args[0] === 'rm') return okLine(args[2]);
      if (args[0] === 'ps') return ok();
      throw new Error(`unexpected builder command: ${args.join(' ')}`);
    } };
    const dependencies = {
      executor, createToken: () => token, validateHost: () => {}, uid: 1000, gid: 1000,
      createTemporaryDirectory: () => {
        const path = mkdtempSync(join(tmpdir(), 'sf-pgq-build-test-'));
        temporaryRoots.push(path);
        return path;
      },
    };
    const evidence = await buildPostgresQualificationProbe({
      repositoryRoot: root, protocol, dependencies,
    });
    const createRequests = requests.filter((request) => request.args[0] === 'create');
    expect(createRequests).toHaveLength(4);
    for (const request of createRequests) {
      expect(request.args).toContain(protocol.builder.reference);
      expect(request.args).toEqual(expect.arrayContaining([
        '--read-only', '--user', '1000:1000', '--pull', 'never', '--cap-drop', 'ALL',
        '--platform', 'linux/amd64', 'HOME=/nonexistent', 'CARGO_HOME=/cargo',
      ]));
      expect(request.args).toContain(`type=bind,src=${root},dst=/workspace,readonly`);
      expect(request.args.join('\n')).not.toMatch(/(?:TOKEN|PASSWORD|CREDENTIAL|SSH_AUTH_SOCK)=/u);
    }
    const fetch = createRequests.find((request) => request.args.includes('fetch'))!;
    const build = createRequests.find((request) => request.args.includes('build'))!;
    expect(fetch.args[fetch.args.indexOf('--network') + 1]).toBe('bridge');
    expect(build.args[build.args.indexOf('--network') + 1]).toBe('none');
    expect(build.args).toContain('--offline');
    expect(evidence.builderImage.configDigest).toBe(protocol.builder.configDigest);
    expect(readFileSync(resolve(root, protocol.probe.artifactPath), 'utf8'))
      .toBe('container-built\n');
    expect(requests.filter((request) => request.args[0] === 'rm')).toHaveLength(4);
    warnOnCreate = true;
    await expect(buildPostgresQualificationProbe({ repositoryRoot: root, protocol, dependencies }))
      .rejects.toThrow(/DOCKER_COMMAND_FAILED/);
    expect(requests.at(-1)?.args[0]).toBe('ps');
  });
});

describe('PostgreSQL qualification Docker boundary', () => {
  it('verifies isolation and removes only its labelled resources', async () => {
    const root = qualificationRoot();
    const fake = new DockerFixture(root, false);
    const run = await runPostgresQualificationContainer({
      repositoryRoot: root,
      protocol,
      image: protocolImage(protocol, '16.9'),
      slot: 1,
      dependencies: fake.dependencies(),
    });
    expect(run.execution).toMatchObject({
      slot: 1,
      exitCode: 0,
      timedOut: false,
      cleanup: {
        containerRemoved: true,
        volumeRemoved: true,
        labelledContainersRemaining: 0,
        labelledVolumesRemaining: 0,
      },
    });
    expect(fake.commands).toContain('rm --force sf-pgq-0123456789abcdef0123456789abcdef');
    expect(fake.commands).toContain(
      'volume rm sf-pgq-volume-0123456789abcdef0123456789abcdef',
    );
    const weakened = new DockerFixture(root, false, undefined, true);
    await expect(runPostgresQualificationContainer({
      repositoryRoot: root, protocol, image: protocolImage(protocol, '16.9'), slot: 1,
      dependencies: weakened.dependencies(),
    })).rejects.toThrow(/CONTAINER_TMPFS_MISMATCH/);
  });
  it('still performs ownership-checked cleanup when the probe rejects', async () => {
    const root = qualificationRoot();
    const fake = new DockerFixture(root, true);
    await expect(runPostgresQualificationContainer({
      repositoryRoot: root,
      protocol,
      image: protocolImage(protocol, '16.9'),
      slot: 1,
      dependencies: fake.dependencies(),
    })).rejects.toThrow(/PROBE_FAILED/);
    expect(fake.commands.some((command) => command.startsWith('container inspect --format {{index')))
      .toBe(true);
    expect(fake.commands).toContain('rm --force sf-pgq-0123456789abcdef0123456789abcdef');
    expect(fake.commands).toContain(
      'volume rm sf-pgq-volume-0123456789abcdef0123456789abcdef',
    );
  });

  it('cleans resources created by a command that also writes a warning', async () => {
    const root = qualificationRoot();
    const containerWarning = new DockerFixture(root, false, 'container');
    await expect(runPostgresQualificationContainer({
      repositoryRoot: root,
      protocol,
      image: protocolImage(protocol, '16.9'),
      slot: 1,
      dependencies: containerWarning.dependencies(),
    })).rejects.toThrow(/DOCKER_COMMAND_FAILED/);
    expect(containerWarning.commands).toContain(
      'rm --force sf-pgq-0123456789abcdef0123456789abcdef',
    );
    expect(containerWarning.commands).toContain(
      'volume rm sf-pgq-volume-0123456789abcdef0123456789abcdef',
    );

    const volumeWarning = new DockerFixture(root, false, 'volume');
    await expect(runPostgresQualificationContainer({
      repositoryRoot: root,
      protocol,
      image: protocolImage(protocol, '16.9'),
      slot: 1,
      dependencies: volumeWarning.dependencies(),
    })).rejects.toThrow(/DOCKER_COMMAND_FAILED/);
    expect(volumeWarning.commands).toContain(
      'volume rm sf-pgq-volume-0123456789abcdef0123456789abcdef',
    );
  });

  it('attempts every cleanup and inventory check after either removal fails', async () => {
    for (const warningAt of ['container-remove', 'volume-remove'] as const) {
      const root = qualificationRoot();
      const fake = new DockerFixture(root, false, warningAt);
      await expect(runPostgresQualificationContainer({
        repositoryRoot: root,
        protocol,
        image: protocolImage(protocol, '16.9'),
        slot: 1,
        dependencies: fake.dependencies(),
      })).rejects.toThrow(/CLEANUP_FAILED/);
      expect(fake.commands).toContain(
        'rm --force sf-pgq-0123456789abcdef0123456789abcdef',
      );
      expect(fake.commands).toContain(
        'volume rm sf-pgq-volume-0123456789abcdef0123456789abcdef',
      );
      expect(fake.commands.some((command) => command.startsWith('ps --all --filter'))).toBe(true);
      expect(fake.commands.some((command) => command.startsWith('volume ls --filter'))).toBe(true);
    }
  });
});

class DockerFixture implements QualificationProcessExecutor {
  readonly commands: string[] = [];
  readonly token = '0123456789abcdef0123456789abcdef';
  readonly container = `sf-pgq-${this.token}`;
  readonly volume = `sf-pgq-volume-${this.token}`;
  readonly id = 'a'.repeat(64);

  constructor(
    private readonly root: string,
    private readonly failProbe: boolean,
    private readonly warningAt?: 'volume' | 'container' | 'container-remove' | 'volume-remove',
    private readonly weakenTmpfs = false,
  ) {}

  dependencies() {
    return {
      executor: this,
      createToken: () => this.token,
      now: () => 1,
      delay: async () => {},
      validateHost: () => {},
    };
  }

  async run(request: Parameters<QualificationProcessExecutor['run']>[0]) {
    const args = [...request.args];
    this.commands.push(args.join(' '));
    if (args[0] === 'image') return okLine(this.imageInspection());
    if (args[0] === 'volume' && args[1] === 'create') {
      return this.warningAt === 'volume'
        ? result({ status: 0, stdout: Buffer.from(`${this.volume}\n`),
          stderr: Buffer.from('warning\n') })
        : okLine(this.volume);
    }
    if (args[0] === 'volume' && args[1] === 'inspect'
      && args[3]?.startsWith('{"Name"')) return okLine(JSON.stringify({
        Name: this.volume, Labels: { [protocol.container.ownerLabelKey]: this.token },
      }));
    if (args[0] === 'create') {
      return this.warningAt === 'container'
        ? result({ status: 0, stdout: Buffer.from(`${this.id}\n`),
          stderr: Buffer.from('warning\n') })
        : okLine(this.id);
    }
    if (args[0] === 'container' && args[1] === 'inspect'
      && args[3]?.startsWith('{"Id"')) {
      return okLine(this.containerInspection(this.commands.includes(`start ${this.container}`)));
    }
    if (args[0] === 'start') return okLine(this.container);
    if (args[0] === 'exec' && args.at(-1) === '/proc/1/comm') return okLine('postgres');
    if (args[0] === 'exec' && args.includes('/usr/bin/pg_isready')) return ok();
    if (args[0] === 'exec' && args.at(-1) === protocol.container.probePath) {
      return this.failProbe ? result({ status: 78 }) : result({
        status: 0, stdout: Buffer.from('{"probe":true}'),
      });
    }
    if (args[0] === 'container' && args[1] === 'inspect') return okLine(this.token);
    if (args[0] === 'rm') {
      return this.warningAt === 'container-remove'
        ? result({ status: 0, stdout: Buffer.from(`${this.container}\n`),
          stderr: Buffer.from('warning\n') })
        : okLine(this.container);
    }
    if (args[0] === 'volume' && args[1] === 'inspect') return okLine(this.token);
    if (args[0] === 'volume' && args[1] === 'rm') {
      return this.warningAt === 'volume-remove'
        ? result({ status: 0, stdout: Buffer.from(`${this.volume}\n`),
          stderr: Buffer.from('warning\n') })
        : okLine(this.volume);
    }
    if (args[0] === 'ps' || (args[0] === 'volume' && args[1] === 'ls')) return ok();
    throw new Error(`unexpected fake Docker command: ${args.join(' ')}`);
  }

  private imageInspection(): string {
    const image = protocolImage(protocol, '16.9');
    return JSON.stringify({
      Id: image.configDigest,
      Os: 'linux',
      Architecture: 'amd64',
      RepoDigests: [image.reference, `postgres@sha256:${'f'.repeat(64)}`],
    });
  }

  private containerInspection(running: boolean): string {
    const fixture = resolve(this.root, 'tests/postgresql/observation-qualification-fixture.sql');
    const artifact = resolve(this.root, protocol.probe.artifactPath);
    return JSON.stringify({
      Id: this.id,
      Image: protocolImage(protocol, '16.9').configDigest,
      Name: `/${this.container}`,
      Running: running,
      NetworkMode: 'none',
      PortBindings: {},
      ReadonlyRootfs: true,
      Tmpfs: this.weakenTmpfs
        ? { '/tmp': 'rw,nosuid,nodev,size=64m', '/var/run/postgresql': 'rw' }
        : { '/tmp': 'rw,noexec,nosuid,nodev,size=64m',
          '/var/run/postgresql': 'rw,nosuid,nodev,size=16m' },
      Mounts: [
        { Type: 'volume', Name: this.volume, Source: '/volume',
          Destination: protocol.container.pgdataPath, RW: true },
        { Type: 'bind', Name: '', Source: fixture,
          Destination: protocol.container.fixturePath, RW: false },
        { Type: 'bind', Name: '', Source: artifact,
          Destination: protocol.container.probePath, RW: false },
      ],
      Labels: { [protocol.container.ownerLabelKey]: this.token },
      Env: [
        `PGDATA=${protocol.container.pgdataPath}`,
        'POSTGRES_HOST_AUTH_METHOD=trust',
        'POSTGRES_INITDB_ARGS=--locale-provider=libc --locale=C --encoding=UTF8',
      ],
    });
  }
}

function qualificationRoot(): string {
  const root = mkdtempSync(join(tmpdir(), 'semantic-fabric-pgq-test-'));
  temporaryRoots.push(root);
  const fixture = resolve(root, 'tests/postgresql/observation-qualification-fixture.sql');
  const artifact = resolve(root, protocol.probe.artifactPath);
  mkdirSync(resolve(fixture, '..'), { recursive: true });
  mkdirSync(resolve(artifact, '..'), { recursive: true });
  writeFileSync(fixture, 'fixture\n');
  writeFileSync(artifact, 'probe\n', { mode: 0o700 });
  chmodSync(artifact, 0o700);
  return root;
}

function git(root: string, args: readonly string[]): void {
  const result = spawnSync('git', ['-C', root, ...args], { encoding: 'utf8' });
  expect(result.status, result.stderr).toBe(0);
}

function okLine(value: string): QualificationProcessResult {
  return result({ status: 0, stdout: Buffer.from(`${value}\n`) });
}

function ok(): QualificationProcessResult {
  return result({ status: 0 });
}

function result(
  overrides: Partial<QualificationProcessResult> = {},
): QualificationProcessResult {
  return {
    status: null,
    signal: null,
    stdout: Buffer.alloc(0),
    stderr: Buffer.alloc(0),
    timedOut: false,
    outputLimitExceeded: false,
    spawnError: null,
    ...overrides,
  };
}

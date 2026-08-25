// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { deflateSync } from 'node:zlib';
import { afterEach, describe, expect, it } from 'vitest';

const GIT = '/usr/bin/git';
const NODE = '/usr/bin/node';
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('trusted issue #8 bootstrap', () => {
  it('reads only a strict verified pack even when the source object database is forged', () => {
    const fixture = controllerStore();
    const blob = gitText(fixture.source, ['rev-parse', 'HEAD:controller.txt']);
    const loose = join(fixture.source, '.git', 'objects', blob.slice(0, 2), blob.slice(2));
    const forged = Buffer.from('forged controller\n');
    chmodSync(loose, 0o600);
    writeFileSync(loose, deflateSync(Buffer.concat([
      Buffer.from(`blob ${forged.length}\0`), forged,
    ])));
    expect(gitText(fixture.source, ['show', `${fixture.commit}:controller.txt`]))
      .toBe('forged controller');
    expect(gitText(fixture.store, ['show', `${fixture.commit}:controller.txt`]))
      .toBe('trusted controller');

    const launcher = readFileSync(new URL('../scripts/launch-issue-8.mjs', import.meta.url));
    const previousUmask = process.umask(0o077);
    const result = spawnSync(NODE, [
      '--no-addons', '--disable-proto=throw', '--input-type=module', '-',
      '--repository', fixture.source,
      '--controller-store', fixture.store,
      '--controller-commit', fixture.commit,
      '--run-id', 'bootstrap_test_run',
      '--swarm-id', 'bootstrap_test_swarm',
      '--coordination-task-id', 'bootstrap_test_task',
      '--hive-id', 'bootstrap_test_hive',
      '--consensus-id', 'bootstrap_test_consensus',
    ], {
      input: launcher,
      env: {
        XDG_RUNTIME_DIR: fixture.runtime,
        DBUS_SESSION_BUS_ADDRESS: `unix:path=${fixture.runtime}/bus`,
        LANG: 'C.UTF-8',
      },
      encoding: 'utf8',
      maxBuffer: 2_000_000,
    });
    process.umask(previousUmask);

    expect(result.status).toBe(1);
    expect(result.stderr).toContain('HARNESS_BOOTSTRAP_GIT_BLOB_FAILED');
    expect(existsSync(fixture.store)).toBe(false);
  });
});

function controllerStore(): Readonly<{
  source: string;
  store: string;
  runtime: string;
  commit: string;
}> {
  const source = privateTemporary('trusted-bootstrap-source-');
  const runtime = privateTemporary('trusted-bootstrap-runtime-');
  const template = join(runtime, 'empty-template');
  mkdirSync(template, { mode: 0o700 });
  git(source, ['init', '--quiet', `--template=${template}`]);
  git(source, ['config', 'user.name', 'Harness Test']);
  git(source, ['config', 'user.email', 'harness@example.invalid']);
  writeFileSync(join(source, 'controller.txt'), 'trusted controller\n');
  git(source, ['add', '--', 'controller.txt']);
  git(source, ['commit', '--quiet', '-m', 'controller']);
  const commit = gitText(source, ['rev-parse', 'HEAD']);
  const store = mkdtempSync(join(runtime, 'semantic-fabric-controller-store-'));
  roots.push(store);
  git(source, ['init', '--quiet', '--bare', `--template=${template}`, store]);
  const pack = git(source, [
    'pack-objects', '--stdout', '--revs', '--no-reuse-object', '--no-reuse-delta',
  ], Buffer.from(`${commit}\n`));
  git(source, [
    '-c', 'pack.writeReverseIndex=false', `--git-dir=${store}`,
    'index-pack', '--strict', '--stdin',
  ], pack);
  git(source, [`--git-dir=${store}`, 'update-ref', 'refs/heads/controller', commit]);
  git(source, [`--git-dir=${store}`, 'symbolic-ref', 'HEAD', 'refs/heads/controller']);
  harden(store);
  return { source, store, runtime, commit };
}

function privateTemporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  roots.push(root);
  chmodSync(root, 0o700);
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
    cwd,
    input,
    env: {
      PATH: '/usr/bin:/bin', HOME: '/nonexistent', LANG: 'C', LC_ALL: 'C',
      GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_ATTR_NOSYSTEM: '1', GIT_NO_REPLACE_OBJECTS: '1', GIT_NO_LAZY_FETCH: '1',
    },
    maxBuffer: 20_000_000,
  });
  if (result.status !== 0) throw new Error(result.stderr.toString('utf8'));
  return result.stdout;
}

function gitText(cwd: string, args: readonly string[]): string {
  return git(cwd, args).toString('utf8').trim();
}

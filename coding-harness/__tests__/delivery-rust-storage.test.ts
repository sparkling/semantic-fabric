import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, it } from 'vitest';
import { ordinaryRustEnvironment } from '../src/delivery-process.js';

it('prevents inherited incremental override from regenerating caches in old snapshots', () => {
  const root = mkdtempSync(join(tmpdir(), 'fabric-rust-storage-'));
  try {
    writeFileSync(join(root, 'Cargo.toml'),
      '[package]\nname="ordinary-storage-proof"\nversion="0.1.0"\nedition="2021"\n' +
      '[lib]\npath="lib.rs"\n[workspace]\n');
    writeFileSync(join(root, 'lib.rs'),
      '#[test] fn safety() { assert!(cfg!(debug_assertions)); ' +
      'assert!(std::panic::catch_unwind(|| std::hint::black_box(u8::MAX) + 1).is_err()); }\n');
    const inherited = Object.fromEntries(Object.entries(process.env).filter(([key, value]) =>
      value !== undefined && !key.startsWith('CARGO_') &&
      !['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'].includes(key))) as Record<string, string>;
    const environment = ordinaryRustEnvironment({ ...inherited, CARGO_INCREMENTAL: '1' });
    expect(environment.CARGO_INCREMENTAL).toBe('0');
    for (const command of ['build', 'test']) {
      const output = spawnSync('cargo', [command, '--offline', '-vv', '--jobs', '1'], {
        cwd: root, env: environment, encoding: 'utf8', timeout: 10_000,
        stdio: ['ignore', 'pipe', 'pipe'],
      });
      expect(output.error).toBeUndefined();
      expect(output.status, output.stderr).toBe(0);
      const invocations = output.stderr.split('\n').filter(line =>
        line.includes('rustc ') && line.includes('--crate-name ordinary_storage_proof'));
      expect(invocations.length).toBeGreaterThan(0);
      for (const invocation of invocations) {
        expect(invocation).toContain('-C debuginfo=1');
        expect(invocation).not.toContain('-C incremental=');
      }
      if (command === 'test') expect(output.stdout).toContain('1 passed');
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

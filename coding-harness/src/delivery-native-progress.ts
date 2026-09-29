// SPDX-License-Identifier: MIT
import { appendFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { NativeStreamDecoder } from './delivery-native-stream.js';
import { NativeStderrTail } from './delivery-native-stderr.js';
import { identifier } from './delivery-contracts.js';
import type { NativeHost } from './models/types.js';
import type { NativeCommandObserver } from './delivery-process.js';

export function nativeCommandProgress(input: { directory: string; taskId: string; stage: string; host: NativeHost;
  warnMs?: number; cancelMs?: number; now?: () => number; environment?: NodeJS.ProcessEnv;
  notify?: (event: Record<string, unknown>) => void }): NativeCommandObserver & { output(): string } {
  const now = input.now ?? performance.now.bind(performance);
  const warnMs = input.warnMs ?? 120_000, cancelMs = input.cancelMs ?? 300_000;
  if (!Number.isFinite(warnMs) || !Number.isFinite(cancelMs) || warnMs <= 0 || cancelMs <= warnMs) throw Error('native-inactivity-policy-invalid');
  identifier(input.taskId);
  if (!['architecture','implementation','repair','review'].includes(input.stage)) throw Error('native-progress-identity-invalid');
  let lastActivity = now(), lastSnapshot = lastActivity, lastToolEvent = -Infinity, warned = false, cancelled = false, pid: number | null = null;
  const stderr = new NativeStderrTail(input.environment ?? process.env);
  const notices = new NativeStderrTail(input.environment ?? process.env);
  const tail: string[] = [];
  const path = join(input.directory, 'progress.jsonl'), tailPath = join(input.directory, 'native-tail.jsonl');
  writeFileSync(path, '', { flag: 'wx', mode: 0o600 });
  writeFileSync(tailPath, '', { flag: 'wx', mode: 0o600 });
  const write = (event: string) => {
    const row = { schemaVersion: 1, taskId: input.taskId, stage: input.stage, host: input.host,
      pid, at: new Date().toISOString(), event, inactiveMs: Math.max(0, Math.round(now() - lastActivity)), ...decoder.counts, ...decoder.digests() };
    const line = JSON.stringify(row) + '\n';
    appendFileSync(path, line);
    tail.push(line); if (tail.length > 32) tail.shift();
    writeFileSync(tailPath, tail.join(''));
    if (event === 'inactivity-warning' || event === 'inactivity-cancel') {
      const notification = { type: 'delivery-native-progress', ...row, evidencePath: path };
      if (input.notify) input.notify(notification);
      else process.stderr.write(JSON.stringify(notification) + '\n');
    }
  };
  const decoder = new NativeStreamDecoder(input.host, () => { lastActivity = now(); warned = false; }, event => {
    if (now() - lastToolEvent >= 1000) { lastToolEvent = now(); write(event); }
  }, undefined, message => notices.push(Buffer.from(message + '\n')));
  return {
    start(value) { pid = value ?? null; lastActivity = now(); lastSnapshot = lastActivity; write('started'); },
    stdout(data) { decoder.stdout(data); },
    stderr(data) { decoder.stderr(data); stderr.push(data); },
    tick() {
      if (cancelled) return 'native-inactivity';
      const elapsed = now() - lastActivity;
      if (decoder.error()) return decoder.error();
      if (elapsed >= cancelMs) { if (!cancelled) { cancelled = true; write('inactivity-cancel'); } return 'native-inactivity'; }
      if (elapsed >= warnMs && !warned) { warned = true; write('inactivity-warning'); }
      if (now() - lastSnapshot >= 5000) { lastSnapshot = now(); write('progress'); }
      return undefined;
    },
    finish() { decoder.finish(); write('stream-closed'); return { stdout: decoder.output(), stderr: stderr.finish() + notices.finish(), error: decoder.error() }; },
    output() { return decoder.output(); },
  };
}

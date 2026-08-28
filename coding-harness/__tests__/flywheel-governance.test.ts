// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';

interface AnchorTask {
  readonly id: string;
  readonly q: string;
  readonly labels: readonly string[];
}

interface Anchor {
  readonly schemaVersion: string;
  readonly version: string;
  readonly status: string;
  readonly tasks: readonly AnchorTask[];
}

interface AnchorManifest {
  readonly schemaVersion: string;
  readonly path: string;
  readonly sha256: string;
}

interface ProvenConfig {
  readonly championId: string;
  readonly manifest: {
    readonly layer: string;
    readonly policy: { readonly ref: string; readonly value: Readonly<Record<string, number>> };
  };
}

interface ActivePolicy {
  readonly championId: string;
  readonly layer: string;
  readonly params: Readonly<Record<string, number>>;
}

const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repositoryRoot = resolve(harnessRoot, '..');
const evalRoot = resolve(repositoryRoot, '.claude/eval');
const manifest = readJson<AnchorManifest>(resolve(evalRoot, 'flywheel-anchor.manifest.json'));
const anchor = readJson<Anchor>(resolve(evalRoot, 'semantic-fabric-relevance-anchor-v1.json'));
const settings = readJson<Record<string, unknown>>(resolve(repositoryRoot, '.claude/settings.json'));
const proven = readJson<ProvenConfig>(resolve(repositoryRoot, '.claude/proven-config.json'));
const active = readJson<ActivePolicy>(resolve(repositoryRoot, '.claude-flow/harness-active-policy.json'));
const provenVersion = readFileSync(resolve(repositoryRoot, '.claude/.proven-config-version'), 'utf8').trim();

describe('retrieval flywheel governance', () => {
  it('pins a broad candidate benchmark with the Ruflo canonical hash', () => {
    expect(manifest.schemaVersion).toBe('ruflo.flywheel-anchor-manifest/v1');
    expect(manifest.path).toBe('semantic-fabric-relevance-anchor-v1.json');
    expect(anchor.schemaVersion).toBe('ruflo.flywheel-anchor/v1');
    expect(anchor.version).toBe('semantic-fabric-relevance-anchor-v1');
    expect(anchor.status).toBe('candidate-maintainer-review-required');
    expect(anchor.tasks).toHaveLength(48);

    const ids = new Set<string>();
    for (const task of anchor.tasks) {
      expect(task.id).toMatch(/^[A-Za-z0-9._-]{1,128}$/);
      expect(ids.has(task.id)).toBe(false);
      ids.add(task.id);
      expect(task.q.trim()).not.toBe('');
      expect(task.labels.length).toBeGreaterThan(0);
      expect(task.labels.every((label) => label.trim() !== '')).toBe(true);
    }

    expect(humanEvalHash(anchor.tasks)).toBe(manifest.sha256);

    const sorted = [...anchor.tasks].sort((left, right) => left.id.localeCompare(right.id));
    const training = sorted.slice(0, sorted.length / 2);
    const holdout = sorted.slice(sorted.length / 2);
    expect(training.every((task) => task.id.startsWith('sf-a'))).toBe(true);
    expect(holdout.every((task) => task.id.startsWith('sf-b'))).toBe(true);
    expect(holdout.map((task) => task.labels)).toEqual(training.map((task) => task.labels));
  });

  it('keeps background tuning and legacy apply off by default', () => {
    const env = settings.env as Record<string, unknown> | undefined;
    expect(env).toHaveProperty('RUFLO_FUNNEL', '0');
    expect(env).not.toHaveProperty('RUFLO_HARNESS_LOOP');
    expect(env).not.toHaveProperty('RUFLO_FLYWHEEL_LEGACY_APPLY');

    const claudeFlow = settings.claudeFlow as Record<string, unknown>;
    const daemon = claudeFlow.daemon as Record<string, unknown>;
    expect(daemon.workers).not.toContain('harness');
  });

  it('checks local inherited policy-pointer consistency without claiming a signed promotion', () => {
    expect(proven.championId).toBe(proven.manifest.policy.ref);
    expect(policyRef(proven.manifest.policy.value)).toBe(proven.championId);
    expect(provenVersion).toBe(proven.championId);
    expect(active).toMatchObject({
      championId: proven.championId,
      layer: proven.manifest.layer,
      params: proven.manifest.policy.value,
    });
    expect(active.layer).not.toBe('repo/local');
  });

  it('protects the benchmark, its pin, and the opt-in settings from candidates', () => {
    expect(SECURE_HARNESS_CONFIG.requiredProtectedPaths).toEqual(expect.arrayContaining([
      '.claude-flow/harness-active-policy.json',
      '.claude/.proven-config-version',
      '.claude/eval/flywheel-anchor.manifest.json',
      '.claude/eval/semantic-fabric-relevance-anchor-v1.json',
      '.claude/proven-config.json',
      '.claude/settings.json',
    ]));
  });

  it('keeps local receipts and daemon-generation state out of version control', () => {
    for (const path of [
      '.claude-flow/flywheel-v1/transaction-state.json',
      '.claude-flow/flywheel/attempts.jsonl',
    ]) {
      const ignored = spawnSync('git', ['-C', repositoryRoot, 'check-ignore', '--quiet', path]);
      expect(ignored.status, ignored.stderr.toString()).toBe(0);
    }
  });
});

function humanEvalHash(tasks: readonly AnchorTask[]): string {
  const sorted = [...tasks].sort((left, right) => left.id.localeCompare(right.id));
  const bytes = JSON.stringify(canonical(sorted));
  return `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
}

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [
      key,
      canonical((value as Record<string, unknown>)[key]),
    ]));
  }
  return value;
}

function policyRef(params: Readonly<Record<string, number>>): string {
  const sorted = Object.fromEntries(Object.keys(params).sort().map((key) => [key, params[key]]));
  return `sha256:${createHash('sha256').update(JSON.stringify(sorted)).digest('hex')}`;
}

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, 'utf8')) as T;
}

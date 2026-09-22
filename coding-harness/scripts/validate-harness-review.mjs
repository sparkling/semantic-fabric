// SPDX-License-Identifier: MIT
// Validation for the standalone review document, not a product-runtime build.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { Script } from 'node:vm';

const file = new URL('../../docs/semantic-fabric-harness-review.html', import.meta.url);
const html = readFileSync(file, 'utf8');
const mode = process.argv[2];
assert(['structure', 'acceptance', 'served'].includes(mode), 'expected structure, acceptance or served');
const digest = value => createHash('sha256').update(value).digest('hex');
let checks = 0;
function check(name, fn) { fn(); checks++; console.log(`PASS ${name}`); }

if (mode === 'structure') {
  check('HTML5, language, viewport and project label', () => {
    assert.match(html, /^<!doctype html>/i);
    assert.match(html, /<html lang="en">/);
    assert.match(html, /<meta name="viewport" content="width=device-width, initial-scale=1">/);
    assert.match(html, /<title>semantic-fabric — Harness review<\/title>/);
    assert.match(html, /<h1>semantic-fabric<\/h1>/);
    assert.equal((html.match(/<h1>/g) ?? []).length, 1);
  });
  check('self-contained styles, scripts and diagram; no remote assets', () => {
    assert.match(html, /<style>/); assert.match(html, /<svg[^>]*viewBox=/);
    assert.doesNotMatch(html, /<(?:script|img|iframe|source)[^>]+src\s*=|<link[^>]+href\s*=|@import|url\(["']?https?:/i);
    assert.match(html, /connect-src 'none'/); assert.match(html, /form-action 'none'/);
    assert(html.split('\n').length < 500);
  });
  check('unique IDs and all local links resolve', () => {
    const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(m => m[1]);
    assert.equal(new Set(ids).size, ids.length);
    for (const [, id] of html.matchAll(/\bhref="#([^"]+)"/g)) assert(ids.includes(id), `missing #${id}`);
  });
  check('inline JavaScript parses', () => {
    const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
    assert.equal(scripts.length, 1);
    new Script(scripts[0][1]);
  });
}
if (mode === 'acceptance') {
  check('required review sections and evidence are present', () => {
    for (const id of ['overview','workflow','model-policy','evidence','limitations','sources','review']) {
      assert(html.includes(`id="${id}"`), id);
    }
    for (const text of ['b678c338aa1d6a4c2d36de0486c59b2a2792dabe','1,031','18','89.414 s',
      '55ea8a2d40174748e986c12bd799dfe8baa749310266e200eebfe63cadc339b4',
      'root-owned Node','systemd user service','ADR-0055','ADR-0037']) assert(html.includes(text), text);
  });
  check('review snapshot has model policy, limits and hold', () => {
    for (const text of ['Luna · low','Terra · medium','Sol · medium','Sol · high','Astra · high',
      'Haiku','Sonnet','Opus','not cryptographic provider attestation',
      'not a learned prediction','Application build remains paused','not been pushed']) assert(html.includes(text), text);
    assert.match(html, /color-scheme/); assert.match(html, /@media\(max-width:850px\)/);
  });
  check('feedback is exportable and cannot send or resume work', () => {
    for (const id of ['copy-feedback','feedback-output','copy-status']) assert(html.includes(`id="${id}"`));
    const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];
    assert.doesNotMatch(script, /\b(?:fetch|XMLHttpRequest|WebSocket|sendBeacon|localStorage|sessionStorage)\b/);
    assert.match(script, /does not authorize resumption/);
  });
}
if (mode === 'served') {
  const url = 'https://gene.tail448fa.ts.net/semantic-fabric-harness-review';
  check('HTTPS endpoint serves the exact reviewed HTML bytes', () => {
    const bytes = execFileSync('curl', ['--noproxy','*','--fail','--silent','--show-error','--max-time','20',url]);
    assert.equal(digest(bytes), digest(html));
  });
  check('Tailscale exposes only this file and has no public Funnel enabled', () => {
    const config = JSON.parse(execFileSync('tailscale', ['serve','status','--json'], { encoding:'utf8' }));
    const path = config.Web?.['gene.tail448fa.ts.net:443']?.Handlers?.['/semantic-fabric-harness-review']?.Path;
    assert.equal(path, file.pathname);
    assert(!Object.values(config.AllowFunnel ?? {}).some(Boolean));
  });
}
assert(checks > 0);
console.log(`${checks} ${mode} checks passed; HTML sha256 ${digest(html)}`);

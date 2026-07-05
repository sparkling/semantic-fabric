#!/usr/bin/env node
// reflect.mjs — ADR-0026 B5. GepaReflector: async (prompt) => { raw, cost }.
// One `claude -p` call with the prompt GEPA's own buildReflectionPrompt() builds. No other model, ever.

import { spawnSync } from 'node:child_process';

const REFLECT_TIMEOUT_MS = 5 * 60 * 1000;
const MAX_BUDGET_USD_PER_REFLECTION = 0.5;

/** @type {import('@metaharness/darwin').GepaReflector} */
export async function reflect(prompt) {
  const result = spawnSync('claude', [
    '-p', prompt,
    '--dangerously-skip-permissions',
    '--max-budget-usd', String(MAX_BUDGET_USD_PER_REFLECTION),
    '--output-format', 'json',
    '--model', 'sonnet',
    '--tools', '',
  ], {
    encoding: 'utf-8',
    timeout: REFLECT_TIMEOUT_MS,
    // Full env inheritance -- see evaluate.mjs for why (OS-keychain/OAuth auth, not an env var;
    // a narrow {PATH,HOME} scrub broke auth silently in the B6 pilot).
    env: process.env,
  });
  if (result.error) {
    throw new Error(`reflect: claude invocation failed: ${result.error.message}`);
  }
  let parsed;
  try {
    parsed = JSON.parse(result.stdout);
  } catch (e) {
    throw new Error(`reflect: could not parse claude JSON output: ${e.message}`);
  }
  if (parsed.is_error) {
    throw new Error(`reflect: claude reported an error (subtype=${parsed.subtype}): ${parsed.result || parsed.errors?.join('; ')}`);
  }
  return { raw: parsed.result ?? '', cost: parsed.total_cost_usd ?? 0 };
}

// CLI: node reflect.mjs <<< "$prompt"   (reads prompt from stdin, for quick manual testing)
if (import.meta.url === `file://${process.argv[1]}`) {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  const prompt = Buffer.concat(chunks).toString('utf-8');
  const result = await reflect(prompt);
  console.log(JSON.stringify(result, null, 2));
}

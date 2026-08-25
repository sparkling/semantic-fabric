// SPDX-License-Identifier: MIT

const mode = process.argv[2];

if (mode === 'environment') {
  console.log(JSON.stringify({
    safe: process.env.SAFE_FLAG,
    openai: process.env.OPENAI_API_KEY,
    openrouter: process.env.OPENROUTER_API_KEY,
    proxy: process.env.HTTP_PROXY,
    baseUrl: process.env.OPENAI_BASE_URL,
  }));
} else if (mode === 'output') {
  process.stdout.write('x'.repeat(20_000));
} else if (mode === 'wait') {
  setInterval(() => undefined, 1_000);
} else {
  process.exitCode = 2;
}

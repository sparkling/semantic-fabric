// SPDX-License-Identifier: MIT

const mode = process.argv[2];

if (mode === 'stdin') {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  process.stdout.write(JSON.stringify({
    input: Buffer.concat(chunks).toString('utf8'),
    openai: process.env.OPENAI_API_KEY,
    openrouter: process.env.OPENROUTER_API_KEY,
    proxy: process.env.HTTPS_PROXY,
  }));
} else if (mode === 'output') {
  process.stdout.write('x'.repeat(20_000));
} else if (mode === 'wait') {
  setInterval(() => undefined, 1_000);
} else {
  process.exitCode = 2;
}

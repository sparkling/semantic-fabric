@../AGENTS.md

# coding-harness host guidance

This directory is a private, development-only MetaHarness control plane. Shared
repository instructions come from `../AGENTS.md`.

- Use native Codex/ChatGPT and Claude Code subscription clients only.
- Never configure OpenRouter, Requesty, provider API keys, base-URL overrides,
  or proxy fallback.
- Treat Ruflo as the coordination ledger and Agentic-QE as advisory evidence;
  neither replaces direct product evaluators.
- Run candidate commands offline in an enforced process boundary. Dependency
  resolution is a separate, registry-pinned `npm ci` stage.
- Preserve the frozen evaluator, policy, lockfile, ADR, manifest, and `.mcp.json`
  digests. A repair must reset, re-admit, rebuild, and rerun every verifier.
- Require independent Codex and Claude reviews and emit a chained
  `development-only-no-promotion` receipt.
- Do not add a CLI, MCP server, publish/deploy path, or evolution command.

Local verification is `npm ci && npm run build && npm test`. Tests must use fake
native executables and must not contact a model provider.

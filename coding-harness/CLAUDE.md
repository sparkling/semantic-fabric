@../AGENTS.md

# coding-harness host guidance

This directory is a private, development-only MetaHarness control plane. Shared
repository instructions come from `../AGENTS.md`.

- Use native Codex/ChatGPT and Claude Code subscription clients only.
- Never configure OpenRouter, Requesty, provider API keys, base-URL overrides,
  or proxy fallback.
- Treat Ruflo as the coordination ledger and Agentic-QE as advisory evidence;
  neither replaces direct product evaluators.
- Every building task uses the mandatory main-only delivery path in README.md;
  bind the actual native model/effort, then use `advance`/`submit` for source-bound
  implementation, automatic checks, feedback-directed repair and independent
  read-only native review. Kernel verification runs per ready stage, while the
  existing host executes requests; do not launch a second build host. Honor user
  review holds. The remaining candidate-specific rules describe the optional experiment.
- Run historical candidate commands offline in an enforced process boundary. Dependency
  resolution is a separate, registry-pinned `npm ci` stage.
- Preserve the frozen evaluator, policy, lockfile, ADR, manifest, and `.mcp.json`
  digests. A repair must reset, re-admit, rebuild, and rerun every verifier.
- Require independent Codex and Claude reviews and emit a chained
  `development-only-no-promotion` receipt.
- The delivery CLI manages native handoffs and checks, never a competing host,
  commit/push, publication/deployment, an MCP server, or evolution.

Local verification is `npm ci && npm run build && npm test`. Tests must use fake
native executables and must not contact a model provider.
Manifests use `latest`; retain the committed lockfile's exact tested resolution.

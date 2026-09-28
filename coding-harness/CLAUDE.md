@../AGENTS.md

# coding-harness host guidance

This directory is a private, development-only MetaHarness control plane. Shared
repository instructions come from `../AGENTS.md`.

- Owner amendment 2026-09-28: native Codex and Claude Code plus the isolated
  direct OpenRouter adapter are authorized under ADR-0058. Preserve native
  gateway configuration; no silent transport fallback. API keys stay out of
  native, tool and verifier environments.
- Treat Ruflo as the coordination ledger and Agentic-QE as advisory evidence;
  neither replaces direct product evaluators.
- Direct implementation, repair, tests and builds are authorized for all work;
  delivery is useful orchestration, not a compulsory gate. When selected,
  bind the actual model/effort, then use `advance`/`submit` for source-bound
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

Use focused tests and the package build for coherent slices; broad tests at the
outcome join. Unit tests use injected transports. The owner-authorized initial
ADR-0058 proof may invoke configured live models through the production adapter.
Manifests use `latest`; retain the committed lockfile's exact tested resolution.

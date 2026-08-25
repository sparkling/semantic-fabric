---
status: superseded
date: 2026-07-16
updated: 2026-08-25
tags: [dev-process, metaharness, darwin-mode, historical]
superseded-by:
  - ADR-0037
supersedes: []
depends-on: []
implements: []
---

# Historical MetaHarness and Darwin-mode development-process experiment

## Status

Superseded by ADR-0037. This concise record preserves the decision lineage;
Git history retains the full experimental report that previously occupied this
file.

## Historical outcome

The 2026-07 experiment established several useful facts:

- MetaHarness and Darwin can remain development-process-only and removable
  from every `sf-*` runtime crate.
- Repository-generic Darwin runs and synthetic agent-mode scores do not prove
  semantic-fabric product quality. Direct Cargo, W3C, differential, live-source,
  and mutation oracles remain authoritative.
- Evolution is meaningful only after a discriminating repository-specific
  evaluator and sealed holdouts exist; a completed run or selected winner is
  not evidence of improvement by itself.
- Score, genome, MCP scan, and threat-model commands are diagnostics. Their
  visibility gaps must be fixed before a clean result can be treated as
  security assurance.

The experiment's ignored `semantic-fabric-harness/` tree is non-authoritative
local history. The versioned `coding-harness/` directory is the only candidate
for the current engineering control plane, and it must be upgraded and
validated rather than implicitly merged with ignored or untracked state.

## Withdrawn execution guidance

Historical references to OpenRouter, Requesty, API-key-backed fallback, or a
custom OpenRouter Darwin mutator are explicitly withdrawn. They authorize no
model execution, routing, retry, fallback, or mutation in this repository.

All current model work uses native OpenAI and Anthropic provider clients and
native subscription authentication. Ruflo coordinates the ledger; native
Codex/ChatGPT and Claude workers execute and independently review hard changes.

## Current decision

ADR-0037 defines the complete dual-host Ruflo engineering MetaHarness,
including its policy gates, router, persistent pool, critique/review/repair
loops, receipts, evaluator boundaries, Agentic-QE role, Darwin/GEPA threshold,
and AVO boundary. ADR-0026 and ADR-0027 remain the accepted records for
Agentic-QE adoption and measured load testing respectively.

## Rules retained from this record

Moved to ADR-0037 Rules R1–R4. This record retains none; consult ADR-0037.

# ruflo MetaHarness — what it is and how it applies to semantic-fabric

> **Historical (2026-07-05).** Grounding for the now-superseded ADR-0013. The read-layer half (§2–3) remains accurate; the Darwin/evolve half was invalidated by first-hand verification — see [ADR-0025](../adr/ADR-0025-claude-only-gepa-harness-evolution.md).

**Research key:** `ruflo-metaharness`
**Date:** 2026-06-27
**Evidence grade:** High — read from the installed packages + the upstream ADR + npm metadata, cross-checked with the ruvnet repos.

---

## 1. What it is

The **MetaHarness** is a tool that **analyses and scaffolds AI-agent harnesses** — *"where ruflo is a harness, metaharness analyses harnesses."* Same author (ruvnet / rUv) as ruflo. Upstream: [ruvnet/agent-harness-generator](https://github.com/ruvnet/agent-harness-generator) ("scaffold your own branded agent harness with its own npx CLI, MCP server, memory, learning loop, witness-signed releases").

Distributed as npm packages, **installed here:**

| Package | Version | Role |
|---|---|---|
| `metaharness` | **0.2.7** | umbrella CLI (`metaharness` / `harness` bins) — the **read** layer |
| `@metaharness/darwin` | **0.3.1** | the **write** layer (evolve) — see `ruflo-metaharness-darwin.md` |
| `@metaharness/router` | ~0.3.2 | cost-optimal model routing |
| `@metaharness/kernel` | ~0.1.0 | shared kernel |

ruflo wraps these as the **`ruflo-metaharness` plugin** (scripts that shell to `npx metaharness …`), under four architectural constraints (upstream ADR-150): **removable · optional (`optionalDependencies` only, never `dependencies`) · graceful degradation (`{degraded:true}`, exit 0 when absent) · CI-gate**.

## 2. The read surface (ADR-150)

| Command / MCP tool | Output |
|---|---|
| `score` | 5-dim readiness scorecard — harnessFit / compileConfidence / taskCoverage / toolSafety / memoryUsefulness + estCostPerRunUsd + scaffoldReady + **recommended template + archetype** |
| `genome` | 7-section categorical report — repo_type / agent_topology / risk_score / mcp_surface / test_confidence / publish_readiness |
| `mcp-scan` | static security scan of `.mcp/servers.json` + `.harness/claims.json` |
| `threat-model` | enterprise-review threat model (`worst` + categorised findings) |
| `oia-audit` | composite (oia-manifest + threat-model + mcp-scan), timestamped, stored in memory |
| `similarity` | weighted genome-fingerprint comparison (ADR-152) |
| `drift-from-history` | readiness drift across snapshots |
| `mint` | **scaffold** a harness (the *birth* verb) — DRY-RUN by default, `--confirm` to write |

## 3. Profiles (the three layers)

* **Templates / verticals** (`mint --template`): **minimal + 19 verticals** (coding, devops, support, legal, …) × ~9 **hosts** (claude-code, codex, pi-dev, opencode, github-actions, …).
* **Archetype** (`score`): the repo's classification.
* **agent_topology** (`genome`): recommended agent roles.

**semantic-fabric is already profiled** (from our score/genome runs): template `vertical:coding`, archetype `rust-crate-harness`, topology `maintainer · tester · security · release`, repo_type `rust_ci`.

## 4. How it applies to semantic-fabric

* **Usable now — readiness telemetry.** `score` + `genome` snapshotted in CI track agent-readiness drift as the engine matures (baseline captured 2026-06-26: `unknown`→`rust_ci`, compileConfidence 12→100, scaffoldReady false→true). This is ADR-0013 role #1.
* **The profile is the Darwin seed.** `mint --template vertical:coding` produces the baseline agent-harness genome that `@metaharness/darwin` then evolves (see the darwin report). Without minting a profile there is no genome for evolve to mutate.
* **Security surface (mcp-scan / threat-model / oia-audit)** has nothing to scan yet (`toolSafety 100`, `local_default_deny`); activates when semantic-fabric gains an MCP surface.
* **Caveat:** `score` is a readiness *gate*, not a quality discriminator (~0.985 ceiling) — engine quality is measured by the test harness (ADR-0005/0012), never the meta-harness score.

## 5. Sources
- [ruvnet/agent-harness-generator](https://github.com/ruvnet/agent-harness-generator) — upstream meta-harness (High)
- [ruvnet/ruflo `docs/metaharness-user-guide.md`](https://github.com/ruvnet/ruflo/blob/main/docs/metaharness-user-guide.md) (High)
- [MetaHarness × Ruflo Integration Dossier (ADR-150 companion)](https://gist.github.com/ruvnet/19d166ff9acf368c9da4172d91ac9113) (Medium)
- Installed packages: `metaharness@0.2.7`, `@metaharness/darwin@0.3.1`, `@metaharness/{router,kernel}` (High — read locally)
- ruflo upstream `ADR-150` (integration surfaces) + the `ruflo-metaharness` plugin scripts (High — read locally)

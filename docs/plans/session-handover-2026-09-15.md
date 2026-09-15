# Semantic Fabric — session handover

Prepared 2026-09-15 at the user's request. Application implementation is stopped
for transfer to another session. This document is local repository material,
not release evidence or publication authorization.

## Read first

1. Read the current [AGENTS.md](../../AGENTS.md), including applicable personal
   instructions supplied by the user.
2. Read accepted [ADR-0055](../adr/ADR-0055-v1-product-completion-and-release-profile.md)
   and the G1–G6 ledger in the
   [completion programme](sota-application-completion-programme.md).
3. Read [coding-harness/README.md](../../coding-harness/README.md), especially
   ownership, pause/resume, adoption, native responses and exact-commit finish.

The objective remains **complete the agreed application, plan, programme and
ADRs**. Neither demo readiness nor completion of individual helpers satisfies it.
ADR-0038 is historical; do not restore deferred research as v1 prerequisites.
The user rejected arbitrary deadlines and percentages: do not invent either.

## Authoritative checkpoint

- Checkout: `/home/claude/src/hm/semantic-fabric`, canonical `main`.
- Application base HEAD at handover inspection:
  `c3d2ca3788d89796ce65692c648a6b16eb5954a1`
  (`test(serve): verify real SQLite worker shutdown ownership`).
- This handover is a separate documentation-only commit after that base.
- Inspection found **60 dirty/untracked paths**, before adding this document.
  Tracked diff: 48 files, 6,377 insertions and 1,537 deletions. Untracked source
  modules are additional; `git diff` alone does not capture them.
- Application changes are **not committed, not finally reviewed, not verified
  as a complete harness task**, and not pushed. Preserve them all.
- Unrelated files, explicitly excluded from the application task:
  `.claude/helpers/.helpers-version`, `.claude/helpers/helpers.manifest.json`,
  `.claude/helpers/hook-handler.cjs`. Do not stage, revert or absorb them.
- No `cargo`/`rustc` process appeared in the handover process inspection. Previous
  test handles were terminal. Recheck live processes before starting checks.
- Last interrupted action only read the code and harness request. It did **not**
  implement the proposed nonconstant operand clone fix.

## Harness ownership and transfer

Task: `controlled-source-preparation-recovery-20260913`.
It was explicitly **paused for this handover** through the supported harness API;
patches remain intact. Do not mark it complete.

Recorded old identity (historical, never impersonate it in a new session):

- owner/executor: `fabric-projection-root-20260911`
- thread: `01a03612-aacf-70e1-ad27-063b931641b2`
- host/model/effort: native Codex / `gpt-6-astra` / `max`
- Ruflo task: `task-1789179393019-jiuv9f`
- previous pending implementation request:
  `c8c43d42301944222a7dfbf9fcedd5346a5327be7da14e8ea09c5467454981c9`
- before pause: no submitted implementation result, no harness check records,
  no final independent review or finish. Manual test passes below are not a
  substitute for that lifecycle.

Read-only inspection:

```bash
git branch --show-current
git status --short
git log -1 --format='%H %s'
node --input-type=module -e '
import {DeliveryHarness} from "./coding-harness/dist/delivery-runtime.js";
const h = new DeliveryHarness(process.cwd());
console.log(h.inspect());
console.log(h.read("controlled-source-preparation-recovery-20260913"));
'
```

The handover commit changes the original task's base/outside snapshot. Do not
force `resume`, reuse its stale request hash, edit receipt JSON, or bypass its
digest checks. Use the documented supported transition: retain the paused task
as unfinished history, explicitly adopt the existing in-scope patches into a
successor task on the current HEAD with **the same full requirement and checks**,
then bind the actual new native executor/model/thread. Inspect the old task's
exact scope rather than guessing it. A prior recovery used this same adoption
mechanism after unrelated helper updates changed the outside snapshot.

Task requirement:

> G1c full source preparation: prospectively govern dedup/branch preparation,
> live metadata materialization/traversal/catalog, validation, SQL/params/twin
> emission and per-branch indexes. Same SourceWork identity on
> cold/warm/single/two-source, exact raw parity, early refusal and recovery.
> Metadata support alone is incomplete.

Scope contains emitter modules, selected executor/backend files, tests and the
existing programme/ADR/catalogue artifacts. It does not automatically authorize
changes to sf-core, compiler facades or new modules. Resolve necessary scope
changes explicitly in the harness; do not silently broaden it.

## What is integrated versus still a patch

Integrated receipts reconciled in the programme:

- `49b9092`: G1b combined compiler/cache profile acceptance.
- `85fc1ef`: post-compile admission.
- `c3d2ca3`: real SQLite worker-shutdown ownership and capacity recovery.

These do not close all G1. **G1c and G1d remain open; G2–G6 remain open.**

Current unfinished patch groups:

| Area | Implementation present | Important limitation |
|---|---|---|
| `sf-sql/source_work.rs` | Prospective charges, vectors, strings, checked parameter binding and nested parameter moves | Logical accounting, not physical allocator/CPU proof |
| Backend metadata | Controlled live materialization, declaration interpretation and source metadata paths | Full public phase qualification pending |
| Branch/projection preparation | Borrowed modifiers and base-first binding overlay; controlled DISTINCT/projection/index work | Not a complete SQL emitter bound |
| `emit/metadata*.rs` | Iterative child/continuation traversal; paid projection, path, reference and subplan metadata transforms/merges | Component review does not cover later rendering changes |
| Rendering handoffs | Same SourceWork passed through regular/aggregate FROM, scans, nested plans, conditions and policy conjunctions | Recursion, some clones and SQL allocation still raw |
| Parameters | Direct, template, IRI, decimal, float, identity and natural-literal binds use checked admission | Numeric parsing/canonicalization can still precede binding without sufficient control |
| Natural decimal | Constant normalization prepays linear work and n+1 output; preserves control errors | Nonconstant operand still clones without prospective accounting |

Untracked modules that must not be lost:

```text
crates/sf-sparql/src/emit/metadata.rs
crates/sf-sparql/src/emit/metadata_path.rs
crates/sf-sparql/src/emit/metadata_projection.rs
crates/sf-sparql/src/emit/metadata_ref_atom.rs
crates/sf-sparql/src/emit/metadata_source.rs
crates/sf-sparql/src/emit/metadata_subplan.rs
crates/sf-sparql/src/emit/metadata_tests.rs
crates/sf-sparql/src/emit/source_control.rs
crates/sf-sparql/src/emit/source_control_tests.rs
crates/sf-sparql/src/exec_core/source_prepare.rs
crates/sf-sparql/src/exec_core/source_prepare_tests.rs
crates/sf-sql/src/source_work.rs
```

## Resume at the actual gaps

1. Inspect `emit/natural_decimal.rs`'s `normalize` closure: the nonconstant arm
   still returns `value.clone()`. Account for actual owned fields or avoid the
   copy without changing operand semantics. The interrupted session had located
   `TermSpec` in `sf-core/src/ir.rs`, but had not yet read its fields or edited it.
2. Finish numeric input parsing/canonicalization control. Read implementations
   rather than assuming an input multiplier bounds library parsing. Relevant:
   `pg_float_value.rs`, `mysql_float_value.rs`, `mysql_float_identity.rs`,
   `literal_cmp.rs`, `sf-core/src/numeric_compare.rs`, `sf-core/src/datatype.rs`.
   A 1024-byte IEEE display allowance bounds the formatted buffer, **not parsing**.
3. Finish SQL construction and metadata-parser admission. Important remaining
   paths include `emit_subplan_sql_controlled`'s synthetic/live catalogue work,
   placeholder rebasing, path/aggregate SQL generation, SQL AST emission, key
   renderers/percent encoding, SQLite metadata-twin comparisons and parser work.
   Charging a completed SQL string does not govern its earlier allocations.
4. Remove/control recursive condition/render traversal and metadata clones;
   `ref_atom::sql` still clones a legacy branch and calls duplicate-safety
   rewriting. Metadata walkers are iterative; SQL rendering is not thereby
   stack-safe. `ColumnCatalog.clone()` uses Arc maps: do not mistake it for a
   deep copy and waste time metering imaginary allocations.
5. Close the whole G1c phase with public cold/warm, ordinary/bearer,
   single/two-source exactness, early refusal, cancellation/deadline and capacity
   recovery at unchanged defaults. Then complete G1d combined qualification.
6. Work through the existing G2–G6 ledger, not a new research programme:
   exact RDF/query behavior; coherent generation/lifecycle; declared federation;
   owned backend/TLS matrix; exact candidate release checks.

No new feature exclusions, increased product limits or deferred required
correctness guarantees are authorized. Avoid another endless helper-by-helper
programme: remaining local fixes must lead to full public acceptance and a
verified integration commit.

## Evidence available, with limits

These are recorded **2026-09-13** runs, not fresh 2026-09-15 qualification:

- Latest full SPARQL library suite: **1,131 passed, four ignored**, 14.89 s,
  after natural-literal/identity control propagation. Later decimal-normalizer
  edits had focused checks, not another complete suite.
- Latest focused natural-decimal suite: **four passed**, including strengthened
  exact-N/N−1 and long-decimal fixtures; serving CLI build passed afterward.
- SourceWork suite: **nine passed** after checked parameter operations.
- Last public query-budget suite: **89 passed**, 9.48 s, before later
  specialized-binding/normalization changes. Requalify affected public paths.
- Metadata evidence includes exact-N/N−1, sticky cancellation/deadline,
  hash-order stability, raw fact parity and deep 128-KiB-stack traversal tests.
- Independent native Sol reviewer `projection_metadata_review` approved metadata
  source layout, projection, finalization and per-arm merging. Those approvals
  are historical component reviews, **not approval of all current changes**.
- Final harness checks, independent exact-delta review, commit and `finish`
  remain undone for the complete source-preparation phase.

Declared task commands (run through the harness workflow when ready):

```bash
cargo test --locked -p sf-sql --lib
cargo test --locked -p sf-sparql --lib
cargo test --locked -p sf-serve --tests
cargo test --locked -p sf-conformance --test capability_matrix --test capability_lineage
cargo build --locked -p sf-cli --no-default-features
cargo fmt --all --check
git diff --check
```

Full locked workspace/release qualification remains required at the meaningful
integration/candidate boundaries in ADR-0055. Do not rerun every historical test
per small edit, but do not promote focused passes to whole-feature evidence.
Use shell `&&` for chained qualification so an earlier failure cannot be hidden
by a later successful command's exit code.

## Memory, models and operating rules

- Ruflo only through live structured MCP. Never read managed memory files,
  use SQL/CLI fallbacks, bulk-import or replace sidecars.
- Repository namespace: `programme-outcomes`. Useful exact keys:
  `20260913-source-preparation-recovery`,
  `20260913-subplan-merge-controls-wip`,
  `20260913-parameter-admission-wip`,
  `20260913-iri-binding-controls-wip`,
  `20260913-decimal-binding-controls-wip`,
  `20260913-float-value-binding-wip`,
  `20260913-identity-natural-binding-wip`,
  `20260913-decimal-normalization-admission-wip`.
- Retrieve through discovered MCP schemas; this document and source suffice if
  recall fails. Cross-project memory access was unavailable earlier, not a gate.
- Native subscription authentication only. No provider keys/OpenRouter or
  quota/spend ceilings. Report exact native availability errors; do not silently
  substitute a model or claim a new session is the old executor.
- One writer on `main`. Use bounded independent read-only review when useful;
  a native reviewer is not by itself proof of a tracked Ruflo swarm.
- Scheduler was cancelled by the user. **Leave it off.** This handover does not
  restart an automatic writer or resume host.
- Node is harness/evidence infrastructure only; runtime stays Rust/Cargo.
- No Product Mock/live database access, flywheel enablement, push, tag, deploy,
  publication, branch/worktree creation or cleanup is granted by this handover.
- Do not commit the full dirty tree. Commit only verified, authorized slices;
  keep unrelated helper updates separate. Many emitter files are near the
  500-line limit: check before adding more inline tests or wrappers.

## Handover acceptance

The outgoing session has preserved the code, paused its harness claim and
committed only this document. The next session must establish sole ownership,
adopt/rebind through the harness and continue the full accepted scope. This is
**not an application completion report**. No defensible programme ETA is
available until the remaining public acceptance gaps are measured.

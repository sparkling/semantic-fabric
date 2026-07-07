---
status: accepted
date: 2026-06-28
tags: [ontop-parity, ws-g, wave-1, test-port, oracle, iq-optimizer, differential-testing, optional, self-join]
supersedes: []
depends-on:
  - ADR-0005
  - ADR-0007
  - ADR-0012
implements:
  - ADR-0021
---

# WS-G — port Ontop's IQ-optimizer test suite as the Rust oracle (Wave 1)

## Context and Problem Statement

WS-A (optimizer parity + the Q5 OPTIONAL fix) is the highest-measured-value workstream of the Ontop-parity program (ADR-0021), and also the **hardest correctness surface** — self-/self-left-join elimination over nullable determinants is the leak between SPARQL's partial-function/3VL semantics and SQL's NULL regime that took Ontop ~a decade to stabilise (ADR-0007). Changing the cascade without an oracle risks trading correctness for speed.

Ontop already encodes that oracle: its JUnit **IQ-optimizer tests** assert exactly which input IQ trees collapse to which optimized trees. **Wave 1 ports those tests to Rust** — as equivalence/oracle tests against semantic-fabric's `iq.rs` and the ADR-0012 `=_bag` differential — *before* any WS-A optimizer change, so the ported (initially failing) tests become the WS-A specification.

## Decision Drivers

* De-risk WS-A: an oracle must exist before the optimizer is touched.
* Reuse Ontop as spec/oracle (ADR-0004) without transliterating its Java.
* Hold the ADR-0012 differential oracle and keep the existing 181 tests green.

## Considered Options

* **Port Ontop's IQ-optimizer JUnit tests to Rust first, then drive WS-A/WS-B against them** — chosen.
* **Implement the optimizer passes first, test ad hoc** — rejected: no trustworthy oracle for the OPTIONAL/NULL surface; ad-hoc tests miss the bag/3VL edge cases that are the whole difficulty.
* **Mechanically transliterate Ontop's Java JUnit/IQ harness** — rejected: the harness is entangled with Ontop's Java IQ model; re-express the *assertions* against `iq.rs`, not the scaffolding (ADR-0004).

## Decision Outcome

Chosen: **port four Ontop optimizer test classes to Rust** oracle/equivalence tests against `iq.rs` + the `=_bag` differential; the failing subset is the WS-A backlog.

### The four test classes to port

Under `~/source/ontop/core/optimization/src/test/java/`:

* `it/unibz/inf/ontop/iq/executor/LeftJoinOptimizationTest.java` — Q5-relevant (OPTIONAL / left-join elimination).
* `it/unibz/inf/ontop/iq/executor/RedundantSelfJoinTest.java` — Q5-relevant (self-join collapse via unique constraints).
* `it/unibz/inf/ontop/iq/optimizer/SelfJoinSameTermsTest.java` — shared self-join machinery.
* `it/unibz/inf/ontop/iq/executor/RedundantJoinFKTest.java` — maps to the existing `crates/sf-sparql/src/cascade/joinelim.rs`.

### Mapping onto semantic-fabric

* JUnit IQ assertions (input tree → expected optimized tree) → Rust tests over `iq.rs`, checked structurally and via the `=_bag` NoREC-style differential (unoptimized vs optimized IQ) of ADR-0012.
* WS-A spec classes these tests target (for the next wave): `LeftJoinIQOptimizer` (+ `impl/lj/`), `SelfJoinUCIQOptimizer`, `SelfJoinSameTermIQOptimizer` / `AbstractSelfJoinSimplifier`, `RedundantJoinFKOptimizer`, and `LeftJoinNormalizerImpl` (null-safe LJ).

### Execution

implement (port tests) → verify (`cargo test --workspace` + `=_bag` differential + W3C RDB2RDF floor + `cargo clippy --all-targets -D warnings` + `cargo fmt --check`) → adversarial review → conditional fix. Tests are re-expressed against `iq.rs`, never transliterated.

### Consequences

* Good, because the ported tests are the oracle that de-risks WS-A/WS-B; the failing subset becomes the WS-A spec with no ambiguity.
* Good, because `RedundantJoinFKTest` immediately exercises the existing `joinelim.rs`, catching any regression there for free.
* Bad, because Wave 1 ships tests, not user-visible features — visible parity progress waits for WS-A.
* Neutral, because some Ontop assertions will not map 1:1 onto `iq.rs` (a different IQ model) and are re-expressed by intent, not by structure.

### Confirmation

* The ported tests compile and run in `cargo test --workspace`: **GREEN** where semantic-fabric already conforms (e.g. FK/PK elimination vs `RedundantJoinFKTest`), **RED** where the optimization is absent (the WS-A backlog — self-/self-left-join elimination, the Q5 case).
* The existing **181 tests stay green**; `clippy -D warnings` and `fmt --check` stay clean (CI gate).
* WS-A is considered started only once the RED set is enumerated and tracked by `horizon-tracker`.

## More Information

* **Umbrella:** ADR-0021. **Rewriter / cascade:** ADR-0007. **Test strategy / differential oracle:** ADR-0012. **Conformance + bench gate:** ADR-0005.
* **Ontop source:** `~/source/ontop` (tag `ontop-5.5.0`). Optimizer spec classes under `core/optimization/src/main/java/it/unibz/inf/ontop/iq/optimizer/`; tests under `core/optimization/src/test/java/`.
* **Q5 root cause (WS-A target):** `HANDOVER-2026-06-28-ontop-parity.md §Findings 1` — redundant self-left-join + unconditional null-safe ON (`leftjoin.rs:58`, `emit.rs:472–475`); no self-(left-)join elimination (`joinelim.rs:64–66` refuses OPTIONAL sides).
* **Theory anchor:** Xiao/Kontchakov et al., *Efficient Handling of SPARQL OPTIONAL for OBDA* (ISWC 2018).

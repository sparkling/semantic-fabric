# SOTA Rust SHACL Engine — selection for the M⋈T gate

**Research key:** `shacl-engine-selection`
**Date:** 2026-06-27 (round-2 deep-research)
**Scope:** the Rust SHACL runner for ADR-0005's `M ⋈ T` mapping-output gate (four SHACL **Core** meta-shapes from the upstream modelling project's mapping-conformance requirements). Must be Rust-native, in-process, no-JVM, oxrdf-aligned, SHACL-Core-only (no SHACL-SPARQL).
**Decision recorded in:** ADR-0005 (decision update 2026-06-27).

> **Version correction (validated on crates.io 2026-06-27).** The rudof workspace reorg has **shipped**: the lean validation crate is now **`shacl` 0.3.4** (renamed from `shacl_validation`; graph via `rudof_rdf`, oxrdf-backed), which `rudof_lib` 0.3.4 itself depends on (`shacl ^0.3.4` + `oxrdf ^0.3.0`). **Pin `shacl = "0.3"` + `oxrdf = "0.3"`.** The `shacl_validation` (last 0.2.12) + `srdf` (0.1.x) crate names referenced below are **pre-reorg and superseded** — the analysis/recommendation stands, only the crate name + version changed.

## Bottom line

**Adopt `rudof` — the lean `shacl` crate (the reorged `shacl_validation`), in `ShaclValidationMode::Native`.** Pin `shacl = "0.3"` (currently 0.3.4) + `oxrdf = "0.3"`. It is the only candidate literally built on our stack: `srdf::srdf_graph` is "the SRDF traits **using OxRDF**" on `oxrdf ^0.3` + `oxttl ^0.2` + `oxigraph ^0.5.2` — **zero RDF-bridge work**, no JVM, single binary, clean `ValidationReport::conforms() -> bool` gate primitive. Use the lean crates, **not** the `rudof_lib` facade (it pulls ShEx/DCTAP/SPARQL-service/MCP).

## Contradiction resolved

| Prior claim | Verdict | Why |
|---|---|---|
| "rudof = primary candidate" | **Correct** | oxrdf-native, no-JVM, full Core in source, peer-reviewed lineage |
| "oxirs-shacl = production, 27/27 W3C Core" | **Misleading** | "27/27 W3C" is **not** the official W3C suite (**121 tests**); oxirs is **absent** from the W3C implementation report; AI-generated-claim markers (0 issues across 26 crates; "43,500 tests/100%"); validates `oxirs-core` types (+ `scirs2-*` platform), **not** oxrdf |
| "grafeo = 28/28 + SHACL-SPARQL" | **Real, wrong architecture** | healthy project (678★) but a **separate graph DB** — data must live in GrafeoDB = a second RDF stack |

Decisive framing: the 8 components we need (`NodeShape`, `property`, `class`, `datatype`, `nodeKind`, `minCount`/`maxCount`, `in`, `hasValue`) are the **most trivial, universally-supported** subset of SHACL Core — all three support them, so coverage is **not** the differentiator; **integration + credibility + binary footprint** are. And **none of the three has an official W3C implementation-report submission** — every "27/27"/"28/28" number is self-reported.

## Candidate facts

- **rudof** — `rudof_lib` 0.3.4 (2026-06-16); `shacl_validation` ~0.2.13; WESO group (Labra Gayo, U. Oviedo); MIT OR Apache-2.0; 114★, 164 releases, active. CI runs the actual W3C `data-shapes-test-suite/tests/core/` files through its NativeEngine; the complete Core constraint set is present in `shacl/src/validator/constraints/core/`. Perf (CEUR/ISWC-2024 LUBM): rudof **7.90 ms** vs RDF4J 1.64 ms vs Jena 60.36 ms vs TopQuadrant 85.74 ms (the JVM engines are all disqualified; rudof is the only Rust candidate with a published SHACL benchmark). Predecessor **shaclex** (Scala) is in the official W3C report at **98/121 (81%)**. **Risks:** thin docs (~12%); `rudof_lib` 0.3.4 docs.rs build failed (last good 0.2.6); published-vs-master module-path skew (published 0.2.x uses flat paths `shacl_validation::shacl_processor`; master unifies into a `shacl` crate) → **pin one version, confirm import paths against its rustdoc**.
- **oxirs-shacl** — 0.3.1 (2026-06-06), Apache-2.0, 71★; cool-japan/OxiRS; not recommended (see table). Validates `oxirs-core` (the "zero-dependency" claim is false — ~50+ deps incl. `scirs2-*`, `tokio`, `reqwest`, `tower`, `wasmi`).
- **grafeo-engine** — 0.5.42 (2026-05-04), Apache-2.0, 678★; genuine, healthy pure-Rust graph DB; wrong fit (separate store).

## The spike (now a standard unit test, not a gate)

Lean deps: `shacl = "0.3"` (0.3.4), `oxrdf = "0.3"`, `oxttl = "0.2"` (graph via `rudof_rdf`, oxrdf-backed). Flow: materialized/virtualized output → `GraphValidation` (oxrdf-backed, no endpoint) → compile the 4 meta-shapes → `validate(&shapes, &ShaclValidationMode::Native)` → `report.conforms()`. **Acceptance:** a fixture violating each of the 8 components returns `conforms() == false` with one `ValidationResult` each. `rudof_lib`'s own integration test `validate_shacl_tests.rs` is the fastest "spike B" to green, then slim to the lean crates.

## Evidence grades
- rudof srdf = oxrdf-0.3/oxttl-0.2-native, no endpoint — **High** (docs.rs/srdf deps).
- rudof implements all 8 + full Core; Native = no-SPARQL — **High** (read from source).
- LUBM benchmark numbers — **High** (CEUR Vol-3828 paper32).
- W3C suite = 121 tests; none of the three submitted; shaclex 81% — **High** (W3C report).
- oxirs "27/27" ≠ W3C suite; validates oxirs-core+scirs2 — **High** (W3C report + docs.rs deps).
- oxirs AI-generated-claim pattern — **Med** (README + repo metrics).

## Sources
- https://github.com/rudof-project/rudof · https://crates.io/crates/rudof · https://docs.rs/shacl_validation/ · https://docs.rs/srdf/latest/srdf/ · https://docs.rs/crate/rudof_lib/latest
- https://ceur-ws.org/Vol-3828/paper32.pdf (rudof CEUR/ISWC-2024 + LUBM SHACL benchmark)
- https://w3c.github.io/data-shapes/data-shapes-test-suite/ (official SHACL implementation report — 121 tests)
- https://github.com/cool-japan/oxirs · https://docs.rs/crate/oxirs-shacl/latest · https://docs.rs/oxirs-core/latest/oxirs_core/
- https://github.com/GrafeoDB/grafeo · https://grafeo.dev/ · https://lib.rs/crates/grafeo-engine
- https://www.w3.org/TR/shacl/ (SHACL Core — 28 constraint components)

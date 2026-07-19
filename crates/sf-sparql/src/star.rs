//! ADR-0032 D3 — RDF-star query rewrite: a `GraphPattern → GraphPattern`
//! pre-pass that desugars quoted-triple patterns onto the native-reification
//! encoding Wave 1 now emits (`sf-mapping`'s `r2rml/star.rs`), applied once at
//! the top of both `translate_tree` and `translate_inner_flat` (`lib.rs`) —
//! mirrors the DESCRIBE→CBD rewrite already living there (a recursive algebra
//! rebuild minting `__sf_`-prefixed synthetic variables), so
//! `build.rs`/`iq/*.rs`/`unfold.rs`/`cascade/`/`emit.rs` never see a
//! `TermPattern::Triple` at all (R1).
//!
//! This supersedes ADR-0031's rules R2/R5 in place (ADR-0032 D3). Ground
//! truth (pinned `spargebra 0.4.6+sparql-12`, unchanged from ADR-0031):
//! bare `<<s p o>>` is parser-desugared to `_:b rdf:reifies <<( s p o )>>` +
//! `_:b` at the original position; parenthesized `<<( s p o )>>` yields
//! `TermPattern::Triple` in place.
//!
//! Rewrite rules per triple pattern (ADR-0032 D3, order matters):
//! (R1) a triple pattern whose own SUBJECT is a triple term — the outer
//! pattern's, OR (recursively) any quoted triple reached through an R4
//! object-chain — can never match (SPARQL 1.2 §18.1.3): rewritten to a
//! **statically empty** group, never an error, never a match. Checked before
//! R2 inspects the predicate, so `X rdf:reifies TT` with X itself a triple
//! term is equally empty. (R2) `X rdf:reifies TT` (all bare/explicit-reifier/
//! annotation sugar desugars here — parser-verified): **no elision**. `X`
//! stays untouched; the wrapper triple is KEPT as `X rdf:reifies ?pf` (fresh
//! var) with the 4 basic-encoding patterns appended on `?pf` — matches only
//! genuinely reified statements (a v1 unsoundness: bare-sugar over-matching
//! unreified object-position triple terms, fixed). (R3) else object-is-Triple
//! → fresh `?pf` + 4 patterns, symmetric with R2's minting. (R4) a quoted
//! triple's own OBJECT being ANOTHER quoted triple recurses bottom-up,
//! arbitrary depth (mirrors `sf-mapping`'s recursive `quote_shape`): the
//! inner quote mints its own `?pf` first, spliced in as the outer's
//! `propositionFormObject`. (R5) recursion covers every `GraphPattern`
//! container, `Expression::Exists` bodies, and `GraphPattern::Path` endpoints
//! (a fresh var + the 4 patterns joined alongside the path node) — path
//! endpoints keep v1's exact shape and inherit the same pre-existing,
//! unrelated boundary (D6, see `differential_star.rs`'s locked test). (R6,
//! Wave 2b) `GraphPattern::Values` decomposes any column carrying a
//! `GroundTerm::Triple` cell into fresh component-var columns
//! (`decompose::rewrite_values`) — see [`StarEnv`]. (R7, Wave 2b) a CONSTRUCT
//! template is pre-substituted, not guarded ([`substitute_construct_template`]):
//! every env-composed variable it references is replaced with an explicit
//! `TermPattern::Triple` over its component vars, so `exec_core::instantiate`
//! (ADR-0032 D2) never needs to know about [`StarEnv`] at all.
//!
//! **Wave 2b additions (ADR-0032 D3 item 2-4):** a whole-query
//! variable → composed-info environment ([`StarEnv`]) is threaded alongside
//! the fresh-variable counter, populated by three sites — R2's NEW
//! reifies-bare-variable case (`walk::rewrite_triple`), R6's VALUES
//! decomposition (`decompose::rewrite_values`), and a
//! `BIND(TRIPLE(e1,e2,e3) AS ?t)` target (`decompose::rewrite_extend`) — and
//! consumed by the five triple-term functions (`expr::rewrite_function_call`),
//! composed-aware `=`/`sameTerm` (`expr::rewrite_equality`), CONSTRUCT
//! template pre-substitution ([`substitute_construct_template`]), and the
//! projection seam ([`apply_composed_bindings`], called from `lib.rs` after a
//! `Plan`'s branches are otherwise finalized) that realizes a composed
//! variable as a native `Term::Triple` at reconstruction. A `Union`'s two
//! arms are checked for composed-ness agreement on any variable they both
//! mention (`top_level::rewrite_union`) — the uniform-composed-ness law this
//! whole mechanism depends on (a SPARQL variable cannot be "sometimes a
//! triple term"
//! depending on which UNION arm produced it).
//!
//! **Module map** (ledger F3 split — this file grew past the project's
//! 500-line guideline; split along the file's natural seams into a `star/`
//! tree, the same `r2rml.rs`/`r2rml/*.rs` shape `sf-mapping` already uses):
//! [`env`] — [`StarEnv`]/[`ComposedInfo`] and every realization consumer
//! (the projection seam, CONSTRUCT template pre-substitution, the
//! projection-name-list helpers); [`walk`] — the core recursive
//! `GraphPattern`/BGP/property-path walker and basic-encoding emission;
//! [`decompose`] — the two composed-variable mint sites outside a plain BGP
//! triple pattern (VALUES columns, `BIND(TRIPLE(...))`); [`expr`] — the
//! `Expression` tree rewrite (the five triple-term functions, composed-aware
//! `=`/`sameTerm`, the error-marker/boolean-literal leaves); [`top_level`] —
//! the whole-query entry point and the top-level UNION/VALUES
//! composed-ness-mismatch relaxation; `collect_vars` — the variable-collection
//! helper `top_level`'s uniform-composed-ness check uses; `util` — shared
//! vocabulary constants and fresh-variable minting. Every child is a private
//! submodule (not `pub mod`): this file re-exports exactly the items that
//! were `pub`/`pub(crate)` before the split, so the crate's observable
//! surface is unchanged.

mod collect_vars;
mod decompose;
mod env;
mod expr;
mod top_level;
mod util;
mod walk;

pub(crate) use env::composed_term_def;
pub use env::{
    all_component_var_names, apply_composed_bindings, expand_projection_for_cascade,
    substitute_construct_template, ComposedInfo, StarEnv,
};
pub use top_level::rewrite_query;

#[cfg(test)]
mod tests;

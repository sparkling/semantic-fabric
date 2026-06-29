//! Ontop-parity oracle port — batch 7 of 8 (ADR-0022).
//!
//! INTENTIONALLY EMPTY. The batch partition assigns each batch the sorted
//! `*Test.java` slice `[N*5, N*5+5)` taken over the combined listing of
//! `~/source/ontop/core/optimization/src/test/java/it/unibz/inf/ontop/iq/executor/`
//! and `.../iq/optimizer/`. That combined listing contains 33 files (sorted
//! indices 0..=32), not the estimated ~40. Batch 7's slice is therefore
//! `[35, 40)`, which lies entirely beyond the last available index (32) — it
//! resolves to **zero classes**. Batch 6's slice `[30, 35)` already covers the
//! tail (`UriTemplateTest`, `ValuesNodeOptimizationTest`, `ValuesNodeTest` at
//! indices 30/31/32), so there is no overflow for batch 7 to pick up.
//!
//! No oracle tests are written here: with no assigned classes there are no
//! scenarios to port, and fabricating tests for other batches' classes would
//! both violate the slice assignment and risk cross-batch conflicts. This file
//! exists only so the batch's expected test target compiles and runs cleanly
//! (0 tests). See `crates/sf-sparql/src/cascade/ws_g.rs` for the port pattern
//! used by the non-empty batches.

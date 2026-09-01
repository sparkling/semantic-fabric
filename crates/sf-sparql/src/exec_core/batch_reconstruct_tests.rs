//! `reconstruct_batch` must reproduce the exact per-row sequential
//! [`reconstruct`] output, in the exact original row order, whether the
//! batch is small enough to stay sequential, large enough to fan out to
//! rayon (`TERM_GEN_MIN_PARALLEL_ROWS`), or large but `parallel_allowed` is
//! `false` (ledger F8's dump-path gate) — the "safest is strict order
//! preservation via indexed chunks" design note on `reconstruct_batch`.
use sf_core::ir::{TermMap, TermSpec};
use sf_core::Term;
use sf_sql::RawTuple;

use crate::iq::{Branch, ColRef, TermDef};

use super::batch::{
    reconstruct_batch, TERM_GEN_BATCH_SIZE, TERM_GEN_MIN_CHUNK_ROWS, TERM_GEN_MIN_PARALLEL_ROWS,
};
use super::row::{build_col_index, intern_bindings, reconstruct, RawRow};

/// A branch with ONE bound variable `?v`, read from column `"val"` of scan
/// alias 0 as a plain literal — real per-row reconstruction work (unlike
/// `TermDef::Const`, which never touches the row).
fn branch_with_val_binding() -> Branch {
    let mut b = Branch::empty();
    b.bindings.insert(
        "v".to_owned(),
        TermDef::Derived {
            term_map: TermMap::Column("val".into(), TermSpec::plain_literal()),
            alias: 0,
        },
    );
    b
}

/// `n` raw rows, column `"val"` set to the row's index as text — each row
/// must reconstruct to a distinct term, so a reordering or drop is visible.
fn raw_rows(n: usize) -> Vec<RawTuple> {
    (0..n)
        .map(|i| RawTuple {
            values: vec![Some(i.to_string())],
            codes: vec![None],
        })
        .collect()
}

#[test]
fn batched_reconstruction_matches_sequential_reference_in_order() {
    let branch = branch_with_val_binding();
    let schema = vec![ColRef::new(0, "val")];
    let col_index = build_col_index(&schema);
    let interned = intern_bindings(&branch);

    // Spans: below TERM_GEN_MIN_PARALLEL_ROWS (the whole-batch sequential
    // path), astride it (the smallest dispatch that goes parallel at all),
    // exactly one full steady-state batch, and several batches' worth — what
    // `run_branches` actually issues as consecutive `reconstruct_batch` calls
    // for one long branch stream (mirrored below via
    // `.chunks(TERM_GEN_BATCH_SIZE)`).
    for n in [
        1,
        50,
        TERM_GEN_MIN_CHUNK_ROWS,
        TERM_GEN_MIN_PARALLEL_ROWS - 1,
        TERM_GEN_MIN_PARALLEL_ROWS,
        TERM_GEN_BATCH_SIZE,
        2 * TERM_GEN_BATCH_SIZE + 137,
    ] {
        let rows = raw_rows(n);
        let sequential: Vec<Option<Term>> = rows
            .iter()
            .map(|t| {
                let raw = RawRow {
                    values: &t.values,
                    codes: &t.codes,
                    index: &col_index,
                };
                reconstruct(&interned, &raw)
                    .expect("reference reconstruct")
                    .get("v")
                    .cloned()
            })
            .collect();

        // Both gate states must match the reference — `parallel_allowed`
        // only decides WHETHER a big batch may fan out, never the result.
        for parallel_allowed in [true, false] {
            let mut batched: Vec<Option<Term>> = Vec::with_capacity(n);
            for chunk in rows.chunks(TERM_GEN_BATCH_SIZE) {
                for bindings in reconstruct_batch(&interned, chunk, &col_index, parallel_allowed) {
                    batched.push(bindings.expect("batch reconstruct").get("v").cloned());
                }
            }
            assert_eq!(
                sequential, batched,
                "reconstruct_batch must match sequential reconstruct, in order, \
                     at n={n}, parallel_allowed={parallel_allowed}"
            );
        }
    }
}

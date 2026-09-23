//! `reconstruct_batch` must reproduce the exact per-row sequential
//! [`reconstruct`] output, in the exact original row order, whether the
//! batch is small enough to stay sequential, large enough to fan out to
//! rayon (`TERM_GEN_MIN_PARALLEL_ROWS`), or large but `parallel_allowed` is
//! `false` (ledger F8's dump-path gate) — the "safest is strict order
//! preservation via indexed chunks" design note on `reconstruct_batch`.
use sf_core::ir::{TermMap, TermSpec};
use sf_core::term_work::TermWork;
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
                for bindings in reconstruct_batch(
                    &interned,
                    chunk,
                    &col_index,
                    parallel_allowed,
                    TermWork::uncontrolled(),
                ) {
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

// --- request governance over per-row reconstruction --------------------------

use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

fn source_budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
}

/// The SourceWork one batch's reconstruction actually costs, measured against a
/// deliberately unbounded budget — the reference every boundary below is
/// derived from, rather than a hand-computed figure that could drift from the
/// charging code.
fn measured_cost(rows: &[RawTuple], parallel_allowed: bool) -> u64 {
    let branch = branch_with_val_binding();
    let schema = vec![ColRef::new(0, "val")];
    let col_index = build_col_index(&schema);
    let interned = intern_bindings(&branch);
    let budget = source_budget(u64::MAX);
    for row in reconstruct_batch(
        &interned,
        rows,
        &col_index,
        parallel_allowed,
        TermWork::new(Some(&budget)),
    ) {
        row.expect("an unbounded budget reconstructs every row");
    }
    budget.consumed(QueryCharge::SourceWork)
}

/// Reconstructing under exactly the measured cost succeeds; one unit less
/// refuses. This is the property that makes the charge real: if reconstruction
/// were uncharged, the N-1 budget would still pass.
#[test]
fn reconstruction_admits_its_exact_cost_and_refuses_one_unit_less() {
    let branch = branch_with_val_binding();
    let schema = vec![ColRef::new(0, "val")];
    let col_index = build_col_index(&schema);
    let interned = intern_bindings(&branch);
    let rows = raw_rows(64);
    let exact_cost = measured_cost(&rows, false);
    assert!(
        exact_cost > 0,
        "per-row reconstruction must charge the request, not run free"
    );

    let exact = source_budget(exact_cost);
    for row in reconstruct_batch(
        &interned,
        &rows,
        &col_index,
        false,
        TermWork::new(Some(&exact)),
    ) {
        row.expect("the exact measured cost is admitted");
    }
    assert_eq!(exact.consumed(QueryCharge::SourceWork), exact_cost);

    let short = source_budget(exact_cost - 1);
    let refusals = reconstruct_batch(
        &interned,
        &rows,
        &col_index,
        false,
        TermWork::new(Some(&short)),
    );
    let last = refusals.last().expect("a batch of rows returns results");
    assert!(
        matches!(
            last,
            Err(crate::Error::QueryControl(
                QueryControlError::SourceWorkExceeded
            ))
        ),
        "one unit below the measured cost must refuse, got {last:?}"
    );
}

/// `parallel_allowed` is a throughput decision, never an accounting one: a
/// batch large enough to fan out to rayon must charge exactly what the
/// sequential pass charges, and produce the same terms in the same order.
#[test]
fn parallel_and_sequential_reconstruction_charge_and_produce_identically() {
    let rows = raw_rows(TERM_GEN_MIN_PARALLEL_ROWS + 371);
    let sequential_cost = measured_cost(&rows, false);
    let parallel_cost = measured_cost(&rows, true);
    assert_eq!(
        sequential_cost, parallel_cost,
        "a shared control must accrue the same total whichever way the batch ran"
    );

    let branch = branch_with_val_binding();
    let schema = vec![ColRef::new(0, "val")];
    let col_index = build_col_index(&schema);
    let interned = intern_bindings(&branch);
    let terms = |parallel_allowed: bool| -> Vec<Option<Term>> {
        let budget = source_budget(u64::MAX);
        reconstruct_batch(
            &interned,
            &rows,
            &col_index,
            parallel_allowed,
            TermWork::new(Some(&budget)),
        )
        .into_iter()
        .map(|row| row.expect("unbounded budget").get("v").cloned())
        .collect()
    };
    assert_eq!(
        terms(false),
        terms(true),
        "controlled reconstruction must preserve row order and values under dispatch"
    );
}

/// A request that becomes terminal mid-batch stops inside that batch: the rows
/// after the stop report the sticky cause rather than being reconstructed. This
/// is the gap the batch-boundary checkpoint alone left open.
#[test]
fn a_request_cancelled_mid_batch_stops_within_that_batch() {
    let branch = branch_with_val_binding();
    let schema = vec![ColRef::new(0, "val")];
    let col_index = build_col_index(&schema);
    let interned = intern_bindings(&branch);
    let rows = raw_rows(512);

    // Budget exactly one row's worth of reconstruction, so the stop lands
    // inside the batch rather than at its boundary.
    let one_row_cost = measured_cost(&raw_rows(1), false);
    let budget = source_budget(one_row_cost);
    let results = reconstruct_batch(
        &interned,
        &rows,
        &col_index,
        false,
        TermWork::new(Some(&budget)),
    );
    assert_eq!(
        results.len(),
        rows.len(),
        "every row still reports an outcome"
    );
    assert!(
        results[0].is_ok(),
        "the affordable first row is reconstructed"
    );
    assert!(
        results[1..].iter().all(|row| matches!(
            row,
            Err(crate::Error::QueryControl(
                QueryControlError::SourceWorkExceeded
            ))
        )),
        "every row past the budget must observe the sticky terminal cause"
    );
    assert!(
        budget.consumed(QueryCharge::SourceWork) <= one_row_cost,
        "a refused row must not commit further work"
    );
}

/// Cancellation (not budget exhaustion) is observed per row too, and consumes
/// no budget: the checkpoint, not the charge, is what stops the batch.
#[test]
fn cancellation_stops_reconstruction_without_consuming_budget() {
    let branch = branch_with_val_binding();
    let schema = vec![ColRef::new(0, "val")];
    let col_index = build_col_index(&schema);
    let interned = intern_bindings(&branch);
    let budget = source_budget(u64::MAX);
    budget.terminate(QueryControlError::Cancelled);

    let results = reconstruct_batch(
        &interned,
        &raw_rows(32),
        &col_index,
        false,
        TermWork::new(Some(&budget)),
    );
    assert!(
        results.iter().all(|row| matches!(
            row,
            Err(crate::Error::QueryControl(QueryControlError::Cancelled))
        )),
        "a cancelled request reconstructs no row"
    );
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
}

/// The uncontrolled identity preserves the previous behavior exactly: the
/// existing raw/conformance entry points keep reconstructing without a budget.
#[test]
fn the_uncontrolled_identity_reconstructs_exactly_as_before() {
    let branch = branch_with_val_binding();
    let schema = vec![ColRef::new(0, "val")];
    let col_index = build_col_index(&schema);
    let interned = intern_bindings(&branch);
    let rows = raw_rows(TERM_GEN_MIN_PARALLEL_ROWS + 5);
    for parallel_allowed in [true, false] {
        for row in reconstruct_batch(
            &interned,
            &rows,
            &col_index,
            parallel_allowed,
            TermWork::uncontrolled(),
        ) {
            row.expect("uncontrolled reconstruction never refuses");
        }
    }
}

/// §10 canonicalization can make a literal WIDER than its raw column value, so
/// the charge must cover the literal actually built, not just the bytes read.
/// `sf_core::datatype`'s own tests pin the growth this guards: `"0"` becomes
/// `"false"` and `"100"` becomes `"1.0E2"`. Without
/// `natural_lexical_growth_allowance`, a budget set to exactly
/// `raw-value-width` worth of reconstruction would admit a strictly wider
/// allocation than it paid for.
#[test]
fn canonicalization_widening_is_charged_for_natural_typed_columns() {
    use sf_core::datatype::XsdTypeCode;

    // (code, raw column value, its canonical form) — each canonical form is
    // strictly wider than the raw value it is built from.
    for (code, raw, canonical) in [
        (XsdTypeCode::Boolean, "0", "false"),
        (XsdTypeCode::Boolean, "1", "true"),
        (XsdTypeCode::Double, "100", "1.0E2"),
        // The largest measured (canonical - raw) growth for a double: a
        // one-byte raw value expanding to the mandatory mantissa/exponent
        // form. A Double allowance below this growth fails here.
        (XsdTypeCode::Double, "0", "0.0E0"),
        // `decimal::write_canonical` supplies the leading `0` a leading-dot
        // form omits, so this grows by exactly one byte.
        (XsdTypeCode::Decimal, ".5", "0.5"),
        (XsdTypeCode::Decimal, "-.5", "-0.5"),
    ] {
        assert!(
            canonical.len() > raw.len(),
            "fixture must actually widen: {raw:?} -> {canonical:?}"
        );

        let mut branch = Branch::empty();
        branch.bindings.insert(
            "v".to_owned(),
            TermDef::Derived {
                term_map: TermMap::Column("val".into(), TermSpec::plain_literal()),
                alias: 0,
            },
        );
        let schema = vec![ColRef::new(0, "val")];
        let col_index = build_col_index(&schema);
        let interned = intern_bindings(&branch);
        let rows = vec![RawTuple {
            values: vec![Some(raw.to_owned())],
            codes: vec![Some(code)],
        }];

        // The value really is reconstructed through the natural-type path.
        let unbounded = source_budget(u64::MAX);
        let built = reconstruct_batch(
            &interned,
            &rows,
            &col_index,
            false,
            TermWork::new(Some(&unbounded)),
        );
        let term = built[0]
            .as_ref()
            .expect("unbounded budget reconstructs")
            .get("v")
            .cloned()
            .expect("the natural-typed column binds");
        let Term::Literal(literal) = &term else {
            panic!("expected a literal, got {term:?}");
        };
        assert_eq!(
            literal.value(),
            canonical,
            "fixture must exercise the canonicalizing path"
        );

        // The charge must cover the canonical width, not just the raw width.
        let charged = unbounded.consumed(QueryCharge::SourceWork);
        assert!(
            charged >= canonical.len() as u64,
            "reconstruction charged {charged} for a {raw} -> {canonical:?} ({} byte) \
             literal; a charge below the built width undercharges the allocation",
            canonical.len()
        );

        // Stronger: isolate the VALUE-dependent part of the charge from the
        // fixed per-row/per-binding overhead, by re-measuring the same shape
        // with an empty value. What is left must still cover the growth, so a
        // per-code allowance that is present but too small (or missing) fails
        // here even when the raw value's own length happens to absorb it — the
        // Decimal case (".5" -> "0.5") is exactly that: charging value.len()
        // alone already exceeds the 3-byte output, so only this comparison
        // proves the +1 allowance is really there.
        let baseline = {
            let budget = source_budget(u64::MAX);
            let empty = vec![RawTuple {
                values: vec![Some(String::new())],
                codes: vec![Some(XsdTypeCode::String)],
            }];
            let _ = reconstruct_batch(
                &interned,
                &empty,
                &col_index,
                false,
                TermWork::new(Some(&budget)),
            );
            budget.consumed(QueryCharge::SourceWork)
        };
        let value_dependent = charged.saturating_sub(baseline);
        // Compared against the literal's OWN canonical width, which is a fact
        // about `canonical_lexical` rather than about the allowance table, so
        // an allowance that is present but too small cannot satisfy it by
        // moving the expectation with itself.
        assert!(
            value_dependent >= canonical.len() as u64,
            "value-dependent charge for {code:?} {raw:?} was {value_dependent}, which does \
             not cover the {} byte canonical form {canonical:?} it builds; the per-code \
             growth allowance is missing or too small",
            canonical.len()
        );
    }
}

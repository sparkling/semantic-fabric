use super::*;

/// Ontop `ValuesNodeOptimization::test4SliceUnionValuesValues`: `Slice` over a
/// `Union` of two bare `VALUES` blocks — covered FOR FREE by composing the two
/// existing rules above (no new production code): each bare `VALUES` arm is
/// already an `IqNode::Values` (no `Construction` wrapper needed, unlike `BIND`),
/// so `fold_constant_union`'s "absorb an already-Values arm" case (added for
/// the left-associative 3-arm fix) folds the whole `Union` to one `Values`, which
/// `normalize_slice` then truncates in place — verified here as its own named
/// scenario, not assumed from the two rules' own tests.
#[test]
fn slice_over_union_of_values_values_folds_and_truncates() {
    let n = norm("SELECT ?x WHERE { { VALUES ?x { 1 2 } } UNION { VALUES ?x { 3 4 } } } LIMIT 3");
    let IqNode::Construction { child, .. } = &n else {
        panic!("expected the identity-projection Construction wrapper: {n:?}")
    };
    let IqNode::Values { rows, .. } = child.as_ref() else {
        panic!("expected the Union to fold to ONE Values leaf, then truncate: {child:?}")
    };
    assert_eq!(
        rows.iter().map(|r| row_int(r)).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "as-written order across both arms, truncated to LIMIT 3: {rows:?}"
    );
}

/// Ontop `ValuesNodeOptimization::test5SliceUnionValuesNonValues`: a `Slice`
/// window that falls ENTIRELY within a leading `Values` arm's known row count
/// drops the trailing DATA (real-pattern) arm outright — no scan of it survives.
#[test]
fn slice_over_union_drops_unreachable_data_arm() {
    let n = norm(
        "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" } } \
         UNION { ?s <http://ex/name> ?n } } LIMIT 2",
    );
    let IqNode::Values { vars, rows } = &n else {
        panic!("expected full resolution to a bare Values leaf (no Slice/Union survives): {n:?}")
    };
    assert_eq!(vars.len(), 1);
    assert_eq!(
        rows.iter().map(|r| row_str(r)).collect::<Vec<_>>(),
        vec!["a", "b"],
        "the data arm must not appear at all: {rows:?}"
    );
}

/// The OFFSET half: a leading `Values` arm big enough to cover BOTH the offset
/// skip and the whole limit drops the data arm too, keeping only the surviving
/// window from the `Values` arm.
#[test]
fn slice_over_union_offset_fully_satisfied_by_values_drops_data_arm() {
    let n = norm(
        "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" \"c\" } } \
         UNION { ?s <http://ex/name> ?n } } LIMIT 2 OFFSET 1",
    );
    let IqNode::Values { rows, .. } = &n else {
        panic!("expected full resolution to a bare Values leaf: {n:?}")
    };
    assert_eq!(
        rows.iter().map(|r| row_str(r)).collect::<Vec<_>>(),
        vec!["b", "c"],
        "OFFSET 1 skips \"a\"; the data arm is still unreachable: {rows:?}"
    );
}

/// Ontop `ValuesNodeOptimization::test6/7SliceUnionValuesNonValues` (residual
/// limit): the `Values` arm doesn't cover the whole window, so the data arm
/// survives — but under a Slice reset to `offset=0` (the survivors already
/// account for everything before them in as-written order) with the ORIGINAL
/// limit (not reduced: a plain Slice-over-Union already reads the reconstructed
/// sequence from position 0, so the survivor rows themselves count toward it).
#[test]
fn slice_over_union_residual_limit_keeps_the_data_arm() {
    let n = norm(
        "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" \"c\" } } \
         UNION { ?s <http://ex/name> ?n } } LIMIT 5 OFFSET 1",
    );
    let IqNode::Slice {
        child,
        offset,
        limit,
    } = &n
    else {
        panic!("expected a residual Slice to survive (the data arm is still needed): {n:?}")
    };
    assert_eq!(
        *offset, 0,
        "the offset skip is already baked into the survivors"
    );
    assert_eq!(*limit, Some(5), "the ORIGINAL limit, not reduced");
    let IqNode::Union { children, .. } = child.as_ref() else {
        panic!("expected the survivor Values arm + the data arm: {child:?}")
    };
    assert_eq!(
        children.len(),
        2,
        "one survivor arm + the data arm: {children:?}"
    );
    let IqNode::Values { rows, .. } = &children[0] else {
        panic!("expected the FIRST child to be the bare survivor Values arm: {children:?}")
    };
    assert_eq!(
        rows.iter().map(|r| row_str(r)).collect::<Vec<_>>(),
        vec!["b", "c"],
        "OFFSET 1 dropped \"a\"; \"b\"/\"c\" survive as explicit rows: {rows:?}"
    );
    assert!(
        !matches!(children[1], IqNode::Values { .. }),
        "the second child must still be the untouched data arm: {:?}",
        children[1]
    );
}

/// Adversarial-review-caught regression: OFFSET exceeds the known (`Values`) arm's
/// ENTIRE row count while a data arm still remains -- the whole `Values` arm is
/// dropped (all of it falls before the window), but the LEFTOVER skip must carry
/// forward onto the data arm itself, not vanish. A first draft hardcoded the
/// residual `Slice`'s offset to 0 unconditionally, silently discarding that
/// leftover and leaking extra rows the true OFFSET should have skipped.
#[test]
fn slice_over_union_offset_exceeding_values_carries_forward_onto_data_arm() {
    let n = norm(
        "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" \"c\" } } \
         UNION { ?s <http://ex/name> ?n } } LIMIT 5 OFFSET 4",
    );
    let IqNode::Slice {
        child,
        offset,
        limit,
    } = &n
    else {
        panic!("expected a residual Slice to survive (the data arm is still needed): {n:?}")
    };
    assert_eq!(
        *offset, 1,
        "3 of the 4 requested skips are consumed by the (fully-dropped) Values \
         arm; exactly 1 more must land on the data arm: {n:?}"
    );
    assert_eq!(*limit, Some(5));
    assert!(
        !matches!(child.as_ref(), IqNode::Union { .. }),
        "the Values arm contributed ZERO survivor rows (all 3 before the window) \
         -- the reconstructed child is the data arm alone, no Union wrapper: {child:?}"
    );
}

/// Nothing to drop or truncate at all (OFFSET 0, the `Values` arm fully survives
/// unmodified, the data arm is reached before the window is satisfied) — the
/// rule must decline (keep `Slice{Union{..}}` byte-identical to the input),
/// not needlessly rebuild an identical tree.
#[test]
fn slice_over_union_declines_when_nothing_changes() {
    let n = norm(
        "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" } } \
         UNION { ?s <http://ex/name> ?n } } LIMIT 5",
    );
    let IqNode::Slice { child, .. } = &n else {
        panic!("expected the Slice to survive untouched: {n:?}")
    };
    let IqNode::Union { children, .. } = child.as_ref() else {
        panic!("expected the Union to survive untouched: {child:?}")
    };
    assert_eq!(children.len(), 2, "both arms untouched: {children:?}");
    assert!(
        matches!(&children[0], IqNode::Construction { child, .. } if matches!(**child, IqNode::Values { .. })),
        "the Values arm keeps its ORIGINAL Construction wrapper (not rebuilt): {:?}",
        children[0]
    );
}

/// A DATA arm appearing BEFORE any `Values` arm can never be dropped (its
/// cardinality is unknown from the very first arm) — declines immediately.
#[test]
fn slice_over_union_declines_when_data_arm_is_first() {
    let n = norm(
        "SELECT ?n WHERE { { ?s <http://ex/name> ?n } \
         UNION { VALUES ?n { \"a\" \"b\" } } } LIMIT 1",
    );
    assert!(
        matches!(&n, IqNode::Slice { child, .. } if matches!(child.as_ref(), IqNode::Union { .. })),
        "must decline -- the data arm is first, nothing is provably unreachable: {n:?}"
    );
}

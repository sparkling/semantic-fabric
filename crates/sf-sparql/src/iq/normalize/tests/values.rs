use super::*;

/// Ontop `ValuesNodeOptimization::test1/test2normalizationSlice`: `Slice` directly
/// over a literal `Values` table (through the builder's identity-projection
/// `Construction` wrapper, confirmed empirically to be the actual top-level shape)
/// truncates the row list in place instead of surviving as a `Slice` node — no
/// `Slice` reaches LOWER at all, so it can never fall back to lowering the full
/// table + a `Plan`-level LIMIT/OFFSET.
#[test]
fn slice_over_values_truncates_in_place() {
    let n = norm("SELECT ?x WHERE { VALUES ?x { 1 2 3 } } LIMIT 1");
    assert!(
        !matches!(n, IqNode::Slice { .. }),
        "Slice must not survive normalize when its child is a Values leaf: {n:?}"
    );
    let IqNode::Construction { child, .. } = &n else {
        panic!("expected the identity-projection Construction wrapper to survive: {n:?}")
    };
    let IqNode::Values { vars, rows } = child.as_ref() else {
        panic!("expected a truncated Values leaf: {child:?}")
    };
    assert_eq!(vars.len(), 1, "one VALUES var");
    assert_eq!(rows.len(), 1, "LIMIT 1 keeps exactly one row: {rows:?}");
    assert_eq!(
        row_int(&rows[0]),
        1,
        "the as-written first row (1), not an arbitrary survivor"
    );
}

/// The OFFSET half of the same rule, and a limit exceeding the remaining rows
/// (clamped, not an out-of-bounds panic).
#[test]
fn slice_over_values_offset_and_overrun() {
    let n = norm("SELECT ?x WHERE { VALUES ?x { 1 2 3 } } LIMIT 5 OFFSET 1");
    let IqNode::Construction { child, .. } = &n else {
        panic!("expected the identity-projection Construction wrapper: {n:?}")
    };
    let IqNode::Values { rows, .. } = child.as_ref() else {
        panic!("expected a truncated Values leaf: {child:?}")
    };
    assert_eq!(
        rows.iter().map(|r| row_int(r)).collect::<Vec<_>>(),
        vec![2, 3],
        "OFFSET 1 skips the first row; LIMIT 5 overruns the remainder harmlessly"
    );
}

/// Ontop `ValuesNodeOptimization::test3normalizationDistinct`: `Distinct` directly
/// over a literal `Values` table (through the identity-projection `Construction`
/// wrapper) dedups the row list in place instead of surviving as a `Distinct` node.
#[test]
fn distinct_over_values_dedups_in_place() {
    let n = norm("SELECT DISTINCT ?x WHERE { VALUES ?x { 1 1 2 2 2 } }");
    assert!(
        !matches!(n, IqNode::Distinct { .. }),
        "Distinct must not survive normalize when its child is a Values leaf: {n:?}"
    );
    let IqNode::Construction { child, .. } = &n else {
        panic!("expected the identity-projection Construction wrapper: {n:?}")
    };
    let IqNode::Values { rows, .. } = child.as_ref() else {
        panic!("expected a deduped Values leaf: {child:?}")
    };
    assert_eq!(
        rows.iter().map(|r| row_int(r)).collect::<Vec<_>>(),
        vec![1, 2],
        "first-occurrence order preserved, duplicates removed: {rows:?}"
    );
}

/// A cell that isn't a plain `Const` (here, a `CONCAT` `TermDef::Concat` produced
/// by the constant-Union fold) has no comparable form at this stage — the dedup
/// declines (a safe no-op: `Distinct` still runs, correctly, at LOWER/exec).
#[test]
fn distinct_over_non_const_cells_declines_safely() {
    let n = norm(
        "SELECT DISTINCT ?x WHERE { { BIND(CONCAT(\"a\",\"b\") AS ?x) } \
         UNION { BIND(CONCAT(\"a\",\"b\") AS ?x) } }",
    );
    let IqNode::Construction { child, .. } = &n else {
        panic!("expected the identity-projection Construction wrapper: {n:?}")
    };
    // The constant-Union fold (test14's rule) still fires here (CONCAT of
    // constants IS foldable into a Values row) -- it's the DEDUP that must
    // decline on the resulting non-Const cell, leaving 2 (duplicate) rows for
    // Distinct/exec to handle downstream, same as before this rule existed.
    let IqNode::Distinct { child: values } = child.as_ref() else {
        panic!("expected Distinct to survive (decline) over a non-Const Values cell: {child:?}")
    };
    let IqNode::Values { rows, .. } = values.as_ref() else {
        panic!("expected the folded (but not deduped) Values leaf: {values:?}")
    };
    assert_eq!(
        rows.len(),
        2,
        "the dedup declined, leaving both rows: {rows:?}"
    );
}

/// Ontop `ValuesNodeOptimization::test14ConstructionUnionTrueTrue`: a `Union` of
/// bare-constant `BIND`-only arms (each `Construction{child: True, ...}`) folds to
/// one `Values` leaf carrying one row per arm.
#[test]
fn constant_union_folds_to_values() {
    let n = norm("SELECT ?x WHERE { { BIND(\"a\" AS ?x) } UNION { BIND(\"b\" AS ?x) } }");
    let IqNode::Construction { child, .. } = &n else {
        panic!("expected the identity-projection Construction wrapper: {n:?}")
    };
    let IqNode::Values { vars, rows } = child.as_ref() else {
        panic!("expected the Union to fold to a Values leaf: {child:?}")
    };
    assert_eq!(vars.len(), 1);
    assert_eq!(rows.len(), 2, "one row per constant arm: {rows:?}");
}

/// THREE arms: `A UNION B UNION C` parses left-associative (`(A UNION B) UNION
/// C`), so the inner pair folds to a bare `Values` before the outer `Union` (with
/// arm C still a `Construction`) ever runs — the full constant fold must absorb
/// an already-folded `Values` arm's rows directly, not just a `Construction` one.
/// (RED before the fix: the outer fold declined on the first arm not being a
/// `Construction`, leaving `Union[Values{[a,b]}, Construction{c}]` unfolded.)
#[test]
fn three_arm_constant_union_folds_to_one_values() {
    let n = norm(
        "SELECT ?x WHERE { { BIND(\"a\" AS ?x) } UNION { BIND(\"b\" AS ?x) } \
         UNION { BIND(\"c\" AS ?x) } }",
    );
    let IqNode::Construction { child, .. } = &n else {
        panic!("expected the identity-projection Construction wrapper: {n:?}")
    };
    let IqNode::Values { rows, .. } = child.as_ref() else {
        panic!("expected all three arms to fold to ONE Values leaf: {child:?}")
    };
    assert_eq!(
        rows.len(),
        3,
        "one row per constant arm, not 2+1 split: {rows:?}"
    );
}

/// Ontop `ValuesNodeOptimization::test26MergeableCombination`: two `VALUES`
/// blocks binding the SAME two variables but declaring them in DIFFERENT header
/// order still fold into one `Values` leaf, cells correctly reordered by name
/// (not position) to the outer `project`'s canonical order — no transposition.
#[test]
fn constant_union_folds_reordered_columns_without_transposing() {
    let n = norm(
        "SELECT ?x ?y WHERE { { VALUES (?x ?y) { (1 2) } } \
         UNION { VALUES (?y ?x) { (3 4) } } }",
    );
    let IqNode::Construction { child, .. } = &n else {
        panic!("expected the identity-projection Construction wrapper: {n:?}")
    };
    let IqNode::Values { vars, rows } = child.as_ref() else {
        panic!("expected both arms to fold to ONE Values leaf: {child:?}")
    };
    assert_eq!(
        vars.iter().map(|v| v.as_ref()).collect::<Vec<&str>>(),
        vec!["x", "y"],
        "outer project order: {vars:?}"
    );
    // Second VALUES block declared (?y ?x) with row (3 4): y=3, x=4 -- must
    // land as (x=4, y=3) once reordered to the [x,y] project order, NOT (x=3,
    // y=4) (a transposition the naive positional copy this rule replaced would
    // produce).
    assert_eq!(
        rows.iter()
            .map(|r| (row_int(&r[0..1]), row_int(&r[1..2])))
            .collect::<Vec<_>>(),
        vec![(1, 2), (4, 3)],
        "row order preserved, but each row's OWN cells reordered by name, not \
         position: {rows:?}"
    );
}

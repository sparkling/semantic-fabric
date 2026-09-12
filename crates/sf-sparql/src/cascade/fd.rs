//! Pass (3) of the optimizer cascade — functional-dependency inference
//! (transitive closure). Kept in its own file to hold `cascade/mod.rs` within
//! the size budget; the result gates pass (4) FK/PK join elimination.

use sf_core::ir::LogicalSource;
use sf_sql::TableSchema;

use crate::iq::{Branch, ColRef, SqlCond};

/// The functional dependencies that hold over a branch's row stream. An entry
/// `(det, alias)` means **`det` determines every column of scan `alias`** (a
/// superkey of that scan's projection). Built for pass (4): a join may be
/// eliminated only once uniqueness is proven, and uniqueness is exactly "the
/// join column is a key" — an FD whose determinant is that column.
///
/// `col_deps` tracks column-level non-unique FDs (`det_col → dep_col`), seeded
/// from [`TableSchema::functional_dependencies`] and used by pass (2e).
#[derive(Debug, Default)]
pub struct Fds {
    /// `(det, alias)`: `det` functionally determines all columns of `alias`.
    pub(super) deps: Vec<(ColRef, usize)>,
    /// `(det_col, dep_col)`: non-unique column-level FD — `det_col` determines
    /// `dep_col` (multiple rows may share the same `det_col` value).
    pub(super) col_deps: Vec<(ColRef, ColRef)>,
}

impl Fds {
    pub(super) fn has(&self, det: &ColRef, alias: usize) -> bool {
        self.deps.iter().any(|(d, a)| d == det && *a == alias)
    }

    pub(super) fn add(&mut self, det: ColRef, alias: usize) -> bool {
        if self.has(&det, alias) {
            false
        } else {
            self.deps.push((det, alias));
            true
        }
    }

    /// Is `c` a key — does it determine its own scan's whole row? This is the
    /// uniqueness precondition pass (4) consults.
    pub fn is_key(&self, c: &ColRef) -> bool {
        self.has(c, c.alias)
    }

    /// Does `det` determine `dep` at the column level (non-unique FD)?
    pub fn determines_col(&self, det: &ColRef, dep: &ColRef) -> bool {
        self.col_deps.iter().any(|(d, p)| d == det && p == dep)
    }

    pub(super) fn add_col_dep(&mut self, det: ColRef, dep: ColRef) -> bool {
        if self.determines_col(&det, &dep) {
            false
        } else {
            self.col_deps.push((det, dep));
            true
        }
    }
}

/// Derive the FD set with its **transitive closure** (ADR-0007 step iii — "FD
/// inference, transitive closure, through unions, *must precede* FK/PK join
/// elimination"). Seeds each single-column unique key as a key→row FD, then
/// closes to a fixpoint under two sound rules:
///
/// * **equality** — for a core key equality `ColEq(a, b)` (`a` and `b` hold the
///   same value on every surviving row), anything `a` determines `b` also
///   determines, and vice-versa.
/// * **transitivity** — if `x` determines all of scan `m` and a column `y` of
///   `m` determines scan `n`, then `x` determines `n`.
///
/// "Through unions" is honoured at the branch granularity: each UNION arm is a
/// separate [`Branch`], so its FDs are inferred independently and a join is
/// eliminated per-arm only on that arm's proven keys.
pub fn infer_functional_dependencies(b: &Branch, schema: &[TableSchema]) -> Fds {
    let mut fds = Fds::default();
    // Seed: every single-column unique key (PK or UNIQUE) determines its row.
    for scan in &b.core {
        if let Some(LogicalSource::Table(t)) = scan.source.logical() {
            if let Some(ts) = schema.iter().find(|s| &s.name == t) {
                for col in single_col_keys(ts) {
                    fds.add(ColRef::new(scan.alias, col), scan.alias);
                }
                // Seed non-unique column-level FDs from TableSchema.
                for fd in &ts.functional_dependencies {
                    if fd.det.len() == 1 {
                        let det = ColRef::new(scan.alias, fd.det[0].as_str());
                        for dep_col in &fd.dep {
                            fds.add_col_dep(det.clone(), ColRef::new(scan.alias, dep_col.as_str()));
                        }
                    }
                }
            }
        }
    }
    // Closure to a fixpoint.
    loop {
        let mut changed = false;
        // Propagate column-level FDs through equality: if det→dep and ColEq(det, x),
        // then x→dep; and if dep→x and ColEq(det, dep), then det→x. Snapshot
        // col_deps ONCE per ROUND (not once per condition, ADR-0024/M4 perf) — a
        // condition later in this same round that would have seen an earlier
        // condition's additions instead picks them up on the NEXT round; the
        // fixpoint loop already runs until nothing changes, so the FINAL `fds` is
        // identical, only the round count can grow (mirrors the `deps` snapshot
        // below, which was already round-scoped).
        let col_snapshot: Vec<(ColRef, ColRef)> = fds.col_deps.clone();
        for cond in &b.where_conds {
            if let SqlCond::ColEq(a, c) = cond {
                changed |= propagate_eq(&mut fds, a, c);
                changed |= propagate_eq(&mut fds, c, a);
                for (det, dep) in &col_snapshot {
                    if det == a {
                        changed |= fds.add_col_dep(c.clone(), dep.clone());
                    }
                    if det == c {
                        changed |= fds.add_col_dep(a.clone(), dep.clone());
                    }
                }
            }
        }
        // transitivity — snapshot the current edges to avoid borrow conflicts.
        let snapshot: Vec<(ColRef, usize)> = fds.deps.clone();
        for (x, m) in &snapshot {
            for (y, n) in &snapshot {
                if y.alias == *m && fds.add(x.clone(), *n) {
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    fds
}

/// Equality rule: given `ColEq(from, to)` (equal values), copy every FD whose
/// determinant is `from` onto `to`. Returns whether anything was added.
fn propagate_eq(fds: &mut Fds, from: &ColRef, to: &ColRef) -> bool {
    let targets: Vec<usize> = fds
        .deps
        .iter()
        .filter(|(d, _)| d == from)
        .map(|(_, a)| *a)
        .collect();
    let mut changed = false;
    for a in targets {
        changed |= fds.add(to.clone(), a);
    }
    changed
}

/// The single-column unique keys of a table (the determinants that fix a row):
/// a single-column primary key, plus any single-column `UNIQUE` constraint.
pub fn single_col_keys(ts: &TableSchema) -> Vec<String> {
    let mut keys = Vec::new();
    if ts.primary_key.len() == 1 {
        keys.push(ts.primary_key[0].clone());
    }
    for u in &ts.unique {
        if u.len() == 1 && !keys.contains(&u[0]) {
            keys.push(u[0].clone());
        }
    }
    keys
}

#[cfg(test)]
mod controlled_fk_tests {
    use super::*;
    use crate::iq::{Scan, TermDef};
    use crate::{build::control::BuildWork, compiler_control::CompileContext, CompilerWorkMode};
    use sf_core::ir::{TermMap, TermSpec};
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn budget(units: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
    }
    struct Stop {
        budget: QueryBudget,
        calls: AtomicUsize,
        at: usize,
        cause: QueryControlError,
    }
    impl QueryControl for Stop {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            kind: QueryCharge,
            units: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.budget.consume(kind, units)?;
            if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
                self.budget.terminate(self.cause);
            }
            Ok(())
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.budget.terminate(cause)
        }
    }

    #[test]
    fn fd_merge_cannot_rewrite_filters_outside_the_dependency() {
        for (optional, location) in [false, true]
            .into_iter()
            .flat_map(|o| (0..3).map(move |l| (o, l)))
        {
            let mut table = TableSchema::new("items");
            table.columns = vec![sf_sql::Column::new("d", "integer", true)];
            table.functional_dependencies.push(sf_sql::FunctionalDep {
                det: vec!["d".into()],
                dep: vec!["p".into()],
            });
            let scan = |alias| Scan {
                alias,
                source: LogicalSource::Table("items".into()).into(),
            };
            let mut branch = Branch::single(scan(0));
            let equal = SqlCond::ColEq(ColRef::new(0, "d"), ColRef::new(1, "d"));
            if optional {
                branch.opts.push(crate::iq::OptJoin {
                    scan: scan(1),
                    on: vec![equal],
                    extra: vec![],
                });
            } else {
                branch.core.push(scan(1));
                branch.where_conds.push(equal);
            }
            branch.where_conds.push(SqlCond::Cmp(
                ColRef::new(1, "x"),
                crate::iq::CmpOp::Eq,
                "1".into(),
            ));
            if location != 0 {
                let filter = branch.where_conds.pop().unwrap();
                let mut later = crate::iq::OptJoin {
                    scan: Scan {
                        alias: 2,
                        source: LogicalSource::Table("other".into()).into(),
                    },
                    on: vec![],
                    extra: vec![],
                };
                if location == 1 {
                    later.on.push(filter);
                } else {
                    later.extra.push(filter);
                }
                branch.opts.push(later);
            }
            branch.bindings.insert(
                "x".into(),
                TermDef::Derived {
                    alias: 0,
                    term_map: TermMap::Column("x".into(), TermSpec::plain_literal()),
                },
            );
            let schema = [table];
            let map = super::super::build_schema_map(&schema);
            let context = super::super::CascadeCtx {
                distinct: true,
                project: None,
            };
            let mut raw = branch.clone();
            super::super::fd_self_join_elimination(&mut raw, &map, &context);
            assert_eq!(
                format!("{raw:?}"),
                format!("{branch:?}"),
                "outside-FD filters cannot move to the kept scan"
            );
            super::super::control_fd::eliminate(
                &mut branch,
                &map,
                &context,
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&budget(
                    u64::MAX,
                )))),
            )
            .unwrap();
            assert_eq!(format!("{raw:?}"), format!("{branch:?}"));
        }
    }

    #[test]
    fn malformed_composite_proofs_preserve_parent_scan() {
        for variant in 0..5 {
            let mut child = TableSchema::new("child");
            child.columns = vec![
                sf_sql::Column::new("c1", "text", true),
                sf_sql::Column::new("c2", "text", true),
            ];
            child.foreign_keys.push(sf_sql::ForeignKey {
                columns: vec!["c1".into(), if variant == 2 { "c1" } else { "c2" }.into()],
                parent_table: "parent".into(),
                parent_columns: if variant == 0 {
                    vec!["p1".into()]
                } else {
                    vec!["p1".into(), if variant == 2 { "p1" } else { "p2" }.into()]
                },
            });
            let mut parent = TableSchema::new("parent");
            parent.primary_key = vec!["p1".into(), if variant == 3 { "p1" } else { "p2" }.into()];
            let mut branch = Branch::single(Scan {
                alias: 0,
                source: LogicalSource::Table("child".into()).into(),
            });
            branch.core.push(Scan {
                alias: 1,
                source: LogicalSource::Table("parent".into()).into(),
            });
            branch
                .where_conds
                .push(SqlCond::ColEq(ColRef::new(0, "c1"), ColRef::new(1, "p1")));
            branch.where_conds.push(SqlCond::ColEq(
                ColRef::new(0, if variant < 3 { "c1" } else { "c2" }),
                ColRef::new(1, if variant < 3 { "p1" } else { "p2" }),
            ));
            let schema = [child, parent];
            let fds = infer_functional_dependencies(&branch, &schema);
            let mut raw = branch.clone();
            super::super::joinelim::fk_pk_join_elimination(&mut raw, &schema, &fds);
            assert_eq!(
                raw.core.len(),
                if variant == 4 { 1 } else { 2 },
                "variant {variant}"
            );
            super::super::joinelim::with_work(
                &mut branch,
                &schema,
                &fds,
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&budget(
                    u64::MAX,
                )))),
            )
            .unwrap();
            assert_eq!(format!("{raw:?}"), format!("{branch:?}"));
        }
    }

    #[test]
    fn complete_fk_pass_has_exact_limits_and_sticky_stops() {
        for composite in [false, true] {
            for variant in 0..5 {
                let mut child = TableSchema::new("child");
                child.columns = vec![
                    sf_sql::Column::new("fk", "text", variant != 1),
                    sf_sql::Column::new("second", "text", true),
                ];
                let mut parent = TableSchema::new("parent");
                parent.primary_key = if composite {
                    vec!["id".into(), "other".into()]
                } else {
                    vec!["id".into()]
                };
                child.foreign_keys.push(sf_sql::ForeignKey {
                    columns: if composite {
                        vec!["fk".into(), "second".into()]
                    } else {
                        vec!["fk".into()]
                    },
                    parent_table: "parent".into(),
                    parent_columns: if composite {
                        vec!["id".into(), "other".into()]
                    } else {
                        vec!["id".into()]
                    },
                });
                let mut branch = Branch::single(Scan {
                    alias: 0,
                    source: LogicalSource::Table("child".into()).into(),
                });
                branch.core.push(Scan {
                    alias: 1,
                    source: LogicalSource::Table("parent".into()).into(),
                });
                branch
                    .where_conds
                    .push(SqlCond::ColEq(ColRef::new(0, "fk"), ColRef::new(1, "id")));
                if composite {
                    branch.where_conds.push(SqlCond::ColEq(
                        ColRef::new(0, "second"),
                        ColRef::new(1, "other"),
                    ));
                }
                if variant == 2 {
                    child.foreign_keys.clear();
                }
                if variant == 3 {
                    branch
                        .where_conds
                        .push(SqlCond::Not(Box::new(SqlCond::DecodedIsNotNull(
                            ColRef::new(1, "id"),
                        ))));
                }
                branch.bindings.insert(
                    "value".into(),
                    TermDef::Derived {
                        alias: 1,
                        term_map: TermMap::Column(
                            if variant == 4 { "outside" } else { "id" }.into(),
                            TermSpec::iri(),
                        ),
                    },
                );
                let schema = [child, parent];
                let fds = infer_functional_dependencies(&branch, &schema);
                let run = |control: &dyn QueryControl| {
                    let mut output = branch.clone();
                    super::super::joinelim::with_work(
                        &mut output,
                        &schema,
                        &fds,
                        BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
                    )?;
                    Ok::<_, crate::Error>(output)
                };
                let measured = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at: usize::MAX,
                    cause: QueryControlError::Cancelled,
                };
                let output = run(&measured).unwrap();
                assert_eq!(output.core.len(), if variant == 0 { 1 } else { 2 });
                if variant == 0 {
                    let TermDef::Derived {
                        alias,
                        term_map: TermMap::Column(name, _),
                    } = &output.bindings["value"]
                    else {
                        panic!("column binding");
                    };
                    assert_eq!((*alias, name.as_ref()), (0, "fk"));
                }
                let units = measured.budget.consumed(QueryCharge::CompilerWork);
                assert_eq!(
                    format!("{:?}", run(&budget(units)).unwrap()),
                    format!("{output:?}")
                );
                assert!(matches!(
                    run(&budget(units - 1)),
                    Err(crate::Error::QueryControl(
                        QueryControlError::CompilerWorkExceeded
                    ))
                ));
                for cause in [
                    QueryControlError::Cancelled,
                    QueryControlError::DeadlineExceeded,
                ] {
                    for at in 1..=measured.calls.load(Ordering::Relaxed) {
                        let stop = Stop {
                            budget: budget(u64::MAX),
                            calls: AtomicUsize::new(0),
                            at,
                            cause,
                        };
                        assert!(
                            matches!(run(&stop), Err(crate::Error::QueryControl(actual)) if actual == cause)
                        );
                        assert_eq!(stop.checkpoint(), Err(cause));
                    }
                }
            }
        }
    }
}

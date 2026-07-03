//! Resolve — the operator-tree ([`IqNode`]) RESOLVE stage (ADR-0023 M3a,
//! `docs/design/ADR-0023-M3-resolution-pipeline.md` §3). It consumes the
//! context-free tree produced by [`crate::build::build_tree`] and returns a tree
//! with **ZERO** [`IqNode::Intensional`] leaves: every unresolved triple pattern is
//! replaced by the resolved relational subtree it unfolds to against the
//! T-mappings. Everything else — `Construction`/`Filter`/`InnerJoin`/`LeftJoin`/
//! `Union`/`Aggregation`/`Distinct`/`Slice`/`OrderBy`/`Values`/`Empty`/`True`/`Path`
//! — passes through, only recursing into children. **FILTER/BIND stay symbolic:**
//! an [`IqCond::Expr`] and a [`BindDef::Expr`] are carried through untouched and
//! resolved per-leaf-CQ only at LOWER (M3 design §3, §5).
//!
//! ## Status: tree path only (NOT the live engine)
//!
//! This is M3a; it is **not** wired into the live [`Plan`](crate::Plan)/exec/unfold
//! path. The flat [`crate::unfold`] stays the production engine and the proven
//! oracle. `cargo test --workspace` must stay green with the flat path byte-for-byte
//! unchanged.
//!
//! ## Smallest change: the bridge over the flat oracle (M3 design §3.3 fallback)
//!
//! Rather than fork the per-`(triples-map × POM)` atom logic into a new shared
//! primitive (which risks perturbing the byte-identical flat `atom`/`pattern_branches`
//! oracle this milestone must preserve), RESOLVE **calls the flat
//! [`Unfolder::pattern_branches`] verbatim** (via the graph-scoped
//! [`Unfolder::resolve_pattern`]) and **bridges** each resulting [`Branch`] to an
//! [`IqNode`] arm. The arm set, conds, fresh aliases, and `predicate_can_match`
//! pruning are therefore *identical* to the flat translation by construction — that is
//! the `=_bag` argument (M3 design §3.3, §6, ledger R1). The flat `Vec<Branch>` bag
//! union becomes:
//!
//! * **0 arms** ⇒ [`IqNode::Empty`] over the pattern's variables;
//! * **1 arm** ⇒ that arm's subtree directly (no `Union` wrapper);
//! * **N ≥ 2 arms** ⇒ [`IqNode::Union`] over the arms, `project` = the pattern's
//!   variables.
//!
//! Each arm bridges one [`Branch`] (which, for a single triple pattern, only ever uses
//! `core` + `where_conds` + `bindings` — never `opts`/`path`/`agg`) to:
//!
//! ```text
//! Construction {
//!   subst:   branch.bindings  → BindDef::Resolved(TermDef)   (the var → term scope)
//!   project: branch.bindings.keys()                          (the arm's bound vars)
//!   child:   InnerJoin {                                     (CROSS JOIN + WHERE-eq,
//!     children: branch.core → Extensional { scan }            exactly the flat core)
//!     cond:     branch.where_conds → IqCond::Sql(SqlCond)     (constant-position &
//!   }                                                          shared-var / refObjectMap
//! }                                                            equalities)
//! ```
//!
//! A single-scan arm with no conds collapses to a bare [`IqNode::Extensional`] (no
//! degenerate one-child `InnerJoin`). A `rr:refObjectMap` arm needs no special case:
//! the flat `atom` already pushed the parent scan into `core` and the
//! [`SqlCond::ColEq`] join into `where_conds`, so it bridges to the contract's 2-scan
//! `InnerJoin` automatically (M3 design §3.1, §3.4).
//!
//! All join logic lives in the `IqCond::Sql` conds (not in `Extensional.bind`, which
//! the bridge leaves empty): the `InnerJoin` is a cross-join driven by explicit
//! equalities, mirroring the flat `core` + `where_conds` "CROSS JOIN + WHERE-eq ≡ inner
//! join" lowering (`iq.rs` [`Branch`] doc).

use std::collections::BTreeMap;

use sf_core::ir::TriplesMap;

use crate::iq::node::{triple_pattern_vars, BindDef, IqCond, IqNode};
use crate::iq::Branch;
use crate::saturate::Tbox;
use crate::unfold::Unfolder;
use crate::Result;

/// The resolution context (M3 design §3): the T-mappings, the T-Box, the SQL
/// dialect, and a **monotone alias counter** shared across the whole tree.
///
/// It wraps a single [`Unfolder`] so that every [`IqNode::Intensional`] in the tree
/// draws from the *same* alias counter — sibling intensionals therefore get disjoint
/// scan aliases (the precondition for a parent `InnerJoin`/`Union` to compose their
/// arms without alias collisions, design §3.2). The wrapped `Unfolder` is the proven
/// flat oracle, used read-only here except for its alias counter and the transient
/// `current_graph` that [`Unfolder::resolve_pattern`] saves/restores per leaf.
pub struct ResolveCx<'a> {
    unfolder: Unfolder<'a>,
}

impl<'a> ResolveCx<'a> {
    /// A fresh resolution context over the given mappings, T-Box, and dialect (the
    /// same `(maps, tbox, dialect)` the flat [`Unfolder::new`] takes). The alias
    /// counter starts at zero and advances monotonically across the whole tree.
    pub fn new(maps: &'a [TriplesMap], tbox: &'a Tbox, dialect: sf_sql::Dialect) -> Self {
        Self {
            unfolder: Unfolder::new(maps, tbox, dialect),
        }
    }
}

/// Resolve a whole tree (M3 design §3): walk `node` and replace every
/// [`IqNode::Intensional`] leaf with its resolved subtree, returning a tree with
/// **ZERO** `Intensional` leaves. Every other node recurses into its children
/// unchanged; FILTER/BIND symbolic leaves ([`IqCond::Expr`] / [`BindDef::Expr`]) are
/// carried through untouched (resolved per-leaf-CQ at LOWER, design §5).
///
/// `EXISTS`/`NOT EXISTS` carry a built [`IqNode`] subtree (in [`IqCond::Exists`] /
/// [`IqCond::NotExists`]) that may itself contain `Intensional` leaves, so RESOLVE
/// descends into those subtrees too — the "ZERO Intensional" invariant is over the
/// *entire* tree, including condition-embedded patterns.
pub fn resolve(node: IqNode, cx: &mut ResolveCx) -> Result<IqNode> {
    match node {
        // ---- the one resolving case ---------------------------------------------
        IqNode::Intensional { pattern, graph } => {
            let vars = triple_pattern_vars(&pattern);
            let branches = cx.unfolder.resolve_pattern(&pattern, graph.as_ref())?;
            let mut arms: Vec<IqNode> = branches.into_iter().map(bridge_branch).collect();
            Ok(match arms.len() {
                0 => IqNode::Empty { vars },
                1 => arms.pop().expect("len checked == 1"),
                _ => IqNode::Union {
                    children: arms,
                    project: vars,
                },
            })
        }

        // ---- the property-path resolving case (M5 Wave 1) -----------------------
        // Reuse the flat `path_branch` VERBATIM via `resolve_path` (pinning the
        // constant active graph exactly as the flat `GRAPH <g> { ?s PATH ?o }` path
        // does), then bridge the single resulting `Branch` (which carries a
        // `path = Some(PathClosure)`) to an `IqNode::Path` UNDER its `Construction`
        // bindings via the SAME `bridge_branch` the triple case uses. A path pattern
        // is one bag alternative (the flat `GraphPattern::Path` arm yields exactly
        // `vec![path_branch(...)]`), so there is never a `Union` wrapper here. ZERO
        // `UnresolvedPath` survives — `bridge_branch` produces no `UnresolvedPath`.
        IqNode::UnresolvedPath {
            subject,
            path,
            object,
            graph,
        } => {
            let branch = cx
                .unfolder
                .resolve_path(&subject, &path, &object, graph.as_ref())?;
            Ok(bridge_branch(branch))
        }

        // ---- recurse into children, structure unchanged -------------------------
        IqNode::Construction {
            child,
            subst,
            project,
        } => Ok(IqNode::Construction {
            child: Box::new(resolve(*child, cx)?),
            subst,
            project,
        }),
        IqNode::Filter { child, cond } => Ok(IqNode::Filter {
            child: Box::new(resolve(*child, cx)?),
            cond: resolve_conds(cond, cx)?,
        }),
        IqNode::InnerJoin { children, cond } => Ok(IqNode::InnerJoin {
            children: resolve_children(children, cx)?,
            cond: resolve_conds(cond, cx)?,
        }),
        IqNode::LeftJoin { left, right, cond } => Ok(IqNode::LeftJoin {
            left: Box::new(resolve(*left, cx)?),
            right: Box::new(resolve(*right, cx)?),
            cond: resolve_conds(cond, cx)?,
        }),
        IqNode::Union { children, project } => Ok(IqNode::Union {
            children: resolve_children(children, cx)?,
            project,
        }),
        IqNode::Aggregation {
            child,
            grouping,
            aggs,
        } => Ok(IqNode::Aggregation {
            child: Box::new(resolve(*child, cx)?),
            grouping,
            aggs,
        }),
        IqNode::Distinct { child } => Ok(IqNode::Distinct {
            child: Box::new(resolve(*child, cx)?),
        }),
        IqNode::Slice {
            child,
            offset,
            limit,
        } => Ok(IqNode::Slice {
            child: Box::new(resolve(*child, cx)?),
            offset,
            limit,
        }),
        IqNode::OrderBy { child, keys } => Ok(IqNode::OrderBy {
            child: Box::new(resolve(*child, cx)?),
            keys,
        }),

        // ---- already-resolved leaves / identities pass through ------------------
        leaf @ (IqNode::Values { .. }
        | IqNode::Extensional { .. }
        | IqNode::Empty { .. }
        | IqNode::True
        | IqNode::Path { .. }) => Ok(leaf),
    }
}

/// Resolve each child of an n-ary node (every `Intensional` inside is replaced).
fn resolve_children(children: Vec<IqNode>, cx: &mut ResolveCx) -> Result<Vec<IqNode>> {
    children.into_iter().map(|c| resolve(c, cx)).collect()
}

/// Resolve a conjunction of [`IqCond`]s: the symbolic `Expr`/`Sql` leaves are left
/// untouched (FILTER/BIND are NOT resolved by RESOLVE); only the `EXISTS`/`NOT EXISTS`
/// subtrees recurse, so an `Intensional` embedded in a FILTER is resolved like any
/// other.
fn resolve_conds(conds: Vec<IqCond>, cx: &mut ResolveCx) -> Result<Vec<IqCond>> {
    conds.into_iter().map(|c| resolve_cond(c, cx)).collect()
}

/// Resolve one [`IqCond`] (design §3, recursion clause). `Expr`/`Sql` are symbolic
/// FILTER/ON leaves — passed through verbatim; `Exists`/`NotExists` recurse into their
/// built subtrees.
fn resolve_cond(cond: IqCond, cx: &mut ResolveCx) -> Result<IqCond> {
    match cond {
        IqCond::Expr(e) => Ok(IqCond::Expr(e)),
        IqCond::Sql(s) => Ok(IqCond::Sql(s)),
        IqCond::And(cs) => Ok(IqCond::And(resolve_conds(cs, cx)?)),
        IqCond::Or(cs) => Ok(IqCond::Or(resolve_conds(cs, cx)?)),
        IqCond::Not(c) => Ok(IqCond::Not(Box::new(resolve_cond(*c, cx)?))),
        IqCond::Exists(n) => Ok(IqCond::Exists(Box::new(resolve(*n, cx)?))),
        IqCond::NotExists { inner, is_minus } => Ok(IqCond::NotExists {
            inner: Box::new(resolve(*inner, cx)?),
            is_minus,
        }),
    }
}

/// Bridge one resolved flat [`Branch`] to an [`IqNode`] arm (module docs; M3 design
/// §3.1, §3.3 fallback; M5 Wave 1 path extension). A **triple-pattern** branch only ever
/// populates `core` + `where_conds` + `bindings` (`atom`/`class_atoms` never set
/// `opts`/`path`/`agg`): the scans become [`IqNode::Extensional`] leaves, the WHERE conds
/// become [`IqCond::Sql`] join/constant conditions, and the bindings become the
/// [`IqNode::Construction`] substitution (the var → [`TermDef`] scope, design §3.2). A
/// **property-path** branch (M5 Wave 1) instead carries `path = Some(PathClosure)` with an
/// empty `core`: the bridge must NOT drop it (the old `..` rest pattern silently did), so
/// the relational body becomes the [`IqNode::Path`] closure leaf, with any `where_cond`
/// (the `?s PATH ?s` self-unify [`SqlCond::ColEq`] from `bind`) wrapping it in a `Filter`.
/// Either way the body sits under the same `Construction(bindings)`, so an outer
/// `InnerJoin`/`LeftJoin`/`Filter` composes over the path arm exactly as over a triple arm.
fn bridge_branch(branch: Branch) -> IqNode {
    let Branch {
        core,
        bindings,
        where_conds,
        path,
        ..
    } = branch;

    let conds: Vec<IqCond> = where_conds.into_iter().map(IqCond::Sql).collect();

    let child = match path {
        // ---- property-path closure arm (M5 Wave 1) ------------------------------
        // The relational body is the recursive `PathClosure` leaf (empty `core` by
        // construction). A self-path `where_cond` (`?s PATH ?s` ⇒ `ColEq sf_s,sf_o`)
        // wraps the leaf in a `Filter` so LOWER pushes it via `apply_conds` after the
        // `Branch::path` leaf lowers (it never rides an `InnerJoin`, which has no scans).
        Some(closure) => {
            let leaf = IqNode::Path { closure };
            if conds.is_empty() {
                leaf
            } else {
                IqNode::Filter {
                    child: Box::new(leaf),
                    cond: conds,
                }
            }
        }
        // ---- triple-pattern arm (M3a) -------------------------------------------
        // The flat `core` + `where_conds`, as a CROSS JOIN + WHERE-eq InnerJoin
        // (`Extensional.bind` left empty — all join logic is carried by the explicit
        // `IqCond::Sql` conds, exactly mirroring the flat lowering).
        None => {
            let scans: Vec<IqNode> = core
                .into_iter()
                .map(|scan| IqNode::Extensional {
                    scan,
                    bind: BTreeMap::new(),
                })
                .collect();
            if scans.len() == 1 && conds.is_empty() {
                scans.into_iter().next().expect("len checked == 1")
            } else {
                IqNode::InnerJoin {
                    children: scans,
                    cond: conds,
                }
            }
        }
    };

    // The var → resolved-term scope (design §3.2): each flat binding becomes a
    // `BindDef::Resolved(TermDef)`; the arm projects exactly its bound variables.
    let project = bindings.keys().map(|v| v.as_str().into()).collect();
    let subst = bindings
        .into_iter()
        .map(|(v, td)| (v.into(), BindDef::Resolved(td)))
        .collect();

    IqNode::Construction {
        child: Box::new(child),
        subst,
        project,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::build_tree;
    use crate::iq::node::IqNode;
    use sf_core::ir::{
        LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, SubjectMap, Template, TermMap,
        TermSpec, TriplesMap,
    };
    use sf_core::NamedNode;
    use spargebra::algebra::GraphPattern;

    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    fn template_iri(t: &str) -> TermMap {
        TermMap::Template(Template::parse(t).unwrap(), TermSpec::iri())
    }

    fn column_literal(c: &str) -> TermMap {
        TermMap::Column(c.into(), TermSpec::plain_literal())
    }

    fn pom(predicate: &str, object: ObjectMap) -> PredicateObjectMap {
        PredicateObjectMap {
            predicates: vec![TermMap::Constant(iri(predicate).into())],
            objects: vec![object],
            graphs: vec![],
        }
    }

    /// EMP(id,name,dept_id) + DEPT(id,dname); EMP :name (column) and EMP :dept
    /// (refObjectMap → DEPT) — a representative single-/multi-map mapping.
    fn mapping() -> Vec<TriplesMap> {
        let emp = TriplesMap {
            id: "EMP".to_owned(),
            source: LogicalSource::Table("emp".to_owned()),
            subject: SubjectMap {
                term: template_iri("http://ex/emp/{id}"),
                classes: vec![iri("http://ex/Employee")],
                graphs: vec![],
            },
            predicate_object_maps: vec![
                pom("http://ex/name", ObjectMap::Term(column_literal("name"))),
                pom(
                    "http://ex/dept",
                    ObjectMap::Ref(RefObjectMap {
                        parent_triples_map: "DEPT".to_owned(),
                        joins: vec![sf_core::ir::Join {
                            child: "dept_id".to_owned(),
                            parent: "id".to_owned(),
                        }],
                    }),
                ),
            ],
        };
        let dept = TriplesMap {
            id: "DEPT".to_owned(),
            source: LogicalSource::Table("dept".to_owned()),
            subject: SubjectMap {
                term: template_iri("http://ex/dept/{id}"),
                classes: vec![iri("http://ex/Department")],
                graphs: vec![],
            },
            predicate_object_maps: vec![pom(
                "http://ex/dname",
                ObjectMap::Term(column_literal("dname")),
            )],
        };
        vec![emp, dept]
    }

    fn pattern(q: &str) -> GraphPattern {
        match spargebra::SparqlParser::new().parse_query(q).unwrap() {
            spargebra::Query::Select { pattern, .. } => pattern,
            other => panic!("expected SELECT, got {other:?}"),
        }
    }

    /// `true` iff any node in the tree is an `Intensional` (including inside
    /// FILTER EXISTS / NOT EXISTS subtrees).
    fn has_intensional(node: &IqNode) -> bool {
        match node {
            IqNode::Intensional { .. } => true,
            IqNode::Construction { child, .. }
            | IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. }
            | IqNode::Aggregation { child, .. } => has_intensional(child),
            IqNode::Filter { child, cond } => {
                has_intensional(child) || cond.iter().any(cond_has_intensional)
            }
            IqNode::InnerJoin { children, cond } => {
                children.iter().any(has_intensional) || cond.iter().any(cond_has_intensional)
            }
            IqNode::LeftJoin { left, right, cond } => {
                has_intensional(left)
                    || has_intensional(right)
                    || cond.iter().any(cond_has_intensional)
            }
            IqNode::Union { children, .. } => children.iter().any(has_intensional),
            // `UnresolvedPath` is the M5 path companion of `Intensional` — also a
            // transient leaf that must not survive RESOLVE; treated as a violation here so
            // the `resolve_leaves_zero_intensional` invariant covers paths too.
            IqNode::UnresolvedPath { .. } => true,
            IqNode::Values { .. }
            | IqNode::Extensional { .. }
            | IqNode::Empty { .. }
            | IqNode::True
            | IqNode::Path { .. } => false,
        }
    }

    fn cond_has_intensional(c: &IqCond) -> bool {
        match c {
            IqCond::Expr(_) | IqCond::Sql(_) => false,
            IqCond::And(cs) | IqCond::Or(cs) => cs.iter().any(cond_has_intensional),
            IqCond::Not(c) => cond_has_intensional(c),
            IqCond::Exists(n) | IqCond::NotExists { inner: n, .. } => has_intensional(n),
        }
    }

    /// The flat per-pattern arm count (the oracle this milestone reproduces).
    fn flat_arm_count(q: &str, maps: &[TriplesMap]) -> usize {
        let tp = match pattern(q) {
            GraphPattern::Project { inner, .. } => match *inner {
                GraphPattern::Bgp { mut patterns } => patterns.pop().unwrap(),
                other => panic!("expected single-triple BGP, got {other:?}"),
            },
            other => panic!("expected Project, got {other:?}"),
        };
        let tbox = Tbox::new();
        let mut u = Unfolder::new(maps, &tbox, sf_sql::Dialect::Sqlite);
        u.resolve_pattern(&tp, None).unwrap().len()
    }

    /// The resolved-tree arm count for the same single-triple pattern: count the
    /// arms of the `Union` (≥2), or 1 for a bare arm, or 0 for `Empty`.
    fn resolved_arm_count(q: &str, maps: &[TriplesMap]) -> usize {
        let tbox = Tbox::new();
        let mut cx = ResolveCx::new(maps, &tbox, sf_sql::Dialect::Sqlite);
        let tree = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
        // Strip the outer Project Construction the parser wraps `SELECT *` in.
        let inner = match tree {
            IqNode::Construction { child, .. } => *child,
            other => other,
        };
        match inner {
            IqNode::Empty { .. } => 0,
            IqNode::Union { children, .. } => children.len(),
            _ => 1,
        }
    }

    /// An `Intensional` resolves to the SAME arm count as the flat
    /// `pattern_branches` oracle — for a constant-predicate single-map pattern, a
    /// variable-predicate multi-arm pattern, an `rdf:type` class atom, and a pattern
    /// no map can serve (0 arms).
    #[test]
    fn resolve_arm_count_matches_flat_oracle() {
        let maps = mapping();
        for q in [
            // EMP :name ?n — one constant-predicate arm.
            "SELECT * WHERE { ?s <http://ex/name> ?n }",
            // ?s ?p ?o — every (class atom + POM) arm across both maps.
            "SELECT * WHERE { ?s ?p ?o }",
            // rdf:type ?c — the two rr:class atoms.
            &format!("SELECT * WHERE {{ ?s <{RDF_TYPE}> ?c }}"),
            // an unmapped predicate — zero arms.
            "SELECT * WHERE { ?s <http://ex/nope> ?o }",
        ] {
            assert_eq!(
                resolved_arm_count(q, &maps),
                flat_arm_count(q, &maps),
                "arm-count parity broken for {q}"
            );
        }
    }

    /// A `rr:refObjectMap` pattern bridges to a 2-scan `InnerJoin` (child scan ⋈
    /// parent scan) under the arm's `Construction` (design §3.1, §3.4).
    #[test]
    fn ref_object_map_resolves_to_two_scan_inner_join() {
        let maps = mapping();
        let tbox = Tbox::new();
        let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite);
        let q = "SELECT * WHERE { ?s <http://ex/dept> ?d }";
        let tree = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
        // Project Construction → the single arm Construction → InnerJoin of 2 scans.
        let inner = match tree {
            IqNode::Construction { child, .. } => *child,
            other => panic!("expected Project Construction, got {other:?}"),
        };
        let IqNode::Construction { child, .. } = inner else {
            panic!("expected arm Construction, got {inner:?}");
        };
        let IqNode::InnerJoin { children, cond } = *child else {
            panic!("expected 2-scan InnerJoin, got {child:?}");
        };
        assert_eq!(children.len(), 2, "refObjectMap → child ⋈ parent scan");
        assert!(
            children
                .iter()
                .all(|c| matches!(c, IqNode::Extensional { .. })),
            "both children are Extensional scans"
        );
        // The join condition is the rr:joinCondition ColEq, carried as IqCond::Sql.
        assert!(
            cond.iter().any(|c| matches!(c, IqCond::Sql(_))),
            "the refObjectMap join equality is carried as IqCond::Sql: {cond:?}"
        );
    }

    /// `resolve` leaves ZERO `Intensional` leaves anywhere in the tree, including
    /// inside a FILTER EXISTS subtree.
    #[test]
    fn resolve_leaves_zero_intensional() {
        let maps = mapping();
        let tbox = Tbox::new();
        for q in [
            "SELECT * WHERE { ?s <http://ex/name> ?n . ?s <http://ex/dept> ?d }",
            "SELECT * WHERE { ?s ?p ?o OPTIONAL { ?s <http://ex/name> ?n } }",
            "SELECT * WHERE { { ?s <http://ex/name> ?n } UNION { ?s <http://ex/dname> ?n } }",
            "SELECT * WHERE { ?s <http://ex/name> ?n FILTER EXISTS { ?s <http://ex/dept> ?d } }",
            "SELECT * WHERE { ?s <http://ex/name> ?n MINUS { ?s <http://ex/dept> ?d } }",
        ] {
            let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite);
            let tree = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
            assert!(
                !has_intensional(&tree),
                "Intensional survived resolve for {q}: {tree:?}"
            );
        }
    }

    /// FILTER (`IqCond::Expr`) and BIND (`BindDef::Expr`) survive RESOLVE untouched —
    /// they are resolved per-leaf-CQ at LOWER, never by RESOLVE (design §3, §5).
    #[test]
    fn filter_and_bind_survive_resolve_untouched() {
        let maps = mapping();
        let tbox = Tbox::new();
        let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite);
        // FILTER(?n > "5") stays an IqCond::Expr; BIND(?b := ?n) stays a BindDef::Expr.
        let q = "SELECT * WHERE { ?s <http://ex/name> ?n . BIND(?n AS ?b) FILTER(?n > \"5\") }";
        let tree = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
        assert!(
            symbolic_filter_present(&tree),
            "IqCond::Expr must survive: {tree:?}"
        );
        assert!(
            symbolic_bind_present(&tree),
            "BindDef::Expr must survive: {tree:?}"
        );
    }

    fn symbolic_filter_present(node: &IqNode) -> bool {
        match node {
            IqNode::Filter { child, cond } => {
                cond.iter().any(|c| matches!(c, IqCond::Expr(_))) || symbolic_filter_present(child)
            }
            IqNode::Construction { child, .. }
            | IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. }
            | IqNode::Aggregation { child, .. } => symbolic_filter_present(child),
            IqNode::InnerJoin { children, .. } | IqNode::Union { children, .. } => {
                children.iter().any(symbolic_filter_present)
            }
            IqNode::LeftJoin { left, right, .. } => {
                symbolic_filter_present(left) || symbolic_filter_present(right)
            }
            _ => false,
        }
    }

    fn symbolic_bind_present(node: &IqNode) -> bool {
        match node {
            IqNode::Construction { child, subst, .. } => {
                subst.values().any(|d| matches!(d, BindDef::Expr(_)))
                    || symbolic_bind_present(child)
            }
            IqNode::Filter { child, .. }
            | IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. }
            | IqNode::Aggregation { child, .. } => symbolic_bind_present(child),
            IqNode::InnerJoin { children, .. } | IqNode::Union { children, .. } => {
                children.iter().any(symbolic_bind_present)
            }
            IqNode::LeftJoin { left, right, .. } => {
                symbolic_bind_present(left) || symbolic_bind_present(right)
            }
            _ => false,
        }
    }
}

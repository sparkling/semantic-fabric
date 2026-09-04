use std::collections::BTreeMap;

use super::*;
use crate::build::build_tree;
use crate::iq::node::{BindDef, IqCond, IqNode, Var};
use crate::iq::resolve::{resolve, ResolveCx};
use crate::iq::{Scan, TermDef};
use crate::saturate::Tbox;
use sf_core::ir::{
    LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, SubjectMap, Template, TermMap,
    TermSpec, TriplesMap,
};
use sf_core::NamedNode;
use spargebra::algebra::GraphPattern;

mod distinct;
mod joins;
mod slice;
mod spine;
mod values;

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
/// (refObjectMap → DEPT) — mirrors the resolve.rs fixture.
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

/// `emp`/`dept`'s schema, PK-keyed on `id` (matching `mapping()`'s own
/// `{id}`-templated subjects) — ADR-0034 D1 forces `SELECT DISTINCT` on any
/// unkeyed scan, so these structural, shape-focused tests must supply a
/// schema proving `mapping()`'s tables ARE keyed, or every arm here would
/// grow an extra `Distinct` wrapper unrelated to what each test examines.
fn keyed_schema() -> Vec<sf_sql::TableSchema> {
    let mut emp = sf_sql::TableSchema::new("emp");
    emp.primary_key = vec!["id".to_owned()];
    let mut dept = sf_sql::TableSchema::new("dept");
    dept.primary_key = vec!["id".to_owned()];
    vec![emp, dept]
}

fn pattern(q: &str) -> GraphPattern {
    match spargebra::SparqlParser::new().parse_query(q).unwrap() {
        spargebra::Query::Select { pattern, .. } => pattern,
        other => panic!("expected SELECT, got {other:?}"),
    }
}

/// build → resolve → normalize a query against the fixture mapping.
fn norm(q: &str) -> IqNode {
    let maps = mapping();
    let tbox = Tbox::new();
    let schema = keyed_schema();
    let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite, &schema);
    let resolved = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
    normalize(resolved).unwrap()
}

/// Strip the outermost modifier spine (Distinct/Slice/OrderBy/Aggregation) and the
/// top *projection* Construction, returning the body the spine sits over. A
/// Construction is stripped ONLY when it sits over a `Union`/`LeftJoin` (a pure
/// spine projection); a Construction over a relational body IS the single leaf-CQ
/// (the projection folded into it) and is returned as-is.
fn strip_spine(node: &IqNode) -> &IqNode {
    match node {
        IqNode::Distinct { child }
        | IqNode::Slice { child, .. }
        | IqNode::OrderBy { child, .. }
        | IqNode::Aggregation { child, .. } => strip_spine(child),
        IqNode::Construction { child, .. }
            if matches!(**child, IqNode::Union { .. } | IqNode::LeftJoin { .. }) =>
        {
            strip_spine(child)
        }
        other => other,
    }
}

/// A leaf-CQ body is a Join/LeftJoin/Filter of leaves, or a bare leaf — but NEVER
/// a nested Construction (the single-bindings-map invariant) and NEVER a Union.
fn is_leaf_or_join_of_leaves(node: &IqNode) -> bool {
    match node {
        IqNode::Extensional { .. } | IqNode::Values { .. } | IqNode::Path { .. } => true,
        IqNode::InnerJoin { children, .. } => children.iter().all(|c| {
            // join children are leaves, or LeftJoin/Filter sub-CQs (which keep
            // their own internal Construction), never a bare nested Construction.
            matches!(
                c,
                IqNode::Extensional { .. }
                    | IqNode::Values { .. }
                    | IqNode::Path { .. }
                    | IqNode::LeftJoin { .. }
                    | IqNode::Filter { .. }
            )
        }),
        IqNode::Filter { child, .. } => is_leaf_or_join_of_leaves(child),
        IqNode::LeftJoin { .. } => true,
        _ => false,
    }
}

/// Assert a normalized arm is a leaf-CQ: exactly ONE Construction at its root over
/// a Join/Filter of leaves (no nested Construction, no Union below).
fn assert_leaf_cq(arm: &IqNode) {
    let IqNode::Construction { child, .. } = arm else {
        panic!("leaf-CQ must be a Construction at its root, got {arm:?}");
    };
    assert!(
        is_leaf_or_join_of_leaves(child),
        "leaf-CQ Construction must be over a Join/Filter of leaves, got {child:?}"
    );
}

fn arm_count(body: &IqNode) -> usize {
    match body {
        IqNode::Union { children, .. } => children.len(),
        IqNode::Empty { .. } => 0,
        _ => 1,
    }
}

/// One row's sole cell as a plain integer, for the assertions above.
fn row_int(row: &[Option<TermDef>]) -> i64 {
    let TermDef::Const(t) = row[0].as_ref().expect("VALUES cell is bound") else {
        panic!("expected a Const cell: {row:?}")
    };
    let sf_core::Term::Literal(lit) = t else {
        panic!("expected a literal term: {t:?}")
    };
    lit.value()
        .parse()
        .unwrap_or_else(|_| panic!("expected an integer literal: {lit:?}"))
}

/// One row's sole cell as a plain string, for the arm-drop assertions above.
fn row_str(row: &[Option<TermDef>]) -> String {
    let TermDef::Const(t) = row[0].as_ref().expect("VALUES cell is bound") else {
        panic!("expected a Const cell: {row:?}")
    };
    let sf_core::Term::Literal(lit) = t else {
        panic!("expected a literal term: {t:?}")
    };
    lit.value().to_owned()
}

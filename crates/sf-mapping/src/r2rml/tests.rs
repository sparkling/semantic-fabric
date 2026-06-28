//! Unit tests for the R2RML → IR parser. `use super::*` re-uses the parent
//! module's IR imports (`TriplesMap`, `TermMap`, `Term`, `NamedNode`, …) and the
//! public [`parse_r2rml`].

use super::*;

/// A small R2RML mapping exercising: a template subject, a column object, an
/// `rr:class`, a referencing object map with a join, `rr:tableName` vs
/// `rr:sqlQuery`, and an explicit `rr:datatype`. Relative `<#...>` triples-map
/// IRIs exercise base-IRI resolution.
const FIXTURE: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

<#TriplesMap_Employee>
    rr:logicalTable [ rr:tableName "EMP" ] ;
    rr:subjectMap [
        rr:template "http://example.com/emp/{empno}" ;
        rr:class foaf:Person
    ] ;
    rr:predicateObjectMap [
        rr:predicate foaf:name ;
        rr:objectMap [ rr:column "ename" ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:department ;
        rr:objectMap [
            rr:parentTriplesMap <#TriplesMap_Dept> ;
            rr:joinCondition [ rr:child "deptno" ; rr:parent "deptno" ]
        ]
    ] .

<#TriplesMap_Dept>
    rr:logicalTable [ rr:sqlQuery "SELECT deptno, dname FROM DEPT" ] ;
    rr:subjectMap [ rr:template "http://example.com/dept/{deptno}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:name ;
        rr:objectMap [ rr:column "dname" ; rr:datatype xsd:string ]
    ] .
"#;

fn map_by_suffix<'a>(maps: &'a [TriplesMap], suffix: &str) -> &'a TriplesMap {
    maps.iter()
        .find(|m| m.id.ends_with(suffix))
        .unwrap_or_else(|| panic!("no triples map ending in {suffix}"))
}

/// Find the predicate-object map carrying a given constant predicate IRI
/// (order-independent of how the Turtle parser emits blank-node triples).
fn pom_with_predicate<'a>(map: &'a TriplesMap, predicate: &str) -> &'a PredicateObjectMap {
    map.predicate_object_maps
        .iter()
        .find(|pom| {
            pom.predicates.iter().any(|p| {
                matches!(p, TermMap::Constant(Term::NamedNode(n)) if n.as_str() == predicate)
            })
        })
        .unwrap_or_else(|| panic!("no predicate-object map for {predicate}"))
}

#[test]
fn parses_r2rml_fixture_into_ir() {
    let maps = parse_r2rml(FIXTURE).expect("fixture parses");
    assert_eq!(maps.len(), 2, "two triples maps");

    let emp = map_by_suffix(&maps, "#TriplesMap_Employee");
    let dept = map_by_suffix(&maps, "#TriplesMap_Dept");

    // Logical sources: rr:tableName vs rr:sqlQuery.
    assert!(matches!(&emp.source, LogicalSource::Table(t) if t == "EMP"));
    assert!(matches!(&dept.source, LogicalSource::Query(q) if q == "SELECT deptno, dname FROM DEPT"));

    // Template subject defaults to an IRI term type; rr:class captured.
    match &emp.subject.term {
        TermMap::Template(tmpl, spec) => {
            assert_eq!(spec.term_type, TermType::Iri);
            assert_eq!(
                tmpl.segments(),
                Template::parse("http://example.com/emp/{empno}")
                    .unwrap()
                    .segments()
            );
        }
        other => panic!("expected a template subject, got {other:?}"),
    }
    assert_eq!(emp.subject.classes.len(), 1);
    assert_eq!(
        emp.subject.classes[0].as_str(),
        "http://xmlns.com/foaf/0.1/Person"
    );

    assert_eq!(emp.predicate_object_maps.len(), 2);

    // Column object: defaults to a plain literal (object position + rr:column).
    let name_pom = pom_with_predicate(emp, "http://xmlns.com/foaf/0.1/name");
    match &name_pom.objects[0] {
        ObjectMap::Term(TermMap::Column(col, spec)) => {
            assert_eq!(&**col, "ename");
            assert_eq!(spec.term_type, TermType::Literal);
            assert!(spec.datatype.is_none());
            assert!(spec.language.is_none());
        }
        other => panic!("expected a column object, got {other:?}"),
    }

    // Referencing object map: parent id resolves to exactly the Dept map's id.
    let dept_ref_pom = pom_with_predicate(emp, "http://example.com/department");
    match &dept_ref_pom.objects[0] {
        ObjectMap::Ref(r) => {
            assert_eq!(r.parent_triples_map, dept.id);
            assert_eq!(r.joins.len(), 1);
            assert_eq!(r.joins[0].child, "deptno");
            assert_eq!(r.joins[0].parent, "deptno");
        }
        other => panic!("expected a referencing object map, got {other:?}"),
    }

    // Explicit rr:datatype on a column object.
    let dname_pom = pom_with_predicate(dept, "http://example.com/name");
    match &dname_pom.objects[0] {
        ObjectMap::Term(TermMap::Column(col, spec)) => {
            assert_eq!(&**col, "dname");
            assert_eq!(spec.term_type, TermType::Literal);
            assert_eq!(
                spec.datatype.as_ref().map(NamedNode::as_str),
                Some("http://www.w3.org/2001/XMLSchema#string")
            );
        }
        other => panic!("expected a typed column object, got {other:?}"),
    }
}

#[test]
fn relative_triples_map_iris_resolve_against_the_base() {
    let maps = parse_r2rml(FIXTURE).unwrap();
    // <#TriplesMap_*> resolved to absolute IRIs under the default base.
    assert!(maps.iter().all(|m| m.id.starts_with("http://")));
}

#[test]
fn triples_map_without_logical_table_is_rejected() {
    let doc = r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <http://ex.org/tm> rr:subjectMap [ rr:template "http://ex.org/{id}" ] .
    "#;
    assert!(parse_r2rml(doc).is_err());
}

#[test]
fn malformed_turtle_is_a_mapping_error() {
    assert!(parse_r2rml("this is not turtle @@@").is_err());
}

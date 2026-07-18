//! Unit tests for the R2RML → IR parser. `use super::*` re-uses the parent
//! module's IR imports (`TriplesMap`, `TermMap`, `Term`, `NamedNode`, …) and the
//! public [`parse_r2rml`].

use super::*;
use sf_core::ir::Segment;

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
            pom.predicates.iter().any(
                |p| matches!(p, TermMap::Constant(Term::NamedNode(n)) if n.as_str() == predicate),
            )
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
    assert!(
        matches!(&dept.source, LogicalSource::Query(q) if q == "SELECT deptno, dname FROM DEPT")
    );

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

// --- ADR-0032 D1: rml:StarMap role-split id expansion (supersedes ADR-0029 §B) -

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_PROPOSITION_FORM: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#PropositionForm";
const RDF_PROPOSITION_FORM_SUBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormSubject";
const RDF_PROPOSITION_FORM_PREDICATE: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormPredicate";
const RDF_PROPOSITION_FORM_OBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormObject";
const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// ADR-0032 §A-equivalent example (asserted variant): `<#PersonAge>` is the
/// quoted triples map, `<#PersonAgeAssertion>`'s subject is the D1 reifier id.
/// `rml:quotedTriplesMap` only (no `rml:nonAssertedTriplesMap`) ⇒ asserted:
/// `<#PersonAge>` is also emitted as its own ordinary triples map.
const STAR_ASSERTED_FIXTURE: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#PersonAgeAssertion>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [
        rml:starMap [
            rml:quotedTriplesMap <#PersonAge>
        ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:CensusRecord2026 ]
    ] .
"#;

/// Same shape, but `rml:nonAssertedTriplesMap` also marks `<#PersonAge>`
/// non-asserted ⇒ it must be suppressed from `parse_r2rml`'s output (the
/// description-map carrier stays, unconditionally — D1, like v1's object
/// position before it).
const STAR_NON_ASSERTED_FIXTURE: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#PersonAgeAssertion>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [
        rml:starMap [
            rml:quotedTriplesMap <#PersonAge> ;
            rml:nonAssertedTriplesMap <#PersonAge>
        ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:CensusRecord2026 ]
    ] .
"#;

/// The standalone description-map carrier for a quoted shape (ADR-0032 D1: id
/// scheme `urn:sf-star:desc:{quoted_id}`) — shared by subject- and
/// object-position star maps and every nesting level, keyed on the quoted
/// map's own id (never the referencing outer map's), so two different outer
/// maps quoting the same shape resolve to the very same carrier.
fn description_map_for<'a>(maps: &'a [TriplesMap], quoted_id_suffix: &str) -> &'a TriplesMap {
    maps.iter()
        .find(|m| m.id.contains("urn:sf-star:desc:") && m.id.ends_with(quoted_id_suffix))
        .unwrap_or_else(|| panic!("no description map found for {quoted_id_suffix}"))
}

#[test]
fn star_map_subject_position_reifier_and_description_map() {
    let maps = parse_r2rml(STAR_ASSERTED_FIXTURE).expect("fixture parses");
    // Outer (reifier) map + asserted plain quoted map + standalone description map.
    assert_eq!(
        maps.len(),
        3,
        "outer + asserted quoted map + description map"
    );

    let outer = map_by_suffix(&maps, "#PersonAgeAssertion");
    let quoted = map_by_suffix(&maps, "#PersonAge");
    let desc = description_map_for(&maps, "#PersonAge");

    // The outer TM's subject is now the REIFIER id, not the proposition id.
    let reifier_tmpl = match &outer.subject.term {
        TermMap::Template(tmpl, spec) => {
            assert_eq!(spec.term_type, TermType::Iri);
            tmpl
        }
        other => panic!("expected a reifier template subject, got {other:?}"),
    };
    let reifier_str = format!("{reifier_tmpl:?}");
    assert!(
        reifier_str.contains("urn:sf-star:r:"),
        "reifier id should carry the urn:sf-star:r: family prefix: {reifier_str}"
    );

    // Exactly the author's own POM plus ONE injected rdf:reifies POM (not 4).
    assert_eq!(outer.predicate_object_maps.len(), 2);
    pom_with_predicate(outer, "http://example.com/assertedBy"); // annotation rides the reifier
    let reifies_pom = pom_with_predicate(outer, RDF_REIFIES);
    let pfid_via_reifies = match &reifies_pom.objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, spec)) => {
            assert_eq!(spec.term_type, TermType::Iri);
            let s = format!("{tmpl:?}");
            assert!(
                s.contains("urn:sf-star:pf:"),
                "rdf:reifies object must be the pf-family proposition id: {s}"
            );
            tmpl.clone()
        }
        other => panic!("expected rdf:reifies -> proposition id template, got {other:?}"),
    };

    // The description map's own subject IS that same proposition id — the
    // rdf:reifies POM and the description map's subject must be co-identical.
    match &desc.subject.term {
        TermMap::Template(tmpl, spec) => {
            assert_eq!(spec.term_type, TermType::Iri);
            assert_eq!(tmpl.segments(), pfid_via_reifies.segments());
        }
        other => panic!("expected a proposition-id template subject, got {other:?}"),
    }
    assert!(matches!(&desc.source, LogicalSource::Table(t) if t == "census_row"));

    // The 4 basic-encoding POMs live on the description map now, not the outer TM.
    assert_eq!(desc.predicate_object_maps.len(), 4);
    let type_pom = pom_with_predicate(desc, RDF_TYPE);
    match &type_pom.objects[0] {
        ObjectMap::Term(TermMap::Constant(Term::NamedNode(n))) => {
            assert_eq!(n.as_str(), RDF_PROPOSITION_FORM)
        }
        other => panic!("expected rdf:PropositionForm constant, got {other:?}"),
    }
    let subj_pom = pom_with_predicate(desc, RDF_PROPOSITION_FORM_SUBJECT);
    match &subj_pom.objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, _)) => {
            assert_eq!(
                tmpl.segments(),
                Template::parse("http://ex.org/person/{person_id}")
                    .unwrap()
                    .segments(),
                "propositionFormSubject must carry the quoted triple's own subject term"
            );
        }
        other => panic!("expected the quoted triple's subject template, got {other:?}"),
    }
    let pred_pom = pom_with_predicate(desc, RDF_PROPOSITION_FORM_PREDICATE);
    match &pred_pom.objects[0] {
        ObjectMap::Term(TermMap::Constant(Term::NamedNode(n))) => {
            assert_eq!(n.as_str(), "http://example.com/hasAge")
        }
        other => panic!("expected ex:hasAge constant, got {other:?}"),
    }
    let obj_pom = pom_with_predicate(desc, RDF_PROPOSITION_FORM_OBJECT);
    match &obj_pom.objects[0] {
        ObjectMap::Term(TermMap::Column(col, spec)) => {
            assert_eq!(&**col, "age");
            assert_eq!(spec.term_type, TermType::Literal);
        }
        other => panic!("expected the quoted triple's object column, got {other:?}"),
    }

    // The quoted triples map is also emitted plainly, matchable outside <<>>.
    assert!(matches!(&quoted.source, LogicalSource::Table(t) if t == "census_row"));
}

#[test]
fn star_map_non_asserted_keeps_description_map_suppresses_plain_quoted_map() {
    let maps = parse_r2rml(STAR_NON_ASSERTED_FIXTURE).expect("fixture parses");
    // Outer map + description map only; the plain quoted map is suppressed
    // (the description carrier, like v1's object-position carrier, is
    // unconditional — its id never matches a bare quoted_id).
    assert_eq!(maps.len(), 2, "quoted map suppressed when non-asserted");
    assert!(maps.iter().any(|m| m.id.ends_with("#PersonAgeAssertion")));
    assert!(!maps
        .iter()
        .any(|m| m.id.ends_with("#PersonAge") && !m.id.contains("urn:sf-star:desc:")));
    let desc = description_map_for(&maps, "#PersonAge");
    assert_eq!(desc.predicate_object_maps.len(), 4);
}

#[test]
fn star_map_ids_are_deterministic_across_parses() {
    let maps1 = parse_r2rml(STAR_ASSERTED_FIXTURE).expect("fixture parses (1st)");
    let maps2 = parse_r2rml(STAR_ASSERTED_FIXTURE).expect("fixture parses (2nd)");
    let outer1 = map_by_suffix(&maps1, "#PersonAgeAssertion");
    let outer2 = map_by_suffix(&maps2, "#PersonAgeAssertion");
    let (TermMap::Template(t1, _), TermMap::Template(t2, _)) =
        (&outer1.subject.term, &outer2.subject.term)
    else {
        panic!("expected template subjects on both parses");
    };
    assert_eq!(
        t1.segments(),
        t2.segments(),
        "same mapping must yield the same reifier template on repeated parses"
    );
    let desc1 = description_map_for(&maps1, "#PersonAge");
    let desc2 = description_map_for(&maps2, "#PersonAge");
    assert_eq!(
        desc1.id, desc2.id,
        "description-map carrier id must be stable across parses"
    );
}

/// D1's reifier multiplicity: two DIFFERENT outer maps quoting the SAME shape
/// mint DISTINCT reifiers (keyed on each outer map's own id) but resolve to
/// the SAME proposition — asserted here by comparing the two description
/// maps' subject templates (pf co-identity) while the reifier templates
/// differ.
const STAR_TWO_ASSERTIONS_SAME_SHAPE_FIXTURE: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#AssertionA>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#PersonAge> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:SourceA ] ] .

<#AssertionB>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#PersonAge> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:SourceB ] ] .
"#;

#[test]
fn star_map_distinct_reifiers_share_one_proposition() {
    let maps = parse_r2rml(STAR_TWO_ASSERTIONS_SAME_SHAPE_FIXTURE).expect("fixture parses");
    // #AssertionA + #AssertionB + #PersonAge (asserted plain, deduped: both
    // stars reference it) + ONE shared description map (deduped by quoted_id).
    assert_eq!(
        maps.len(),
        4,
        "two outer maps + one asserted plain quote + one shared description map"
    );

    let a = map_by_suffix(&maps, "#AssertionA");
    let b = map_by_suffix(&maps, "#AssertionB");

    let (TermMap::Template(reifier_a, _), TermMap::Template(reifier_b, _)) =
        (&a.subject.term, &b.subject.term)
    else {
        panic!("expected reifier template subjects on both outer maps");
    };
    assert_ne!(
        reifier_a.segments(),
        reifier_b.segments(),
        "distinct star-map declarations must mint distinct reifiers"
    );

    let pf_a = match &pom_with_predicate(a, RDF_REIFIES).objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, _)) => tmpl.clone(),
        other => panic!("expected rdf:reifies -> proposition id, got {other:?}"),
    };
    let pf_b = match &pom_with_predicate(b, RDF_REIFIES).objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, _)) => tmpl.clone(),
        other => panic!("expected rdf:reifies -> proposition id, got {other:?}"),
    };
    assert_eq!(
        pf_a.segments(),
        pf_b.segments(),
        "both reifiers must reify the SAME proposition id (co-identity)"
    );

    // Only ONE description map exists for the shared shape (dedup by quoted_id).
    let desc = description_map_for(&maps, "#PersonAge");
    match &desc.subject.term {
        TermMap::Template(tmpl, _) => assert_eq!(tmpl.segments(), pf_a.segments()),
        other => panic!("expected the proposition-id template subject, got {other:?}"),
    }
}

#[test]
fn rejects_nested_star_map_in_quoted_subject_citing_concepts_3_1() {
    // D5: the quoted triples map's own subject must never itself be a
    // StarMap (RDF 1.2 Concepts §3.1: a triple-term subject is not RDF).
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#Inner>
            rr:logicalTable [ rr:tableName "t" ] ;
            rr:subjectMap [ rr:template "http://ex.org/inner/{id}" ] ;
            rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .

        <#Quoted>
            rr:logicalTable [ rr:tableName "t" ] ;
            rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#Inner> ] ] ;
            rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column "w" ] ] .

        <#Outer>
            rr:logicalTable [ rr:tableName "t" ] ;
            rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#Quoted> ] ] ;
            rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:X ] ] .
    "#;
    let err = parse_r2rml(doc).expect_err("subject-side StarMap nesting must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(msg.contains("Concepts §3.1"), "unexpected message: {msg}");
}

#[test]
fn rejects_star_map_in_predicate_position_citing_concepts_3_1() {
    // R4/D5: rml:starMap must never appear under rr:predicateMap.
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#Bad>
            rr:logicalTable [ rr:tableName "t" ] ;
            rr:subjectMap [ rr:template "http://ex.org/{id}" ] ;
            rr:predicateObjectMap [
                rr:predicateMap [ rml:starMap [ rml:quotedTriplesMap <#Bad> ] ] ;
                rr:objectMap [ rr:column "v" ]
            ] .
    "#;
    let err = parse_r2rml(doc).expect_err("StarMap in predicate position must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(
        msg.contains("predicate position") && msg.contains("Concepts §3.1"),
        "unexpected message: {msg}"
    );
}

#[test]
fn rejects_non_single_spo_quoted_triples_map() {
    // The quoted triples map must have exactly one predicate-object map with
    // exactly one predicate and one object — unaffected by D1.
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#TwoPoms>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
            rr:predicateObjectMap [ rr:predicate ex:hasAge ; rr:objectMap [ rr:column "age" ] ] ;
            rr:predicateObjectMap [ rr:predicate ex:hasName ; rr:objectMap [ rr:column "name" ] ] .

        <#Outer>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#TwoPoms> ] ] ;
            rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:X ] ] .
    "#;
    let err = parse_r2rml(doc).expect_err("non-single-spo quoted triples map must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(msg.contains("non-single-spo"), "unexpected message: {msg}");
}

// --- ADR-0032 D1: rml:StarMap in OBJECT position (unchanged shape, new naming) -
//
// Object position still has no reifier — the object map's own enclosing
// triples map cannot host it (D1 §"Object-position star map"). Only the id
// family naming (pf: prefix) and the shared `urn:sf-star:desc:` carrier
// scheme change from v1.

const STAR_OBJECT_ASSERTED_FIXTURE: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#Assertion>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/assertion/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasQuote ;
        rr:objectMap [
            rml:starMap [
                rml:quotedTriplesMap <#PersonAge>
            ]
        ]
    ] .
"#;

const STAR_OBJECT_NON_ASSERTED_FIXTURE: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#Assertion>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/assertion/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasQuote ;
        rr:objectMap [
            rml:starMap [
                rml:quotedTriplesMap <#PersonAge> ;
                rml:nonAssertedTriplesMap <#PersonAge>
            ]
        ]
    ] .
"#;

#[test]
fn star_map_object_position_asserted_expansion_uses_pf_naming() {
    let maps = parse_r2rml(STAR_OBJECT_ASSERTED_FIXTURE).expect("fixture parses");
    assert_eq!(
        maps.len(),
        3,
        "outer + asserted quoted map + description carrier"
    );

    let outer = map_by_suffix(&maps, "#Assertion");
    let quoted = map_by_suffix(&maps, "#PersonAge");
    let desc = description_map_for(&maps, "#PersonAge");

    // The outer map's own subject is untouched (a plain template) — object
    // position never rewrites the outer subject, and there is no reifier.
    match &outer.subject.term {
        TermMap::Template(tmpl, _) => assert_eq!(
            tmpl.segments(),
            Template::parse("http://ex.org/assertion/{person_id}")
                .unwrap()
                .segments()
        ),
        other => panic!("expected the outer map's own template subject, got {other:?}"),
    }

    // Exactly the author's own predicate-object map — no rdf:reifies, no
    // basic-encoding POMs injected onto the outer map in object position.
    assert_eq!(outer.predicate_object_maps.len(), 1);
    let quote_pom = pom_with_predicate(outer, "http://example.com/hasQuote");
    let outer_object_tmpl = match &quote_pom.objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, spec)) => {
            assert_eq!(spec.term_type, TermType::Iri);
            let s = format!("{tmpl:?}");
            assert!(
                s.contains("urn:sf-star:pf:"),
                "outer object must be the pf-family proposition id: {s}"
            );
            tmpl.clone()
        }
        other => panic!("expected the proposition-id template as the object, got {other:?}"),
    };

    // The description carrier's subject is the SAME proposition id template.
    match &desc.subject.term {
        TermMap::Template(tmpl, spec) => {
            assert_eq!(spec.term_type, TermType::Iri);
            assert_eq!(tmpl.segments(), outer_object_tmpl.segments());
        }
        other => panic!("expected a proposition-id template subject, got {other:?}"),
    }
    assert!(matches!(&desc.source, LogicalSource::Table(t) if t == "census_row"));
    assert_eq!(desc.predicate_object_maps.len(), 4);
    pom_with_predicate(desc, RDF_TYPE);
    pom_with_predicate(desc, RDF_PROPOSITION_FORM_SUBJECT);
    pom_with_predicate(desc, RDF_PROPOSITION_FORM_PREDICATE);
    pom_with_predicate(desc, RDF_PROPOSITION_FORM_OBJECT);

    assert!(matches!(&quoted.source, LogicalSource::Table(t) if t == "census_row"));
}

#[test]
fn star_map_object_position_non_asserted_suppresses_plain_quoted_map() {
    let maps = parse_r2rml(STAR_OBJECT_NON_ASSERTED_FIXTURE).expect("fixture parses");
    assert_eq!(maps.len(), 2, "quoted map suppressed when non-asserted");
    assert!(maps.iter().any(|m| m.id.ends_with("#Assertion")));
    assert!(!maps
        .iter()
        .any(|m| m.id.ends_with("#PersonAge") && !m.id.contains("urn:sf-star:desc:")));
    let desc = description_map_for(&maps, "#PersonAge");
    assert_eq!(desc.predicate_object_maps.len(), 4);
}

#[test]
fn rejects_nested_star_map_in_object_position_citing_concepts_3_1() {
    // D5, object position: the quoted triples map's own SUBJECT (not object)
    // is itself an rml:starMap — still illegal at any nesting depth.
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#Inner>
            rr:logicalTable [ rr:tableName "t" ] ;
            rr:subjectMap [ rr:template "http://ex.org/inner/{id}" ] ;
            rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .

        <#Quoted>
            rr:logicalTable [ rr:tableName "t" ] ;
            rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#Inner> ] ] ;
            rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column "w" ] ] .

        <#Outer>
            rr:logicalTable [ rr:tableName "t" ] ;
            rr:subjectMap [ rr:template "http://ex.org/outer/{id}" ] ;
            rr:predicateObjectMap [
                rr:predicate ex:assertedBy ;
                rr:objectMap [ rml:starMap [ rml:quotedTriplesMap <#Quoted> ] ]
            ] .
    "#;
    let err = parse_r2rml(doc).expect_err("subject-side StarMap nesting must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(msg.contains("Concepts §3.1"), "unexpected message: {msg}");
}

// --- ADR-0032 D1 item 5: OBJECT-SIDE nesting (recursive, arbitrary depth) -----

/// Depth-2: `#Outer` (subject position) reifies `#Mid`'s shape; `#Mid`'s own
/// OBJECT carries a nested star map quoting `#Leaf`. Same source throughout.
const STAR_NESTED_DEPTH2_FIXTURE: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#Leaf>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/leaf/{person_id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:hasScore ; rr:objectMap [ rr:column "score" ] ] .

<#Mid>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rml:starMap [ rml:quotedTriplesMap <#Leaf> ] ]
    ] .

<#Outer>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#Mid> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:X ] ] .
"#;

#[test]
fn star_map_object_side_nesting_depth_2() {
    let maps = parse_r2rml(STAR_NESTED_DEPTH2_FIXTURE).expect("fixture parses");
    // #Outer + #Mid (asserted plain) + #Leaf (asserted plain) + desc(#Mid) + desc(#Leaf).
    assert_eq!(
        maps.len(),
        5,
        "outer + 2 asserted quotes + 2 description maps"
    );

    let outer = map_by_suffix(&maps, "#Outer");
    let desc_mid = description_map_for(&maps, "#Mid");
    let desc_leaf = description_map_for(&maps, "#Leaf");

    // #Outer reifies #Mid's proposition id.
    let outer_pf = match &pom_with_predicate(outer, RDF_REIFIES).objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, _)) => tmpl.clone(),
        other => panic!("expected rdf:reifies -> proposition id, got {other:?}"),
    };
    match &desc_mid.subject.term {
        TermMap::Template(tmpl, _) => assert_eq!(tmpl.segments(), outer_pf.segments()),
        other => panic!("expected proposition-id subject, got {other:?}"),
    }

    // desc(#Mid)'s propositionFormObject IS desc(#Leaf)'s own subject
    // (Template segment concatenation / co-identity through nesting).
    let mid_obj_pom = pom_with_predicate(desc_mid, RDF_PROPOSITION_FORM_OBJECT);
    let mid_obj_tmpl = match &mid_obj_pom.objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, spec)) => {
            assert_eq!(spec.term_type, TermType::Iri);
            tmpl.clone()
        }
        other => panic!("expected the inner proposition-id template, got {other:?}"),
    };
    match &desc_leaf.subject.term {
        TermMap::Template(tmpl, _) => assert_eq!(tmpl.segments(), mid_obj_tmpl.segments()),
        other => panic!("expected proposition-id subject, got {other:?}"),
    }

    // desc(#Leaf) carries the leaf shape's own basic encoding.
    let leaf_pred_pom = pom_with_predicate(desc_leaf, RDF_PROPOSITION_FORM_PREDICATE);
    match &leaf_pred_pom.objects[0] {
        ObjectMap::Term(TermMap::Constant(Term::NamedNode(n))) => {
            assert_eq!(n.as_str(), "http://example.com/hasScore")
        }
        other => panic!("expected ex:hasScore constant, got {other:?}"),
    }

    // Outer's own pf id (embedded in desc_mid's subject) must differ from the
    // inner leaf's pf id — the splice must not collapse the two shapes.
    assert_ne!(
        outer_pf.segments(),
        mid_obj_tmpl.segments(),
        "outer and inner proposition ids must be distinguishable"
    );
}

/// Depth-3: one further nesting level (`#Leaf`'s own object quotes `#Deepest`)
/// — proves the recursion is not hard-coded to depth 2.
const STAR_NESTED_DEPTH3_FIXTURE: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#Deepest>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/deepest/{person_id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:hasFlag ; rr:objectMap [ rr:column "flag" ] ] .

<#Leaf>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/leaf/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasScore ;
        rr:objectMap [ rml:starMap [ rml:quotedTriplesMap <#Deepest> ] ]
    ] .

<#Mid>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rml:starMap [ rml:quotedTriplesMap <#Leaf> ] ]
    ] .

<#Outer>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#Mid> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:X ] ] .
"#;

#[test]
fn star_map_object_side_nesting_depth_3() {
    let maps = parse_r2rml(STAR_NESTED_DEPTH3_FIXTURE).expect("fixture parses");
    // #Outer + #Mid + #Leaf + #Deepest (each asserted plain) + 3 description maps.
    assert_eq!(
        maps.len(),
        7,
        "outer + 3 asserted quotes + 3 description maps"
    );

    let outer = map_by_suffix(&maps, "#Outer");
    let desc_mid = description_map_for(&maps, "#Mid");
    let desc_leaf = description_map_for(&maps, "#Leaf");
    let desc_deepest = description_map_for(&maps, "#Deepest");

    let outer_pf = match &pom_with_predicate(outer, RDF_REIFIES).objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, _)) => tmpl.clone(),
        other => panic!("expected rdf:reifies -> proposition id, got {other:?}"),
    };
    assert_eq!(desc_mid_subject_segments(desc_mid), outer_pf.segments());

    let mid_obj = match &pom_with_predicate(desc_mid, RDF_PROPOSITION_FORM_OBJECT).objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, _)) => tmpl.clone(),
        other => panic!("expected the inner proposition-id template, got {other:?}"),
    };
    assert_eq!(desc_mid_subject_segments(desc_leaf), mid_obj.segments());

    let leaf_obj = match &pom_with_predicate(desc_leaf, RDF_PROPOSITION_FORM_OBJECT).objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, _)) => tmpl.clone(),
        other => panic!("expected the innermost proposition-id template, got {other:?}"),
    };
    assert_eq!(desc_mid_subject_segments(desc_deepest), leaf_obj.segments());

    // All three levels' ids must be pairwise distinct.
    assert_ne!(outer_pf.segments(), mid_obj.segments());
    assert_ne!(mid_obj.segments(), leaf_obj.segments());
    assert_ne!(outer_pf.segments(), leaf_obj.segments());
}

/// Small helper: a description map's own subject template's segments.
fn desc_mid_subject_segments(m: &TriplesMap) -> &[Segment] {
    match &m.subject.term {
        TermMap::Template(tmpl, _) => tmpl.segments(),
        other => panic!("expected a template subject, got {other:?}"),
    }
}

// --- ADR-0032 D1 item 4: rml:reifierMap override ------------------------------

#[test]
fn star_map_reifier_map_overrides_default_reifier_template() {
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#PersonAge>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
            rr:predicateObjectMap [ rr:predicate ex:hasAge ; rr:objectMap [ rr:column "age" ] ] .

        <#PersonAgeAssertion>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [
                rml:starMap [
                    rml:quotedTriplesMap <#PersonAge> ;
                    rml:reifierMap [ rr:template "http://ex.org/reifier/{person_id}" ]
                ]
            ] ;
            rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:X ] ] .
    "#;
    let maps = parse_r2rml(doc).expect("fixture parses");
    let outer = map_by_suffix(&maps, "#PersonAgeAssertion");
    match &outer.subject.term {
        TermMap::Template(tmpl, spec) => {
            assert_eq!(spec.term_type, TermType::Iri);
            assert_eq!(
                tmpl.segments(),
                Template::parse("http://ex.org/reifier/{person_id}")
                    .unwrap()
                    .segments(),
                "rml:reifierMap must replace the default deterministic reifier template"
            );
        }
        other => panic!("expected the author's own reifier template, got {other:?}"),
    }
}

#[test]
fn rejects_reifier_map_in_object_position() {
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#PersonAge>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
            rr:predicateObjectMap [ rr:predicate ex:hasAge ; rr:objectMap [ rr:column "age" ] ] .

        <#Assertion>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [ rr:template "http://ex.org/assertion/{person_id}" ] ;
            rr:predicateObjectMap [
                rr:predicate ex:hasQuote ;
                rr:objectMap [
                    rml:starMap [
                        rml:quotedTriplesMap <#PersonAge> ;
                        rml:reifierMap [ rr:constant ex:SomeReifier ]
                    ]
                ]
            ] .
    "#;
    let err = parse_r2rml(doc).expect_err("rml:reifierMap in object position must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(
        msg.contains("reifierMap") && msg.contains("object-position"),
        "unexpected message: {msg}"
    );
}

#[test]
fn rejects_reifier_map_of_literal_term_type() {
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#PersonAge>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
            rr:predicateObjectMap [ rr:predicate ex:hasAge ; rr:objectMap [ rr:column "age" ] ] .

        <#PersonAgeAssertion>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [
                rml:starMap [
                    rml:quotedTriplesMap <#PersonAge> ;
                    rml:reifierMap [ rr:constant "not an iri or blank node" ]
                ]
            ] ;
            rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:X ] ] .
    "#;
    let err = parse_r2rml(doc).expect_err("a literal-valued rml:reifierMap must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(
        msg.contains("IRI") && msg.contains("blank-node"),
        "unexpected message: {msg}"
    );
}

// --- ADR-0032 D4: cross-source star maps ---------------------------------------

#[test]
fn rejects_cross_source_star_map_without_join_condition() {
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#PersonAgeOtherTable>
            rr:logicalTable [ rr:tableName "other_table" ] ;
            rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
            rr:predicateObjectMap [ rr:predicate ex:hasAge ; rr:objectMap [ rr:column "age" ] ] .

        <#Outer>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#PersonAgeOtherTable> ] ] ;
            rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:X ] ] .
    "#;
    let err = parse_r2rml(doc)
        .expect_err("cross-source StarMap without rr:joinCondition must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(msg.contains("joinCondition"), "unexpected message: {msg}");
}

#[test]
fn rejects_cross_source_star_map_without_join_condition_in_object_position() {
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#PersonAgeOtherTable>
            rr:logicalTable [ rr:tableName "other_table" ] ;
            rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
            rr:predicateObjectMap [ rr:predicate ex:hasAge ; rr:objectMap [ rr:column "age" ] ] .

        <#Outer>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [ rr:template "http://ex.org/outer/{id}" ] ;
            rr:predicateObjectMap [
                rr:predicate ex:assertedBy ;
                rr:objectMap [ rml:starMap [ rml:quotedTriplesMap <#PersonAgeOtherTable> ] ]
            ] .
    "#;
    let err = parse_r2rml(doc)
        .expect_err("cross-source StarMap without rr:joinCondition must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(msg.contains("joinCondition"), "unexpected message: {msg}");
}

#[test]
fn cross_source_star_map_with_join_condition_compiles_to_ref_object_map() {
    let doc = r#"
        @prefix rr:  <http://www.w3.org/ns/r2rml#> .
        @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
        @prefix ex:  <http://example.com/> .

        <#PersonAgeOtherTable>
            rr:logicalTable [ rr:tableName "other_table" ] ;
            rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
            rr:predicateObjectMap [ rr:predicate ex:hasAge ; rr:objectMap [ rr:column "age" ] ] .

        <#Outer>
            rr:logicalTable [ rr:tableName "census_row" ] ;
            rr:subjectMap [
                rml:starMap [
                    rml:quotedTriplesMap <#PersonAgeOtherTable> ;
                    rr:joinCondition [ rr:child "person_id" ; rr:parent "person_id" ]
                ]
            ] ;
            rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:X ] ] .
    "#;
    let maps = parse_r2rml(doc).expect("cross-source StarMap with rr:joinCondition must parse");
    let outer = map_by_suffix(&maps, "#Outer");
    let desc = description_map_for(&maps, "#PersonAgeOtherTable");

    // The description map compiles on the QUOTED source, not the outer's.
    assert!(matches!(&desc.source, LogicalSource::Table(t) if t == "other_table"));

    // The rdf:reifies object is a RefObjectMap joined to the description map.
    let reifies_pom = pom_with_predicate(outer, RDF_REIFIES);
    match &reifies_pom.objects[0] {
        ObjectMap::Ref(r) => {
            assert_eq!(r.parent_triples_map, desc.id);
            assert_eq!(r.joins.len(), 1);
            assert_eq!(r.joins[0].child, "person_id");
            assert_eq!(r.joins[0].parent, "person_id");
        }
        other => {
            panic!("expected a RefObjectMap for the cross-source reifies object, got {other:?}")
        }
    }
}

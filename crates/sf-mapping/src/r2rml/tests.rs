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

// --- ADR-0029: rml:StarMap expansion -------------------------------------------

/// ADR-0029 §A's own example (asserted variant): `<#PersonAge>` is the quoted
/// triples map, `<#PersonAgeAssertion>`'s subject is `rml:starMap`-derived.
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
/// non-asserted ⇒ it must be suppressed from `parse_r2rml`'s output.
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

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_PROPOSITION_FORM: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#PropositionForm";
const RDF_PROPOSITION_FORM_SUBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormSubject";
const RDF_PROPOSITION_FORM_PREDICATE: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormPredicate";
const RDF_PROPOSITION_FORM_OBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormObject";

#[test]
fn star_map_asserted_expansion_injects_basic_encoding_and_keeps_quoted_plain_map() {
    let maps = parse_r2rml(STAR_ASSERTED_FIXTURE).expect("fixture parses");
    // Asserted: both the outer map and the quoted map (as its own plain
    // triples map) are present.
    assert_eq!(maps.len(), 2, "outer map + asserted quoted map");

    let outer = map_by_suffix(&maps, "#PersonAgeAssertion");
    let quoted = map_by_suffix(&maps, "#PersonAge");

    // Synthetic-id subject: a template, not the author's own subject syntax.
    let synthetic_tmpl = match &outer.subject.term {
        TermMap::Template(tmpl, spec) => {
            assert_eq!(spec.term_type, TermType::Iri);
            tmpl
        }
        other => panic!("expected a synthetic-id template subject, got {other:?}"),
    };
    let synth_str = format!("{synthetic_tmpl:?}");
    assert!(
        synth_str.contains("urn:sf-star:"),
        "synthetic id should carry the urn:sf-star: prefix: {synth_str}"
    );

    // Author's own POM plus the 4 injected basic-encoding POMs.
    assert_eq!(outer.predicate_object_maps.len(), 5);

    let type_pom = pom_with_predicate(outer, RDF_TYPE);
    match &type_pom.objects[0] {
        ObjectMap::Term(TermMap::Constant(Term::NamedNode(n))) => {
            assert_eq!(n.as_str(), RDF_PROPOSITION_FORM)
        }
        other => panic!("expected rdf:PropositionForm constant, got {other:?}"),
    }

    let subj_pom = pom_with_predicate(outer, RDF_PROPOSITION_FORM_SUBJECT);
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

    let pred_pom = pom_with_predicate(outer, RDF_PROPOSITION_FORM_PREDICATE);
    match &pred_pom.objects[0] {
        ObjectMap::Term(TermMap::Constant(Term::NamedNode(n))) => {
            assert_eq!(n.as_str(), "http://example.com/hasAge")
        }
        other => panic!("expected ex:hasAge constant, got {other:?}"),
    }

    let obj_pom = pom_with_predicate(outer, RDF_PROPOSITION_FORM_OBJECT);
    match &obj_pom.objects[0] {
        ObjectMap::Term(TermMap::Column(col, spec)) => {
            assert_eq!(&**col, "age");
            assert_eq!(spec.term_type, TermType::Literal);
        }
        other => panic!("expected the quoted triple's object column, got {other:?}"),
    }

    // The author's own predicate-object map (ex:assertedBy) is untouched.
    pom_with_predicate(outer, "http://example.com/assertedBy");

    // The quoted triples map is also emitted plainly, matchable outside <<>>.
    assert!(matches!(&quoted.source, LogicalSource::Table(t) if t == "census_row"));
}

#[test]
fn star_map_non_asserted_suppresses_the_quoted_plain_map() {
    let maps = parse_r2rml(STAR_NON_ASSERTED_FIXTURE).expect("fixture parses");
    // Only the outer (synthetic) map remains; the quoted map is suppressed.
    assert_eq!(maps.len(), 1, "quoted map suppressed when non-asserted");
    assert!(maps[0].id.ends_with("#PersonAgeAssertion"));
}

#[test]
fn star_map_synthetic_id_is_deterministic_across_parses() {
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
        "same mapping must yield the same synthetic-id template on repeated parses"
    );
}

#[test]
fn rejects_nested_star_map_in_quoted_subject() {
    // R3: the quoted triples map's own subject is itself an rml:starMap.
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
    let err = parse_r2rml(doc).expect_err("nested StarMap must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(
        msg.contains("nested") && msg.contains("rml:starMap"),
        "unexpected message: {msg}"
    );
}

#[test]
fn rejects_star_map_in_predicate_position() {
    // R4: rml:starMap must never appear under rr:predicateMap.
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
        msg.contains("predicate position"),
        "unexpected message: {msg}"
    );
}

#[test]
fn rejects_cross_source_star_map() {
    // The quoted triples map's logical source must equal the outer map's.
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
    let err = parse_r2rml(doc).expect_err("cross-source StarMap must be rejected");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(msg.contains("logical source"), "unexpected message: {msg}");
}

#[test]
fn rejects_non_single_spo_quoted_triples_map() {
    // The quoted triples map must have exactly one predicate-object map with
    // exactly one predicate and one object.
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

// --- ADR-0029 §B: rml:StarMap in OBJECT position -------------------------------
//
// Unlike subject position (where the outer TriplesMap's subject IS the
// synthetic id, so the 4 basic-encoding POMs inject directly onto the outer
// map), object position puts the synthetic id as the OBJECT of one of the
// outer map's triples. The 4 basic-encoding triples (whose SUBJECT is the
// synthetic id) cannot ride on the outer map, so they get a standalone
// synthetic TriplesMap instead.

/// Asserted variant: `<#Assertion>`'s single predicate-object map quotes
/// `<#PersonAge>` in object position via `rml:starMap`.
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

/// Same shape, but `rml:nonAssertedTriplesMap` also marks `<#PersonAge>`
/// non-asserted ⇒ it must be suppressed from `parse_r2rml`'s output (the
/// standalone basic-encoding carrier must still be present).
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

/// The standalone synthetic-carrier TriplesMap: neither `#Assertion` nor
/// `#PersonAge` — the map whose id carries the `urn:sf-star:objectmap:` marker.
fn standalone_star_map(maps: &[TriplesMap]) -> &TriplesMap {
    maps.iter()
        .find(|m| m.id.contains("urn:sf-star:objectmap:"))
        .unwrap_or_else(|| panic!("no standalone object-position StarMap carrier found"))
}

#[test]
fn star_map_object_position_asserted_expansion() {
    let maps = parse_r2rml(STAR_OBJECT_ASSERTED_FIXTURE).expect("fixture parses");
    // Outer map + asserted plain quoted map + standalone basic-encoding carrier.
    assert_eq!(
        maps.len(),
        3,
        "outer + asserted quoted map + standalone carrier"
    );

    let outer = map_by_suffix(&maps, "#Assertion");
    let quoted = map_by_suffix(&maps, "#PersonAge");
    let standalone = standalone_star_map(&maps);

    // The outer map's own subject is untouched (a plain template, not the
    // synthetic id) — object position never rewrites the outer subject.
    match &outer.subject.term {
        TermMap::Template(tmpl, _) => assert_eq!(
            tmpl.segments(),
            Template::parse("http://ex.org/assertion/{person_id}")
                .unwrap()
                .segments()
        ),
        other => panic!("expected the outer map's own template subject, got {other:?}"),
    }

    // Exactly the author's own predicate-object map — no basic-encoding POMs
    // are injected onto the outer map in object position.
    assert_eq!(outer.predicate_object_maps.len(), 1);
    let quote_pom = pom_with_predicate(outer, "http://example.com/hasQuote");
    let outer_object_tmpl = match &quote_pom.objects[0] {
        ObjectMap::Term(TermMap::Template(tmpl, spec)) => {
            assert_eq!(spec.term_type, TermType::Iri);
            let s = format!("{tmpl:?}");
            assert!(
                s.contains("urn:sf-star:"),
                "outer object must be the synthetic-id template: {s}"
            );
            tmpl.clone()
        }
        other => panic!("expected the synthetic-id template as the object, got {other:?}"),
    };

    // The standalone carrier's subject is the SAME synthetic-id template as
    // the outer map's object.
    match &standalone.subject.term {
        TermMap::Template(tmpl, spec) => {
            assert_eq!(spec.term_type, TermType::Iri);
            assert_eq!(tmpl.segments(), outer_object_tmpl.segments());
        }
        other => panic!("expected a synthetic-id template subject, got {other:?}"),
    }
    // Same logical source as the outer map (single-row v1, same cross-source rule).
    assert!(matches!(&standalone.source, LogicalSource::Table(t) if t == "census_row"));
    // Exactly the 4 basic-encoding predicate-object maps, no author POMs.
    assert_eq!(standalone.predicate_object_maps.len(), 4);
    pom_with_predicate(standalone, RDF_TYPE);
    pom_with_predicate(standalone, RDF_PROPOSITION_FORM_SUBJECT);
    pom_with_predicate(standalone, RDF_PROPOSITION_FORM_PREDICATE);
    pom_with_predicate(standalone, RDF_PROPOSITION_FORM_OBJECT);

    // The quoted triples map is also emitted plainly (asserted).
    assert!(matches!(&quoted.source, LogicalSource::Table(t) if t == "census_row"));
}

#[test]
fn star_map_object_position_non_asserted_suppresses_plain_quoted_map() {
    let maps = parse_r2rml(STAR_OBJECT_NON_ASSERTED_FIXTURE).expect("fixture parses");
    // Outer map + standalone carrier only; the plain quoted map is suppressed.
    assert_eq!(maps.len(), 2, "quoted map suppressed when non-asserted");
    assert!(maps.iter().any(|m| m.id.ends_with("#Assertion")));
    // The standalone carrier's id also *ends with* "#PersonAge" (it embeds
    // `quoted_id` verbatim, see `parse_object_map`'s id scheme) — exclude it
    // to check specifically for the (suppressed) plain quoted map.
    assert!(!maps
        .iter()
        .any(|m| m.id.ends_with("#PersonAge") && !m.id.contains("urn:sf-star:objectmap:")));
    let standalone = standalone_star_map(&maps);
    assert_eq!(standalone.predicate_object_maps.len(), 4);
}

#[test]
fn star_map_object_position_is_deterministic_across_parses() {
    let maps1 = parse_r2rml(STAR_OBJECT_ASSERTED_FIXTURE).expect("fixture parses (1st)");
    let maps2 = parse_r2rml(STAR_OBJECT_ASSERTED_FIXTURE).expect("fixture parses (2nd)");

    let mut ids1: Vec<&str> = maps1.iter().map(|m| m.id.as_str()).collect();
    let mut ids2: Vec<&str> = maps2.iter().map(|m| m.id.as_str()).collect();
    ids1.sort();
    ids2.sort();
    assert_eq!(ids1, ids2, "same set of triples-map ids on repeated parses");

    let standalone1 = standalone_star_map(&maps1);
    let standalone2 = standalone_star_map(&maps2);
    assert_eq!(
        standalone1.id, standalone2.id,
        "standalone carrier id must be stable across parses"
    );
    let (TermMap::Template(t1, _), TermMap::Template(t2, _)) =
        (&standalone1.subject.term, &standalone2.subject.term)
    else {
        panic!("expected template subjects on both parses");
    };
    assert_eq!(
        t1.segments(),
        t2.segments(),
        "same mapping must yield the same synthetic-id template on repeated parses"
    );
}

#[test]
fn rejects_nested_star_map_in_object_position() {
    // R3, object position: the quoted triples map's own subject is itself an
    // rml:starMap.
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
    let err = parse_r2rml(doc).expect_err("nested StarMap must be rejected in object position");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(
        msg.contains("nested") && msg.contains("rml:starMap"),
        "unexpected message: {msg}"
    );
}

#[test]
fn rejects_cross_source_star_map_in_object_position() {
    // The quoted triples map's logical source must equal the outer map's,
    // in object position exactly as in subject position.
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
    let err =
        parse_r2rml(doc).expect_err("cross-source StarMap must be rejected in object position");
    let Error::Mapping(msg) = err else {
        panic!("expected Error::Mapping, got {err:?}")
    };
    assert!(msg.contains("logical source"), "unexpected message: {msg}");
}

use std::sync::atomic::{AtomicU64, Ordering};

use sf_core::ir::{LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap};
use sf_core::ir::{SubjectMap, Template, TermMap, TermSpec, TriplesMap};
use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
use sf_core::query_control::{QueryControl, QueryControlError};
use sf_core::{Column, NamedNode, SourceId, SourceMapping, TableSchema, Term};

use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
use crate::{Backend, IntrospectedSource, SemanticOntology};

use super::{ConstantRole as Role, Coverage, MappingCoverage};

const FIXTURE: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.test/> .
<#a> a rr:TriplesMap;
  rr:logicalTable [ rr:tableName "items" ];
  rr:subjectMap [ rr:constant ex:s; rr:class ex:Class1; rr:graphMap [ rr:constant ex:g ] ];
  rr:predicateObjectMap [ rr:predicate ex:p1; rr:objectMap [ rr:constant ex:o1 ] ];
  rr:predicateObjectMap [ rr:predicate rdf:type; rr:objectMap [ rr:constant ex:Class2 ] ];
  rr:predicateObjectMap [ rr:predicate ex:p2; rr:objectMap [ rr:constant "5"^^xsd:integer ] ];
  rr:predicateObjectMap [ rr:predicate ex:p3; rr:objectMap [ rr:parentTriplesMap <#b> ] ].
<#b> a rr:TriplesMap;
  rr:logicalTable [ rr:tableName "items" ];
  rr:subjectMap [ rr:constant ex:parent ].
"#;

const ONTOLOGY: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix ex: <http://example.test/> .
ex:Item a owl:Class .
ex:value a rdf:Property .
"#;

const GATED: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#items> a rr:TriplesMap;
  rr:logicalTable [ rr:tableName "items" ];
  rr:subjectMap [ rr:constant ex:item; rr:class ex:Item ];
  rr:predicateObjectMap [ rr:predicate ex:value; rr:objectMap [ rr:constant ex:o ] ].
"#;

const CASES: &[(Role, &str, Coverage)] = &[
    (Role::Subject, "s", Coverage::Covered),
    (Role::Subject, "parent", Coverage::Covered),
    (Role::Subject, "p1", Coverage::Uncovered),
    (Role::Subject, "g", Coverage::Uncovered),
    (Role::Predicate, "p1", Coverage::Covered),
    (Role::Predicate, "p2", Coverage::Covered),
    (Role::Predicate, "rdf:type", Coverage::Covered),
    (Role::Predicate, "s", Coverage::Uncovered),
    (Role::Predicate, "o1", Coverage::Uncovered),
    (Role::Object, "o1", Coverage::Covered),
    (Role::Object, "Class1", Coverage::Covered),
    (Role::Object, "Class2", Coverage::Covered),
    (Role::Object, "parent", Coverage::Covered),
    (Role::Object, "s", Coverage::Uncovered),
    (Role::Object, "p1", Coverage::Uncovered),
    (Role::Class, "Class1", Coverage::Covered),
    (Role::Class, "Class2", Coverage::Covered),
    (Role::Class, "o1", Coverage::Uncovered),
    (Role::Class, "parent", Coverage::Uncovered),
    (Role::Class, "p1", Coverage::Uncovered),
    (Role::NamedGraph, "g", Coverage::Covered),
    (Role::NamedGraph, "s", Coverage::Uncovered),
    (Role::NamedGraph, "Class1", Coverage::Uncovered),
    (Role::NamedGraph, "rr:defaultGraph", Coverage::Uncovered),
    (Role::LiteralDatatype, "xsd:integer", Coverage::Covered),
    (Role::LiteralDatatype, "xsd:string", Coverage::Uncovered),
    (Role::LiteralDatatype, "o1", Coverage::Uncovered),
    (Role::Unresolved, "g", Coverage::Covered),
    (Role::Unresolved, "s", Coverage::Covered),
    (Role::Unresolved, "xsd:integer", Coverage::Covered),
    (Role::Unresolved, "nowhere", Coverage::Indeterminate),
];

const OPAQUE: &[(Role, &str, Coverage)] = &[
    (Role::Subject, "x", Coverage::Indeterminate),
    (Role::Subject, "known", Coverage::Covered),
    (Role::Predicate, "x", Coverage::Indeterminate),
    (Role::Predicate, "kp", Coverage::Covered),
    (Role::Object, "x", Coverage::Indeterminate),
    (Role::Object, "ko", Coverage::Covered),
    (Role::Class, "x", Coverage::Indeterminate),
    (Role::NamedGraph, "x", Coverage::Indeterminate),
    (
        Role::LiteralDatatype,
        "xsd:integer",
        Coverage::Indeterminate,
    ),
    (Role::Unresolved, "x", Coverage::Indeterminate),
    (Role::Unresolved, "ko", Coverage::Covered),
];

const CHARGED: &[(Role, &str)] = &[
    (Role::Subject, "missing"),
    (Role::Predicate, "missing"),
    (Role::Object, "missing"),
    (Role::Class, "missing"),
    (Role::NamedGraph, "g"),
    (Role::LiteralDatatype, "missing"),
    (Role::Unresolved, "missing"),
];

fn full(name: &str) -> String {
    match name {
        "rdf:type" => "http://www.w3.org/1999/02/22-rdf-syntax-ns#type".to_owned(),
        "xsd:integer" => "http://www.w3.org/2001/XMLSchema#integer".to_owned(),
        "xsd:string" => "http://www.w3.org/2001/XMLSchema#string".to_owned(),
        "rr:defaultGraph" => "http://www.w3.org/ns/r2rml#defaultGraph".to_owned(),
        _ => format!("http://example.test/{name}"),
    }
}

fn parse(text: &str) -> SourceMapping {
    let source = SourceId::new(0).unwrap();
    let parsed = sf_mapping::parse_r2rml_for_source(text, source);
    parsed.unwrap()
}

fn unlimited() -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX))
}

fn limited(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
}

fn classify(mapping: &SourceMapping, role: Role, name: &str) -> Coverage {
    let coverage = MappingCoverage::new(mapping);
    let result = coverage.classify(role, &full(name), &unlimited());
    result.unwrap()
}

fn work(mapping: &SourceMapping, role: Role, name: &str) -> u64 {
    let budget = unlimited();
    let coverage = MappingCoverage::new(mapping);
    let result = coverage.classify(role, &full(name), &budget);
    result.unwrap();
    budget.consumed(QueryCharge::CompilerWork)
}

fn constant(iri: &str) -> TermMap {
    TermMap::Constant(Term::NamedNode(NamedNode::new_unchecked(iri)))
}

fn object(term: TermMap) -> ObjectMap {
    ObjectMap::Term(term)
}

fn reference(parent: &str) -> ObjectMap {
    ObjectMap::Ref(RefObjectMap {
        parent_triples_map: parent.to_owned(),
        joins: Vec::new(),
    })
}

fn pom(predicate: TermMap, object: ObjectMap, graphs: Vec<TermMap>) -> PredicateObjectMap {
    PredicateObjectMap {
        predicates: vec![predicate],
        objects: vec![object],
        graphs,
    }
}

fn triples_map(id: &str, subject: TermMap, poms: Vec<PredicateObjectMap>) -> TriplesMap {
    TriplesMap {
        id: id.to_owned(),
        source: LogicalSource::Table("items".to_owned()),
        subject: SubjectMap {
            term: subject,
            classes: Vec::new(),
            graphs: Vec::new(),
        },
        predicate_object_maps: poms,
    }
}

fn bundle(maps: Vec<TriplesMap>) -> SourceMapping {
    SourceMapping::new(SourceId::new(0).unwrap(), maps)
}

fn opaque_bundle() -> SourceMapping {
    let template = Template::parse("http://example.test/{id}").unwrap();
    let column = TermMap::Column("c".into(), TermSpec::iri());
    let literal = TermMap::Column("v".into(), TermSpec::plain_literal());
    let link = pom(column.clone(), object(literal), Vec::new());
    let subject = TermMap::Template(template, TermSpec::iri());
    let mut opaque = triples_map("urn:opaque", subject, vec![link]);
    opaque.subject.graphs.push(column);
    let known_pom = pom(
        constant(&full("kp")),
        object(constant(&full("ko"))),
        Vec::new(),
    );
    let known = triples_map("urn:known", constant(&full("known")), vec![known_pom]);
    bundle(vec![opaque, known])
}

struct CancelAfter {
    budget: QueryBudget,
    remaining: AtomicU64,
}

impl QueryControl for CancelAfter {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.budget.checkpoint()
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        if self.remaining.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.budget.terminate(QueryControlError::Cancelled);
        }
        self.budget.consume(charge, amount)
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

#[test]
fn roles_match_miss_and_never_borrow_from_unrelated_roles() {
    let mapping = parse(FIXTURE);
    for (role, name, expected) in CASES {
        let actual = classify(&mapping, *role, name);
        assert_eq!(actual, *expected, "{role:?} {name}");
    }
}

#[test]
fn default_graph_sentinel_is_never_a_named_graph() {
    let mut sentinel = triples_map("urn:m", constant(&full("s")), Vec::new());
    sentinel
        .subject
        .graphs
        .push(constant(&full("rr:defaultGraph")));
    let mut named = triples_map("urn:m", constant(&full("s")), Vec::new());
    named.subject.graphs.push(constant(&full("g")));
    let plain = triples_map("urn:m", constant(&full("s")), Vec::new());
    let sentinel = bundle(vec![sentinel]);
    let named = bundle(vec![named]);
    let plain = bundle(vec![plain]);
    let cases = [
        (&sentinel, "rr:defaultGraph", Coverage::Uncovered),
        (&sentinel, "g", Coverage::Uncovered),
        (&named, "g", Coverage::Covered),
        (&named, "rr:defaultGraph", Coverage::Uncovered),
        (&plain, "g", Coverage::Uncovered),
        (&plain, "rr:defaultGraph", Coverage::Uncovered),
    ];
    for (mapping, name, expected) in cases {
        let actual = classify(mapping, Role::NamedGraph, name);
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn unsupported_producers_stay_indeterminate_unless_a_match_proves_coverage() {
    let mapping = opaque_bundle();
    for (role, name, expected) in OPAQUE {
        let actual = classify(&mapping, *role, name);
        assert_eq!(actual, *expected, "{role:?} {name}");
    }
}

#[test]
fn missing_or_ambiguous_parent_is_indeterminate() {
    let link = pom(constant(&full("p")), reference("urn:missing"), Vec::new());
    let child = triples_map("urn:child", constant(&full("s")), vec![link]);
    let missing = bundle(vec![child]);

    let link = pom(constant(&full("p")), reference("urn:dup"), Vec::new());
    let child = triples_map("urn:child", constant(&full("s")), vec![link]);
    let first = triples_map("urn:dup", constant(&full("d1")), Vec::new());
    let second = triples_map("urn:dup", constant(&full("d2")), Vec::new());
    let ambiguous = bundle(vec![child, first, second]);

    let cases = [
        (&missing, Role::Object, "x", Coverage::Indeterminate),
        (&missing, Role::Subject, "x", Coverage::Uncovered),
        (&missing, Role::Predicate, "p", Coverage::Covered),
        (&ambiguous, Role::Object, "d1", Coverage::Indeterminate),
        (&ambiguous, Role::Subject, "d1", Coverage::Covered),
        (&ambiguous, Role::Class, "d1", Coverage::Uncovered),
    ];
    for (mapping, role, name, expected) in cases {
        let actual = classify(mapping, role, name);
        assert_eq!(actual, expected, "{role:?} {name}");
    }
}

#[test]
fn compiler_work_is_charged_exactly_before_each_comparison() {
    let mapping = parse(FIXTURE);
    let coverage = MappingCoverage::new(&mapping);
    for (role, name) in CHARGED {
        let need = work(&mapping, *role, name);
        assert!(need > 0);

        let exact = limited(need);
        let outcome = coverage.classify(*role, &full(name), &exact);
        assert!(outcome.is_ok());
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), need);

        let short = limited(need - 1);
        let outcome = coverage.classify(*role, &full(name), &short);
        assert_eq!(outcome, Err(QueryControlError::CompilerWorkExceeded));
        assert!(short.consumed(QueryCharge::CompilerWork) < need);
    }
}

#[test]
fn cancellation_and_deadline_are_sticky_before_and_during_the_scan() {
    let mapping = parse(FIXTURE);
    let coverage = MappingCoverage::new(&mapping);
    let name = full("secret-token");
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let budget = unlimited();
        budget.terminate(reason);
        let outcome = coverage.classify(Role::Unresolved, &name, &budget);
        assert_eq!(outcome, Err(reason));
        assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
    }

    let control = CancelAfter {
        budget: unlimited(),
        remaining: AtomicU64::new(3),
    };
    let outcome = coverage.classify(Role::Unresolved, &name, &control);
    assert_eq!(outcome, Err(QueryControlError::Cancelled));
    let spent = control.budget.consumed(QueryCharge::CompilerWork);
    assert!(spent > 0);
    assert!(spent < work(&mapping, Role::Unresolved, "secret-token"));
    assert_eq!(control.checkpoint(), Err(QueryControlError::Cancelled));
    let first = control.terminate(QueryControlError::DeadlineExceeded);
    assert_eq!(first, QueryControlError::Cancelled);
}

#[test]
fn errors_and_debug_output_never_echo_mapping_or_query_constants() {
    let mapping = parse(FIXTURE);
    let coverage = MappingCoverage::new(&mapping);
    let name = full("secret-token");
    let budget = limited(1);
    let result = coverage.classify(Role::Unresolved, &name, &budget);
    let error = result.unwrap_err();
    let rendered = format!("{error} {error:?} {coverage:?}");
    assert!(!rendered.contains("secret-token"));
    assert!(!rendered.contains("example.test"));
    assert_eq!(format!("{:?}", Coverage::Indeterminate), "Indeterminate");
}

#[test]
fn gate_created_receipt_exposes_structural_coverage_without_row_claims() {
    let ontology = SemanticOntology::from_turtle(ONTOLOGY).unwrap();
    let mut table = TableSchema::new("items");
    table.columns = vec![Column::new("value", "text", false)];
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    let source = IntrospectedSource::unchecked(Backend::sqlite(connection), vec![table]);
    let mapping = parse(GATED);
    let origin = MappingOrigin::Authored;
    let validated = ValidatedMapping::validate(mapping, origin, &ontology, &source);
    let receipt = validated.unwrap();

    let coverage = receipt.coverage();
    let budget = unlimited();
    let checks = [
        (Role::Subject, "item", Coverage::Covered),
        (Role::Class, "Item", Coverage::Covered),
        (Role::Predicate, "rdf:type", Coverage::Covered),
        (Role::Predicate, "value", Coverage::Covered),
        (Role::Object, "Item", Coverage::Covered),
        (Role::Object, "o", Coverage::Covered),
        (Role::Class, "o", Coverage::Uncovered),
        (Role::Subject, "value", Coverage::Uncovered),
    ];
    for (role, name, expected) in checks {
        let outcome = coverage.classify(role, &full(name), &budget);
        assert_eq!(outcome, Ok(expected), "{role:?} {name}");
    }
}

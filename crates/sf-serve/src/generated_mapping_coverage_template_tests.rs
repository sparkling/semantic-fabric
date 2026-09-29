use std::sync::atomic::{AtomicU64, Ordering};

use sf_core::ir::encoding::percent_encode_iri;
use sf_core::ir::{Segment, Template, TermMap, TermSpec};
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControl, QueryControlError};
use sf_core::{Column, NamedNode, SourceMapping, TableSchema};

use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
use crate::{Backend, IntrospectedSource, SemanticOntology};

use super::templates::passes_through;
use super::tests::{bundle, constant, limited, object, opaque_bundle_with};
use super::tests::{pom, reference, triples_map, unlimited};
use super::{ConstantRole as Role, Coverage, MappingCoverage};

use super::template_cases::*;

fn at(suffix: &str) -> String {
    format!("{EX}{suffix}")
}

fn tmpl(text: &str, spec: TermSpec) -> TermMap {
    TermMap::Template(Template::parse(text).unwrap(), spec)
}

fn seg_tmpl(parts: Vec<Segment>) -> TermMap {
    TermMap::Template(Template::from_segments(parts).unwrap(), TermSpec::iri())
}

fn lit(text: &str) -> Segment {
    Segment::Literal(text.into())
}

fn col(name: &str) -> Segment {
    Segment::Column(name.into())
}

fn subject_only(term: TermMap) -> SourceMapping {
    bundle(vec![triples_map("urn:t", term, Vec::new())])
}

fn one_pom(predicate: TermMap, obj: TermMap) -> SourceMapping {
    let link = pom(predicate, object(obj), Vec::new());
    bundle(vec![triples_map("urn:t", constant(&at("s")), vec![link])])
}

fn parent_bundle(predicate: &str, parent_spec: TermSpec) -> SourceMapping {
    let link = pom(constant(predicate), reference("urn:parent"), Vec::new());
    let child = triples_map("urn:child", constant(&at("c")), vec![link]);
    let subject = tmpl("http://example.test/parent/{id}", parent_spec);
    bundle(vec![child, triples_map("urn:parent", subject, Vec::new())])
}

fn run(mapping: &SourceMapping, role: Role, query: &str) -> Coverage {
    let coverage = MappingCoverage::new(mapping);
    coverage.classify(role, query, &unlimited()).unwrap()
}

fn expect(mapping: &SourceMapping, role: Role, table: Table) {
    for (suffix, expected) in table {
        let actual = run(mapping, role, &at(suffix));
        assert_eq!(actual, *expected, "{role:?} {suffix:?}");
    }
}

fn roles(mapping: &SourceMapping, query: &str, table: RoleTable) {
    for (role, expected) in table {
        assert_eq!(run(mapping, *role, query), *expected, "{role:?}");
    }
}

fn work(mapping: &SourceMapping, role: Role, query: &str) -> u64 {
    let budget = unlimited();
    let coverage = MappingCoverage::new(mapping);
    coverage.classify(role, query, &budget).unwrap();
    budget.consumed(QueryCharge::CompilerWork)
}

fn encoded(value: &str) -> String {
    let mut out = String::new();
    percent_encode_iri(value, &mut out);
    out
}

fn decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn short_strings() -> Vec<String> {
    let mut all = vec![String::new()];
    let mut frontier = all.clone();
    for _ in 0..4 {
        let mut next = Vec::new();
        for base in &frontier {
            for piece in ALPHABET.chars() {
                next.push(format!("{base}{piece}"));
            }
        }
        all.extend(next.iter().cloned());
        frontier = next;
    }
    all
}

fn late_segments() -> (SourceMapping, String) {
    let mut parts = vec![lit(EX)];
    parts.extend((0..40).map(|_| lit("a")));
    parts.push(col("id"));
    parts.extend((0..40).map(|_| lit("b")));
    let query = format!("{EX}{}7{}", "a".repeat(40), "b".repeat(40));
    (subject_only(seg_tmpl(parts)), query)
}

struct Trip {
    budget: QueryBudget,
    calls: AtomicU64,
    at: u64,
    reason: QueryControlError,
}

impl Trip {
    fn new(at: u64, reason: QueryControlError) -> Self {
        Self {
            budget: unlimited(),
            calls: AtomicU64::new(0),
            at,
            reason,
        }
    }
}

impl QueryControl for Trip {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.budget.checkpoint()
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
            self.budget.terminate(self.reason);
        }
        self.budget.consume(charge, amount)
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

#[test]
fn single_slot_recipes_match_disjoint_and_empty_middle() {
    let bare = subject_only(tmpl("http://example.test/{id}", TermSpec::iri()));
    expect(&bare, Role::Subject, BARE);
    for query in OTHER_HOSTS {
        assert_eq!(run(&bare, Role::Subject, query), NO, "{query}");
    }
    let bounded = subject_only(tmpl("http://example.test/{id}/end", TermSpec::iri()));
    expect(&bounded, Role::Subject, PREFIX_SUFFIX);
    let overlap = subject_only(tmpl("ab{x}ba", TermSpec::iri()));
    assert_eq!(run(&overlap, Role::Subject, "aba"), NO);
    assert_eq!(run(&overlap, Role::Subject, "abba"), YES);
    assert_eq!(run(&overlap, Role::Subject, "abxba"), YES);
}

#[test]
fn literal_only_and_split_literal_recipes_are_exact() {
    let fixed = subject_only(tmpl("http://example.test/fixed", TermSpec::iri()));
    expect(&fixed, Role::Subject, FIXED);
    let braces = subject_only(tmpl("http://example.test/a\\{b\\}", TermSpec::iri()));
    assert_eq!(run(&braces, Role::Subject, &at("a{b}")), YES);
    assert_eq!(run(&braces, Role::Subject, &at("a%7Bb%7D")), NO);
    let parts = vec![lit(EX), lit("p-"), col("id"), lit("-s"), lit("")];
    expect(&subject_only(seg_tmpl(parts)), Role::Subject, SPLIT);
}

#[test]
fn unicode_and_percent_middles_follow_the_actual_encoder() {
    let bare = subject_only(tmpl("http://example.test/{id}", TermSpec::iri()));
    expect(&bare, Role::Subject, ENCODING);
    let literal = subject_only(tmpl("http://example.test/\u{e9}/{id}", TermSpec::iri()));
    expect(&literal, Role::Subject, UNICODE_LITERAL);
}

#[test]
fn pass_through_predicate_matches_the_encoder_for_every_scalar() {
    let mut out = String::new();
    for code in 0..=0x10FFFF_u32 {
        let Some(ch) = char::from_u32(code) else {
            continue;
        };
        out.clear();
        percent_encode_iri(ch.encode_utf8(&mut [0; 4]), &mut out);
        let raw = out.chars().eq(std::iter::once(ch));
        assert_eq!(passes_through(ch), raw, "U+{code:04X}");
    }
}

#[test]
fn every_encoder_output_matches_and_raw_pass_through_stays_raw() {
    let mapping = subject_only(tmpl("http://example.test/{id}", TermSpec::iri()));
    for sample in SAMPLES_A.split('|').chain(SAMPLES_B.split('|')) {
        let query = at(&encoded(sample));
        assert_eq!(run(&mapping, Role::Subject, &query), YES, "{sample:?}");
    }
    assert_eq!(encoded("\u{e9}"), "\u{e9}");
    assert_eq!(encoded("\u{e000}"), "%EE%80%80");
    assert_eq!(encoded("%"), "%25");
}

#[test]
fn coverage_equals_the_encoder_image_for_every_short_middle() {
    let mapping = subject_only(tmpl("http://example.test/{id}", TermSpec::iri()));
    for middle in short_strings() {
        let in_image = decoded(&middle).is_some_and(|value| encoded(&value) == middle);
        let expected = if in_image { YES } else { MAYBE };
        let actual = run(&mapping, Role::Subject, &at(&middle));
        assert_eq!(actual, expected, "{middle:?}");
    }
}

#[test]
fn unsupported_forms_stay_indeterminate_unless_proven_impossible() {
    let relative = TermSpec::iri().with_base("http://base.test/");
    let based = subject_only(tmpl("http://example.test/{id}", relative));
    assert_eq!(run(&based, Role::Subject, &at("abc")), MAYBE);
    assert_eq!(run(&based, Role::Subject, "http://other.test/x"), MAYBE);

    let column = subject_only(TermMap::Column("c".into(), TermSpec::iri()));
    assert_eq!(run(&column, Role::Subject, &at("x")), MAYBE);

    let multi = subject_only(tmpl("http://example.test/{a}/{b}", TermSpec::iri()));
    expect(&multi, Role::Subject, MULTI);
    assert_eq!(run(&multi, Role::Subject, "http://other.test/1/2"), NO);
    let trailing = subject_only(tmpl("{a}-{b}/tail", TermSpec::iri()));
    assert_eq!(run(&trailing, Role::Subject, "http://x/1-2/tail"), MAYBE);
    assert_eq!(run(&trailing, Role::Subject, "http://x/1-2/nope"), NO);

    let whole = subject_only(tmpl("{id}", TermSpec::iri()));
    assert_eq!(run(&whole, Role::Subject, &at("x")), MAYBE);
}

#[test]
fn term_kind_is_exact_for_matching_text() {
    let blank = subject_only(tmpl("http://example.test/{id}", TermSpec::blank_node()));
    assert_eq!(run(&blank, Role::Subject, &at("abc")), NO);
    let literal = tmpl("http://example.test/{id}", TermSpec::plain_literal());
    let as_object = one_pom(constant(&at("p")), literal.clone());
    assert_eq!(run(&as_object, Role::Object, &at("abc")), NO);
    let as_predicate = one_pom(literal, constant(&at("o")));
    assert_eq!(run(&as_predicate, Role::Predicate, &at("abc")), NO);
}

#[test]
fn a_template_covers_only_the_role_that_uses_it() {
    let subject = subject_only(tmpl("http://example.test/s/{id}", TermSpec::iri()));
    roles(&subject, &at("s/1"), SUBJECT_ROLES);
    let predicate = tmpl("http://example.test/p/{n}", TermSpec::iri());
    let mapping = one_pom(predicate, constant(&at("o")));
    roles(&mapping, &at("p/1"), PREDICATE_ROLES);
}

#[test]
fn type_capable_predicate_templates_keep_class_uncertain() {
    let predicate = tmpl(
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#{n}",
        TermSpec::iri(),
    );
    let mapping = one_pom(predicate, constant(&at("K")));
    assert_eq!(run(&mapping, Role::Predicate, RDF_TYPE), YES);
    assert_eq!(run(&mapping, Role::Class, &at("K")), MAYBE);
    assert_eq!(run(&mapping, Role::Object, &at("K")), YES);
    assert_eq!(run(&mapping, Role::Class, &at("Z")), NO);
}

#[test]
fn graph_templates_cover_named_graphs_but_never_the_default_sentinel() {
    let mut map = triples_map("urn:t", constant(&at("s")), Vec::new());
    map.subject
        .graphs
        .push(tmpl("http://example.test/graph/{g}", TermSpec::iri()));
    let mapping = bundle(vec![map]);
    assert_eq!(run(&mapping, Role::NamedGraph, &at("graph/1")), YES);
    assert_eq!(run(&mapping, Role::NamedGraph, &at("other/1")), NO);
    assert_eq!(run(&mapping, Role::Subject, &at("graph/1")), NO);
    assert_eq!(run(&mapping, Role::NamedGraph, DEFAULT_GRAPH), NO);
}

#[test]
fn parent_subject_templates_follow_the_referencing_role() {
    let plain = parent_bundle(&at("p"), TermSpec::iri());
    assert_eq!(run(&plain, Role::Object, &at("parent/5")), YES);
    assert_eq!(run(&plain, Role::Object, &at("other/5")), NO);
    assert_eq!(run(&plain, Role::Class, &at("parent/5")), NO);
    assert_eq!(run(&plain, Role::Subject, &at("parent/5")), YES);

    let typed = parent_bundle(RDF_TYPE, TermSpec::iri());
    assert_eq!(run(&typed, Role::Class, &at("parent/5")), YES);
    assert_eq!(run(&typed, Role::Class, &at("other/5")), NO);

    let relative = TermSpec::iri().with_base("http://base.test/");
    let late = parent_bundle(RDF_TYPE, relative);
    assert_eq!(run(&late, Role::Object, &at("parent/5")), MAYBE);
    assert_eq!(run(&late, Role::Class, &at("parent/5")), MAYBE);
}

#[test]
fn original_base_free_opaque_recipe_is_now_covered_while_columns_stay_uncertain() {
    let mapping = opaque_bundle_with(TermSpec::iri());
    let cases = [
        (Role::Subject, "x", YES),
        (Role::Subject, "known", YES),
        (Role::Predicate, "x", MAYBE),
        (Role::Object, "x", MAYBE),
        (Role::Class, "x", MAYBE),
        (Role::NamedGraph, "x", MAYBE),
        (Role::Unresolved, "x", YES),
    ];
    for (role, name, expected) in cases {
        let actual = run(&mapping, role, &at(name));
        assert_eq!(actual, expected, "{role:?} {name}");
    }
}

#[test]
fn gate_receipt_exposes_static_template_coverage_without_row_claims() {
    let ontology = SemanticOntology::from_turtle(ONTOLOGY).unwrap();
    let mut table = TableSchema::new("items");
    let id = Column::new("id", "text", false);
    table.columns = vec![id, Column::new("value", "text", false)];
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    let source = IntrospectedSource::unchecked(Backend::sqlite(connection), vec![table]);

    let link = pom(
        constant(&at("value")),
        object(constant(&at("o"))),
        Vec::new(),
    );
    let subject = tmpl("http://example.test/item/{id}", TermSpec::iri());
    let mut map = triples_map("http://example.test/map/items", subject, vec![link]);
    map.subject
        .classes
        .push(NamedNode::new_unchecked(at("Item")));
    let mapping = bundle(vec![map]);
    let origin = MappingOrigin::Authored;
    let receipt = ValidatedMapping::validate(mapping, origin, &ontology, &source).unwrap();

    let coverage = receipt.coverage();
    let budget = unlimited();
    for (suffix, expected) in GATED {
        let outcome = coverage.classify(Role::Subject, &at(suffix), &budget);
        assert_eq!(outcome, Ok(*expected), "{suffix}");
    }
    let checks = [
        (Role::Class, at("Item"), YES),
        (Role::Class, at("item/42"), NO),
        (Role::Predicate, at("value"), YES),
        (Role::Predicate, RDF_TYPE.to_owned(), YES),
        (Role::Predicate, at("item/42"), NO),
        (Role::Object, at("Item"), YES),
        (Role::Object, at("item/42"), NO),
    ];
    for (role, query, expected) in checks {
        let outcome = coverage.classify(role, &query, &budget);
        assert_eq!(outcome, Ok(expected), "{role:?} {query}");
    }
}

#[test]
fn template_work_is_charged_exactly_and_fails_at_n_minus_one() {
    for (text, query) in CHARGED {
        let mapping = subject_only(tmpl(text, TermSpec::iri()));
        let coverage = MappingCoverage::new(&mapping);
        for role in [Role::Subject, Role::Unresolved] {
            let need = work(&mapping, role, query);
            assert!(need > 1, "{text} {query}");
            let exact = limited(need);
            assert!(coverage.classify(role, query, &exact).is_ok());
            assert_eq!(exact.consumed(QueryCharge::CompilerWork), need);
            let short = limited(need - 1);
            let outcome = coverage.classify(role, query, &short);
            assert_eq!(outcome, Err(QueryControlError::CompilerWorkExceeded));
            assert!(short.consumed(QueryCharge::CompilerWork) < need);
            assert_eq!(
                short.checkpoint(),
                Err(QueryControlError::CompilerWorkExceeded)
            );
        }
    }
}

#[test]
fn large_inputs_are_charged_before_they_are_scanned() {
    let mapping = subject_only(tmpl("http://example.test/{id}", TermSpec::iri()));
    let coverage = MappingCoverage::new(&mapping);
    let big = at(&"a".repeat(200_000));
    assert!(work(&mapping, Role::Subject, &big) > 200_000);
    let budget = limited(1_000);
    let outcome = coverage.classify(Role::Subject, &big, &budget);
    assert_eq!(outcome, Err(QueryControlError::CompilerWorkExceeded));
    assert!(budget.consumed(QueryCharge::CompilerWork) <= 1_000);

    let far = format!("http://other.test/{}", "a".repeat(200_000));
    assert!(work(&mapping, Role::Subject, &far) < 200);
    assert_eq!(run(&mapping, Role::Subject, &far), NO);
}

#[test]
fn a_terminated_control_stops_template_matching_before_any_work() {
    let mapping = subject_only(tmpl("http://example.test/{id}", TermSpec::iri()));
    let coverage = MappingCoverage::new(&mapping);
    for reason in REASONS {
        let budget = unlimited();
        budget.terminate(reason);
        let outcome = coverage.classify(Role::Subject, &at("abc"), &budget);
        assert_eq!(outcome, Err(reason));
        assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
    }
}

#[test]
fn cancel_and_deadline_are_sticky_at_every_charge_including_late_segments() {
    let (mapping, query) = late_segments();
    let coverage = MappingCoverage::new(&mapping);
    let probe = Trip::new(u64::MAX, QueryControlError::Cancelled);
    let outcome = coverage.classify(Role::Subject, &query, &probe);
    assert_eq!(outcome, Ok(YES));
    let total = probe.calls.load(Ordering::SeqCst);
    let full = probe.budget.consumed(QueryCharge::CompilerWork);
    assert!(total > 80);
    for reason in REASONS {
        for at in 1..=total {
            let trip = Trip::new(at, reason);
            let outcome = coverage.classify(Role::Subject, &query, &trip);
            assert_eq!(outcome, Err(reason), "{reason:?} at {at}");
            assert!(trip.budget.consumed(QueryCharge::CompilerWork) < full);
            assert_eq!(trip.checkpoint(), Err(reason));
        }
        let late = Trip::new(total + 1, reason);
        let outcome = coverage.classify(Role::Subject, &query, &late);
        assert_eq!(outcome, Ok(YES));
    }
}

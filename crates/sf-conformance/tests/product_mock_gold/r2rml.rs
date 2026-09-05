use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use oxrdf::{NamedOrBlankNode, Term, Triple};
use oxttl::{NTriplesSerializer, TurtleParser};
use serde_json::Value;

use sf_core::ir::{LogicalSource, ObjectMap, Segment, TermMap, TermType, TriplesMap};
use sf_core::Term as FabricTerm;

use super::{
    array, expect_str, expect_u64, safe_relative, sha256, string, unsigned, CATEGORY_SHARD_PREFIX,
    STYLE_CLASS, STYLE_MAP, STYLE_NUMBER_PREDICATE, VERSION_PREDICATE,
};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RR_TRIPLES_MAP: &str = "http://www.w3.org/ns/r2rml#TriplesMap";
const RML_NAMESPACE: &str = "http://w3id.org/rml/";
const RML_TRIPLES_MAP: &str = "http://w3id.org/rml/TriplesMap";
const STYLE_SQL_QUERY: &str = "SELECT source.*, CASE source.\"status\" WHEN 'Draft' THEN \
    'https://hm.com/ns/semantic-product-mock/product-design/vocabulary/StyleStatus/Draft' \
    WHEN 'Locked' THEN \
    'https://hm.com/ns/semantic-product-mock/product-design/vocabulary/StyleStatus/Locked' \
    ELSE NULL END AS \"__sb_enum_2789c5144a07118e\" FROM \"style\" AS source";
const STYLE_TEMPLATE_PREFIX: &str = "https://hm.com/ns/semantic-product-mock/resource/record/\
    2e86ee77d484e286f8cf45bf9ef67aee749476294c96b3a13eb8fcd3e554b020/";

pub fn extract_relational_r2rml(
    category_bytes: &[u8],
    shards: &BTreeMap<String, Vec<u8>>,
) -> Result<String, &'static str> {
    let category: Value = serde_json::from_slice(category_bytes)
        .map_err(|_| "source-mapping category manifest is invalid")?;
    validate_category_claims(&category)?;
    let union = load_exact_shard_union(&category, shards)?;
    let (rr_roots, rml_root_count) = classified_roots(&union);
    if rr_roots.len() != 148 || rml_root_count != 134 {
        return Err("source-mapping triples-map root count mismatch");
    }
    let closure = reachable_closure(&union, rr_roots);
    if closure.iter().any(contains_rml_term) {
        return Err("relational R2RML closure reached an RML term");
    }
    serialize_closure(&closure)
}

pub(super) fn validate_mapping(
    category_bytes: &[u8],
    shards: &BTreeMap<String, Vec<u8>>,
) -> Result<String, &'static str> {
    let extracted = extract_relational_r2rml(category_bytes, shards)?;
    let maps = sf_mapping::parse_r2rml(&extracted)
        .map_err(|_| "relational R2RML closure did not parse")?;
    if maps.len() != 148
        || maps
            .iter()
            .map(|mapping| mapping.predicate_object_maps.len())
            .sum::<usize>()
            != 721
    {
        return Err("relational R2RML executable count mismatch");
    }
    validate_style_map(&maps)?;
    Ok(extracted)
}

fn validate_category_claims(category: &Value) -> Result<(), &'static str> {
    expect_u64(category, "/category", 13)?;
    expect_str(category, "/concern", "Source Mapping")?;
    expect_str(category, "/coverageStatus", "facet-scoped-complete")?;
    for (field, expected) in [
        ("rmlTriplesMaps", 134),
        ("predicateObjectMaps", 492),
        ("relationalSchemaTables", 112),
        ("relationalSchemaColumns", 598),
        ("relationalR2rmlTriplesMaps", 148),
        ("relationalR2rmlPredicateObjectMaps", 721),
        ("relationalR2rmlMappedTables", 112),
        ("relationalR2rmlMappedColumns", 598),
    ] {
        expect_u64(category, &format!("/stats/{field}"), expected)?;
    }
    Ok(())
}

fn load_exact_shard_union(
    category: &Value,
    shards: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, Triple>, &'static str> {
    let descriptors = array(category, "/shards")?;
    if descriptors.is_empty() || descriptors.len() != shards.len() {
        return Err("source-mapping shard set mismatch");
    }
    let mut paths = BTreeSet::new();
    let mut union = BTreeMap::new();
    for descriptor in descriptors {
        let path = string(descriptor, "/path")?;
        let bytes = unsigned(descriptor, "/bytes")?;
        let lines = unsigned(descriptor, "/lines")?;
        let digest = string(descriptor, "/digest")?;
        if !safe_relative(path)
            || !path.starts_with(CATEGORY_SHARD_PREFIX)
            || !path.ends_with(".ttl")
            || !paths.insert(path)
        {
            return Err("source-mapping shard descriptor is invalid");
        }
        let source = shards
            .get(path)
            .ok_or("source-mapping shard artifact is missing")?;
        let descriptor_lines = source.iter().filter(|byte| **byte == b'\n').count() as u64
            + u64::from(!source.is_empty());
        if source.len() as u64 != bytes || descriptor_lines != lines || sha256(source) != digest {
            return Err("source-mapping shard seal mismatch");
        }
        let source =
            std::str::from_utf8(source).map_err(|_| "source-mapping shard is not UTF-8")?;
        for triple in TurtleParser::new().for_slice(source) {
            let triple = triple.map_err(|_| "source-mapping shard Turtle is invalid")?;
            union.entry(triple.to_string()).or_insert(triple);
        }
    }
    if paths.len() != shards.len() || shards.keys().any(|path| !paths.contains(path.as_str())) {
        return Err("source-mapping shard set mismatch");
    }
    Ok(union)
}

fn classified_roots(union: &BTreeMap<String, Triple>) -> (Vec<NamedOrBlankNode>, usize) {
    let mut rr_roots = BTreeMap::new();
    let mut rml_roots = HashSet::new();
    for triple in union
        .values()
        .filter(|triple| triple.predicate.as_str() == RDF_TYPE)
    {
        let Term::NamedNode(kind) = &triple.object else {
            continue;
        };
        if kind.as_str() == RR_TRIPLES_MAP {
            rr_roots.insert(triple.subject.to_string(), triple.subject.clone());
        } else if kind.as_str() == RML_TRIPLES_MAP {
            rml_roots.insert(triple.subject.clone());
        }
    }
    (rr_roots.into_values().collect(), rml_roots.len())
}

fn reachable_closure(
    union: &BTreeMap<String, Triple>,
    roots: Vec<NamedOrBlankNode>,
) -> Vec<Triple> {
    let mut by_subject: HashMap<NamedOrBlankNode, Vec<&Triple>> = HashMap::new();
    for triple in union.values() {
        by_subject
            .entry(triple.subject.clone())
            .or_default()
            .push(triple);
    }
    let mut pending = VecDeque::from(roots);
    let mut visited = HashSet::new();
    let mut closure = BTreeMap::new();
    while let Some(subject) = pending.pop_front() {
        if !visited.insert(subject.clone()) {
            continue;
        }
        for triple in by_subject.get(&subject).into_iter().flatten() {
            closure
                .entry(triple.to_string())
                .or_insert_with(|| (*triple).clone());
            let object = match &triple.object {
                Term::NamedNode(node) => Some(NamedOrBlankNode::NamedNode(node.clone())),
                Term::BlankNode(node) => Some(NamedOrBlankNode::BlankNode(node.clone())),
                Term::Literal(_) | Term::Triple(_) => None,
            };
            if let Some(object) = object.filter(|node| by_subject.contains_key(node)) {
                pending.push_back(object);
            }
        }
    }
    closure.into_values().collect()
}

fn contains_rml_term(triple: &Triple) -> bool {
    matches!(&triple.subject,
        NamedOrBlankNode::NamedNode(node) if node.as_str().starts_with(RML_NAMESPACE))
        || triple.predicate.as_str().starts_with(RML_NAMESPACE)
        || matches!(&triple.object, Term::NamedNode(node) if node.as_str().starts_with(RML_NAMESPACE))
}

fn serialize_closure(closure: &[Triple]) -> Result<String, &'static str> {
    let mut serializer = NTriplesSerializer::new().for_writer(Vec::new());
    for triple in closure {
        serializer
            .serialize_triple(triple.as_ref())
            .map_err(|_| "relational R2RML closure serialization failed")?;
    }
    String::from_utf8(serializer.finish())
        .map_err(|_| "relational R2RML closure serialization was not UTF-8")
}

fn validate_style_map(maps: &[TriplesMap]) -> Result<(), &'static str> {
    let map = maps
        .iter()
        .find(|mapping| mapping.id == STYLE_MAP)
        .ok_or("Style R2RML map is missing")?;
    if !matches!(&map.source, LogicalSource::Query(query) if query == STYLE_SQL_QUERY)
        || map.subject.classes.len() != 1
        || map.subject.classes[0].as_str() != STYLE_CLASS
        || map.predicate_object_maps.len() != 11
    {
        return Err("Style R2RML source or class mismatch");
    }
    match &map.subject.term {
        TermMap::Template(template, spec)
            if spec.term_type == TermType::Iri
                && matches!(template.segments(),
                    [Segment::Literal(prefix), Segment::Column(column)]
                    if prefix.as_ref() == STYLE_TEMPLATE_PREFIX && column.as_ref() == "style_number") =>
            {}
        _ => return Err("Style R2RML subject mismatch"),
    }
    require_literal_column(
        map,
        STYLE_NUMBER_PREDICATE,
        "style_number",
        "http://www.w3.org/2001/XMLSchema#string",
    )?;
    require_literal_column(
        map,
        VERSION_PREDICATE,
        "version",
        "http://www.w3.org/2001/XMLSchema#integer",
    )
}

fn require_literal_column(
    map: &TriplesMap,
    expected_predicate: &str,
    expected_column: &str,
    expected_datatype: &str,
) -> Result<(), &'static str> {
    let matches: Vec<_> = map
        .predicate_object_maps
        .iter()
        .filter(|pom| {
            matches!(pom.predicates.as_slice(),
                [TermMap::Constant(FabricTerm::NamedNode(predicate))]
                if predicate.as_str() == expected_predicate)
        })
        .collect();
    let [pom] = matches.as_slice() else {
        return Err("Style R2RML required predicate mismatch");
    };
    match pom.objects.as_slice() {
        [ObjectMap::Term(TermMap::Column(column, spec))]
            if column.as_ref() == expected_column
                && spec.datatype.as_ref().map(|value| value.as_str())
                    == Some(expected_datatype) =>
        {
            Ok(())
        }
        _ => Err("Style R2RML mapped-column contract mismatch"),
    }
}

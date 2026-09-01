use sf_core::ir::{LogicalSource, ObjectMap, Segment, TermMap, TermType};
use sf_core::Term;

use super::{STYLE_CLASS, STYLE_MAP, STYLE_NUMBER_PREDICATE, VERSION_PREDICATE};

pub fn extract_r2rml(source: &str) -> Result<&str, &'static str> {
    let marker = " a rml:TriplesMap ;";
    let marker_at = source
        .find(marker)
        .ok_or("first RML map boundary is missing")?;
    let start = source[..marker_at]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let extracted = &source[..start];
    if extracted.matches(" a rr:TriplesMap ;").count() != 1
        || extracted.contains(" a rml:TriplesMap ;")
        || extracted.contains("rml:logicalSource")
        || extracted.contains("rml:reference ")
    {
        return Err("R2RML extraction boundary mismatch");
    }
    Ok(extracted)
}

pub(super) fn validate_mapping(bytes: &[u8]) -> Result<String, &'static str> {
    let source = std::str::from_utf8(bytes).map_err(|_| "source mapping is not UTF-8")?;
    let extracted = extract_r2rml(source)?;
    let maps = sf_mapping::parse_r2rml(extracted).map_err(|_| "R2RML block did not parse")?;
    if maps.len() != 1 || maps[0].id != STYLE_MAP {
        return Err("Style R2RML map identity mismatch");
    }
    let map = &maps[0];
    if !matches!(&map.source, LogicalSource::Table(table) if table == "style")
        || map.subject.classes.len() != 1
        || map.subject.classes[0].as_str() != STYLE_CLASS
    {
        return Err("Style R2RML source or class mismatch");
    }
    match &map.subject.term {
        TermMap::Template(template, spec)
            if spec.term_type == TermType::Iri
                && matches!(template.segments(), [Segment::Literal(_), Segment::Column(column)] if column.as_ref() == "style_number") =>
            {}
        _ => return Err("Style R2RML subject mismatch"),
    }
    let mut mapped = Vec::new();
    for pom in &map.predicate_object_maps {
        let predicate = match pom.predicates.as_slice() {
            [TermMap::Constant(Term::NamedNode(value))] => value.as_str(),
            _ => return Err("Style R2RML predicate mismatch"),
        };
        let (column, datatype) = match pom.objects.as_slice() {
            [ObjectMap::Term(TermMap::Column(column, spec))] => (
                column.as_ref(),
                spec.datatype.as_ref().map(|value| value.as_str()),
            ),
            _ => return Err("Style R2RML object mismatch"),
        };
        mapped.push((predicate, column, datatype));
    }
    if mapped
        != [
            (
                STYLE_NUMBER_PREDICATE,
                "style_number",
                Some("http://www.w3.org/2001/XMLSchema#string"),
            ),
            (
                VERSION_PREDICATE,
                "version",
                Some("http://www.w3.org/2001/XMLSchema#integer"),
            ),
        ]
    {
        return Err("Style R2RML mapped-column contract mismatch");
    }
    Ok(extracted.to_owned())
}

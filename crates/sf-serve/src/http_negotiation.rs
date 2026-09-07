//! Existing endpoint representation selection, separated from request execution.
use crate::stream::RdfFormat;
use sparesults::QueryResultsFormat;

pub(super) fn negotiate_results(accept: Option<&str>) -> QueryResultsFormat {
    let a = accept.unwrap_or("").to_ascii_lowercase();
    if a.contains("sparql-results+xml") || a.contains("application/xml") || a.contains("text/xml") {
        QueryResultsFormat::Xml
    } else if a.contains("text/tab-separated-values") {
        QueryResultsFormat::Tsv
    } else if a.contains("text/csv") {
        QueryResultsFormat::Csv
    } else {
        QueryResultsFormat::Json
    }
}

pub(super) fn negotiate_rdf(accept: Option<&str>) -> RdfFormat {
    let a = accept.unwrap_or("").to_ascii_lowercase();
    if a.contains("application/ld+json") {
        RdfFormat::JsonLd
    } else if a.contains("application/n-triples") {
        RdfFormat::NTriples
    } else {
        RdfFormat::Turtle
    }
}

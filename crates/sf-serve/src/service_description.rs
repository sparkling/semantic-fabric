//! Fixed, redacted SPARQL Service Description for endpoint discovery.

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::response::Response;

use crate::config::QueryMode;
use crate::problem::{self, ProblemCode};

const MEDIA_TYPE: &str = "text/turtle; charset=utf-8";
// `sd:SPARQL11Query` and `sd:BasicFederatedQuery` denote broader standard
// capabilities than this endpoint implements. The SD vocabulary permits
// service-defined `sd:Language` and `sd:Feature` resources, so fixed project
// URNs describe the tested subsets without upgrading them into standards or
// production-admission claims. Result-format IRIs are the W3C registry terms
// for the serializers reachable in each serving mode.
const SINGLE_SOURCE: &str = r#"@prefix sd: <http://www.w3.org/ns/sparql-service-description#> .
@prefix sf: <urn:semantic-fabric:service-description:> .

<> a sd:Service ;
    sd:endpoint <> ;
    sd:supportedLanguage sf:bounded-read-query-v1 ;
    sd:resultFormat <http://www.w3.org/ns/formats/SPARQL_Results_JSON>,
        <http://www.w3.org/ns/formats/SPARQL_Results_XML>,
        <http://www.w3.org/ns/formats/SPARQL_Results_CSV>,
        <http://www.w3.org/ns/formats/SPARQL_Results_TSV>,
        <http://www.w3.org/ns/formats/Turtle>,
        <http://www.w3.org/ns/formats/N-Triples>,
        <http://www.w3.org/ns/formats/JSON-LD> ;
    sd:feature sf:select-query,
        sf:ask-query,
        sf:construct-query .

sf:bounded-read-query-v1 a sd:Language .
sf:select-query a sd:Feature .
sf:ask-query a sd:Feature .
sf:construct-query a sd:Feature .
"#;
const TWO_SOURCE: &str = r#"@prefix sd: <http://www.w3.org/ns/sparql-service-description#> .
@prefix sf: <urn:semantic-fabric:service-description:> .

<> a sd:Service ;
    sd:endpoint <> ;
    sd:supportedLanguage sf:two-source-select-union-query-v1 ;
    sd:resultFormat <http://www.w3.org/ns/formats/SPARQL_Results_JSON>,
        <http://www.w3.org/ns/formats/SPARQL_Results_XML>,
        <http://www.w3.org/ns/formats/SPARQL_Results_CSV>,
        <http://www.w3.org/ns/formats/SPARQL_Results_TSV> ;
    sd:feature sf:select-query,
        sf:source-affine-two-arm-select-union-v1 .

sf:two-source-select-union-query-v1 a sd:Language .
sf:select-query a sd:Feature .
sf:source-affine-two-arm-select-union-v1 a sd:Feature .
"#;

/// The W3C Service Description discovery request is an exact, query-less GET.
/// HEAD has the same headers and status with its representation body suppressed.
pub(crate) fn is_request<B>(request: &Request<B>) -> bool {
    matches!(request.method(), &Method::GET | &Method::HEAD)
        && request.uri().path() == "/sparql"
        && request.uri().query().is_none()
}

/// Return metadata without acquiring a runtime generation or query-work permit.
pub(crate) fn response(headers: &HeaderMap, mode: QueryMode, head: bool) -> Response {
    if !accepts_turtle(headers) {
        let response = problem::response(ProblemCode::NotAcceptable);
        return if head {
            response.map(|_| Body::empty())
        } else {
            response
        };
    }

    let document = match mode {
        QueryMode::Single(_) => SINGLE_SOURCE,
        QueryMode::SourceAffineUnion(_) => TWO_SOURCE,
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, MEDIA_TYPE)
        .header(header::CONTENT_LENGTH, document.len().to_string())
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::VARY, "Accept")
        .header("x-content-type-options", "nosniff")
        .body(if head {
            Body::empty()
        } else {
            Body::from(document)
        })
        .expect("fixed service-description response")
}

/// Select the sole representation while respecting explicit `q=0` exclusions.
fn accepts_turtle(headers: &HeaderMap) -> bool {
    let values = headers.get_all(header::ACCEPT);
    if values.iter().next().is_none() {
        return true;
    }

    let mut selected: Option<(u8, u16)> = None;
    for value in values {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for item in value.split(',') {
            let mut parts = item.split(';');
            let specificity = match parts.next().unwrap_or_default().trim() {
                media if media.eq_ignore_ascii_case("text/turtle") => 2,
                media if media.eq_ignore_ascii_case("text/*") => 1,
                "*/*" => 0,
                _ => continue,
            };
            let Some(quality) = quality(parts) else {
                continue;
            };
            match selected {
                Some((prior_specificity, _)) if prior_specificity > specificity => {}
                Some((prior_specificity, prior_quality))
                    if prior_specificity == specificity && prior_quality >= quality => {}
                _ => selected = Some((specificity, quality)),
            }
        }
    }
    selected.is_some_and(|(_, quality)| quality > 0)
}

fn quality<'a>(parameters: impl Iterator<Item = &'a str>) -> Option<u16> {
    let mut quality = None;
    for parameter in parameters {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("q") {
            if quality.is_some() {
                return None;
            }
            quality = Some(parse_quality(value.trim())?);
        }
    }
    Some(quality.unwrap_or(1000))
}

fn parse_quality(value: &str) -> Option<u16> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match whole {
        "1" if fraction.bytes().all(|byte| byte == b'0') => Some(1000),
        "0" => {
            let parsed = if fraction.is_empty() {
                0
            } else {
                fraction.parse::<u16>().ok()? * 10_u16.pow((3 - fraction.len()) as u32)
            };
            Some(parsed)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "service_description_tests.rs"]
mod tests;

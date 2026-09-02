//! Header-first, media-specific admission for SPARQL Protocol POST bodies.

use axum::body::Body;
use axum::http::{header, HeaderMap};
use http_body_util::BodyExt;

use crate::problem::ProblemCode;

pub(crate) async fn query(
    headers: &HeaderMap,
    body: Body,
    max_query_len: usize,
    max_form_body_len: usize,
) -> Result<String, ProblemCode> {
    let mut content_types = headers.get_all(header::CONTENT_TYPE).iter();
    let media_type = content_types
        .next()
        .ok_or(ProblemCode::UnsupportedMediaType)?
        .to_str()
        .map_err(|_| ProblemCode::UnsupportedMediaType)?
        .trim();
    if content_types.next().is_some() {
        return Err(ProblemCode::InvalidRequest);
    }
    if media_type.contains(';') {
        return Err(ProblemCode::UnsupportedMediaType);
    }

    match media_type {
        value if value.eq_ignore_ascii_case("application/sparql-query") => {
            let bytes = collect(body, max_query_len).await?;
            String::from_utf8(bytes).map_err(|_| ProblemCode::InvalidRequest)
        }
        value if value.eq_ignore_ascii_case("application/x-www-form-urlencoded") => {
            let bytes = collect(body, max_form_body_len).await?;
            let encoded = String::from_utf8(bytes).map_err(|_| ProblemCode::InvalidRequest)?;
            unique_query_param(encoded.as_bytes(), max_query_len).map_err(|error| match error {
                QueryParamError::Invalid => ProblemCode::InvalidRequest,
                QueryParamError::TooLong => ProblemCode::PayloadTooLarge,
            })
        }
        _ => Err(ProblemCode::UnsupportedMediaType),
    }
}

async fn collect(mut body: Body, limit: usize) -> Result<Vec<u8>, ProblemCode> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ProblemCode::InvalidRequest)?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        let Some(new_len) = bytes.len().checked_add(data.len()) else {
            return Err(ProblemCode::PayloadTooLarge);
        };
        if new_len > limit {
            return Err(ProblemCode::PayloadTooLarge);
        }
        bytes.extend_from_slice(&data);
    }
    Ok(bytes)
}

/// Decode exactly one `query` field and reject duplicates or every other key.
pub(crate) fn unique_query_param(
    encoded: &[u8],
    max_query_len: usize,
) -> Result<String, QueryParamError> {
    let mut query = None;
    for (key, value) in form_urlencoded::parse(encoded) {
        if key != "query" || query.is_some() {
            return Err(QueryParamError::Invalid);
        }
        if value.len() > max_query_len {
            return Err(QueryParamError::TooLong);
        }
        query = Some(value.into_owned());
    }
    query.ok_or(QueryParamError::Invalid)
}

pub(crate) enum QueryParamError {
    Invalid,
    TooLong,
}

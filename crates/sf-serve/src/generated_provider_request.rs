//! Public, versioned request-preparation contract for generated SELECT/ASK
//! requests and the exact Query C2 successor templates (schema
//! [`SCHEMA_VERSION`], ADR-0056 producer interface, SOURCE work only).
//!
//! Status: the request half of the producer interface. Response handling and
//! profile-identity issuance for prepared requests remain dependent on accepted
//! runtime identity and serving work.
//!
//! Preparation is **not** admission:
//! * A raw generated query is neither keyword-scanned nor SPARQL-parsed here;
//!   only emptiness and the explicit byte limit are checked. Parsing, subject
//!   admission and generated-shape/coverage admission remain the existing
//!   governed engine boundary behind the endpoint.
//! * No source, owner or grant check runs, no credential is read, no endpoint
//!   is provisioned and no profile identity is issued. A
//!   [`PreparedGeneratedRequest`] is not a receipt and has no conversion into
//!   admission or authorization.
//! * The versioned JSON document is an embedding/adapter data contract, not an
//!   accepted HTTP body format. The only supported delivery is the existing
//!   [`DELIVERY_METHOD`] [`DELIVERY_PATH`] with the prepared query as the raw
//!   [`DELIVERY_CONTENT_TYPE`] body, accepting [`DELIVERY_ACCEPT`], on a server
//!   whose embedding selected
//!   [`QueryShapeProfile::GeneratedSelectAsk`](crate::QueryShapeProfile::GeneratedSelectAsk).
//!   The selector is never sent and confers no authority.
//!
//! Template selectors render compiled-in byte-for-byte copies of Query's
//! `c2-successor-enumerate-v1` and `c2-successor-root-v1` assets (immutable
//! revision `b676cf4f32a7148ba51273e8356869550ccea15f`). Nothing is appended
//! (no `FROM`); the `LIMIT 65` look-ahead over 64-style pages and the
//! consumer's 100 000-style / 4 MiB containment ceilings are unchanged and
//! remain the consumer's to enforce. Style identities follow Query's
//! `valid_style_token` (1..=64 bytes of ASCII alphanumerics, `-` or `_`); only
//! the enumeration cursor may be empty, selecting the first page. Every
//! placeholder occurrence is replaced and the terminal newline is kept. Each
//! rendered query has its own digest; a template digest identifies the
//! unrendered asset and never depends on the parameter.
//!
//! Test fixtures for this module are preparation-only and must never be handed
//! off as evidence of live admission.

use std::borrow::Cow;
use std::fmt;

use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use sha2::{Digest, Sha256};

use crate::generated_provider_templates::{self as templates, Template};

/// Explicit public schema version of the request document.
pub const SCHEMA_VERSION: &str = "semantic-fabric.generated-provider-request.v1";
/// Ceiling for [`QueryByteLimit`]: the existing 1 MiB compiler text envelope.
pub const MAX_QUERY_BYTES_CEILING: usize = 1024 * 1024;
/// Longest accepted style identity, as in Query's `valid_style_token`.
pub const MAX_STYLE_NUMBER_BYTES: usize = templates::MAX_STYLE_NUMBER_BYTES;

/// Pinned identity of the transferred enumeration template.
pub const C2_SUCCESSOR_ENUMERATE_V1_ID: &str = templates::ENUMERATE_ID;
/// `sha256:<lowerhex>` of the unrendered enumeration template bytes.
pub const C2_SUCCESSOR_ENUMERATE_V1_DIGEST: &str = templates::ENUMERATE_DIGEST;
/// Pinned identity of the transferred per-style root template.
pub const C2_SUCCESSOR_ROOT_V1_ID: &str = templates::ROOT_ID;
/// `sha256:<lowerhex>` of the unrendered root template bytes.
pub const C2_SUCCESSOR_ROOT_V1_DIGEST: &str = templates::ROOT_DIGEST;

/// Supported delivery method: the existing SPARQL Protocol query operation.
pub const DELIVERY_METHOD: &str = "POST";
/// Supported delivery path.
pub const DELIVERY_PATH: &str = "/sparql";
/// Raw body media type; the body is exactly [`PreparedGeneratedRequest::query`].
pub const DELIVERY_CONTENT_TYPE: &str = "application/sparql-query";
/// Results form for generated SELECT/ASK responses.
pub const DELIVERY_ACCEPT: &str = "application/sparql-results+json";

const SELECTOR_KEYS: [(&str, RequestSelector); 3] = [
    RequestSelector::GeneratedQuery.key(),
    RequestSelector::C2SuccessorEnumerateV1.key(),
    RequestSelector::C2SuccessorRootV1.key(),
];
/// A JSON string escape expands one UTF-8 byte to at most six bytes.
const JSON_ESCAPE_FACTOR: usize = 6;
/// Fixed allowance for keys, punctuation and insignificant whitespace.
const JSON_ENVELOPE_BYTES: usize = 4096;

/// Typed, redacted failure. Never carries a query, parameter or any fragment
/// of the request document.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum GeneratedProviderRequestError {
    #[error("maximum query bytes must be within 1..=1048576")]
    InvalidQueryLimit,
    #[error("request document exceeds its byte limit")]
    DocumentTooLarge,
    #[error("request document is malformed or has an unknown field or selector")]
    MalformedDocument,
    #[error("request document has an unsupported schema version")]
    UnsupportedSchema,
    #[error("generated query is empty")]
    EmptyQuery,
    #[error("query exceeds its byte limit")]
    QueryTooLarge,
    #[error("style number is not a valid style token")]
    InvalidStyleNumber,
}

type RequestError = GeneratedProviderRequestError;

/// Explicit finite byte bound for a prepared query, within
/// `1..=`[`MAX_QUERY_BYTES_CEILING`]. Embeddings should pass the serving
/// config's own query limit so a prepared query never exceeds what the
/// endpoint accepts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryByteLimit(usize);

impl QueryByteLimit {
    pub const fn new(max_query_bytes: usize) -> Result<Self, GeneratedProviderRequestError> {
        if max_query_bytes == 0 || max_query_bytes > MAX_QUERY_BYTES_CEILING {
            return Err(RequestError::InvalidQueryLimit);
        }
        Ok(Self(max_query_bytes))
    }

    pub const fn max_query_bytes(self) -> usize {
        self.0
    }

    /// Byte bound checked on a JSON document before deserialization: six
    /// bytes per query byte plus a fixed envelope allowance.
    pub const fn max_document_bytes(self) -> usize {
        self.0 * JSON_ESCAPE_FACTOR + JSON_ENVELOPE_BYTES
    }
}

/// Closed set of typed request selectors in schema v1. A selector records how
/// the query was prepared; it is never sent and confers no authority.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RequestSelector {
    /// Raw generated SELECT/ASK text (`sparql`), carried unchanged.
    GeneratedQuery,
    /// `c2-successor-enumerate-v1` page after `afterStyleNumber`.
    C2SuccessorEnumerateV1,
    /// `c2-successor-root-v1` reread of `styleNumber`.
    C2SuccessorRootV1,
}

impl RequestSelector {
    /// Serialized selector name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::GeneratedQuery => "GeneratedQuery",
            Self::C2SuccessorEnumerateV1 => "C2SuccessorEnumerateV1",
            Self::C2SuccessorRootV1 => "C2SuccessorRootV1",
        }
    }

    const fn field(self) -> &'static str {
        match self {
            Self::GeneratedQuery => "sparql",
            Self::C2SuccessorEnumerateV1 => "afterStyleNumber",
            Self::C2SuccessorRootV1 => "styleNumber",
        }
    }

    const fn key(self) -> (&'static str, Self) {
        (self.name(), self)
    }

    const fn template(self) -> Option<Template> {
        match self {
            Self::GeneratedQuery => None,
            Self::C2SuccessorEnumerateV1 => Some(Template::Enumerate),
            Self::C2SuccessorRootV1 => Some(Template::Root),
        }
    }

    /// Field bounds shared by every constructor and the JSON parameter seed,
    /// checked on borrowed input before anything is copied or hashed.
    fn check(self, value: &str, limit: QueryByteLimit) -> Result<(), RequestError> {
        let too_large = value.len() > limit.max_query_bytes();
        match self {
            Self::GeneratedQuery if value.is_empty() => Err(RequestError::EmptyQuery),
            Self::GeneratedQuery if too_large => Err(RequestError::QueryTooLarge),
            Self::GeneratedQuery => Ok(()),
            Self::C2SuccessorEnumerateV1 if value.is_empty() => Ok(()),
            _ if templates::valid_style_token(value) => Ok(()),
            _ => Err(RequestError::InvalidStyleNumber),
        }
    }
}

/// Immutable prepared request: the exact rendered UTF-8 query, its own
/// `sha256:<lowerhex>` digest and, for template selectors, the pinned template
/// identity. Only this module's validating constructors build it. It is not
/// admitted, authorized or issued, and `Debug` redacts query and parameter.
#[derive(Clone, Eq, PartialEq)]
pub struct PreparedGeneratedRequest {
    selector: RequestSelector,
    /// Template parameter; `None` for a raw generated query.
    parameter: Option<String>,
    query: String,
    query_digest: String,
}

impl PreparedGeneratedRequest {
    /// Prepare raw generated SELECT/ASK text. Only emptiness and the byte
    /// limit are checked; the text is not scanned or parsed.
    pub fn generated_query(
        sparql: &str,
        limit: QueryByteLimit,
    ) -> Result<Self, GeneratedProviderRequestError> {
        Self::prepare(RequestSelector::GeneratedQuery, sparql, limit)
    }

    /// Prepare one `c2-successor-enumerate-v1` page after `after_style_number`.
    /// An empty cursor requests the first page.
    pub fn c2_successor_enumerate(
        after_style_number: &str,
        limit: QueryByteLimit,
    ) -> Result<Self, GeneratedProviderRequestError> {
        let selector = RequestSelector::C2SuccessorEnumerateV1;
        Self::prepare(selector, after_style_number, limit)
    }

    /// Prepare the `c2-successor-root-v1` reread of one enumerated style.
    pub fn c2_successor_root(
        style_number: &str,
        limit: QueryByteLimit,
    ) -> Result<Self, GeneratedProviderRequestError> {
        Self::prepare(RequestSelector::C2SuccessorRootV1, style_number, limit)
    }

    /// Parse one schema-v1 JSON document. The document byte bound is checked
    /// before deserialization and also bounds serde_json's escape scratch
    /// buffer. Keys and the schema version are compared without copying, and
    /// the parameter passes the constructors' own checks before it is copied.
    /// Unknown schema versions, fields and selectors, duplicate keys and
    /// non-object shapes are refused; a malformed document outranks an
    /// unsupported schema, which outranks a field refusal.
    pub fn from_json(
        document: &[u8],
        limit: QueryByteLimit,
    ) -> Result<Self, GeneratedProviderRequestError> {
        if document.len() > limit.max_document_bytes() {
            return Err(RequestError::DocumentTooLarge);
        }
        let mut json = serde_json::Deserializer::from_slice(document);
        let parsed = Object(DocumentSeed(limit)).deserialize(&mut json);
        let parsed = parsed.and_then(|parsed| json.end().map(|()| parsed));
        let (schema, selector, value) = parsed.map_err(|_| RequestError::MalformedDocument)?;
        if !schema {
            return Err(RequestError::UnsupportedSchema);
        }
        Self::prepare(selector, &value?, limit)
    }

    /// Schema-v1 JSON document for this request. Adapter data only; it is
    /// never an HTTP request body.
    pub fn to_json(&self) -> String {
        let value = self.parameter.as_deref().unwrap_or(self.query.as_str());
        let value = serde_json::Value::from(value);
        let (name, field) = (self.selector.name(), self.selector.field());
        let selector = format!(r#"{{"{name}":{{"{field}":{value}}}}}"#);
        format!(r#"{{"schemaVersion":"{SCHEMA_VERSION}","selector":{selector}}}"#)
    }

    pub fn selector(&self) -> RequestSelector {
        self.selector
    }

    /// Exact rendered UTF-8 query: the complete raw delivery body.
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn query_bytes(&self) -> &[u8] {
        self.query.as_bytes()
    }

    /// `sha256:<lowerhex>` over exactly the bytes of [`Self::query`].
    pub fn query_digest(&self) -> &str {
        &self.query_digest
    }

    /// Pinned template identity; `None` for a raw generated query.
    pub fn template_id(&self) -> Option<&'static str> {
        self.selector.template().map(Template::id)
    }

    /// Pinned digest of the unrendered template, independent of the parameter.
    pub fn template_digest(&self) -> Option<&'static str> {
        self.selector.template().map(Template::digest)
    }

    /// The raw delivery body, consuming the prepared request.
    pub fn into_query(self) -> String {
        self.query
    }

    /// The one validating path: check the borrowed value, then copy, render
    /// and hash. The rendered length is computed before any allocation.
    fn prepare(
        selector: RequestSelector,
        value: &str,
        limit: QueryByteLimit,
    ) -> Result<Self, GeneratedProviderRequestError> {
        selector.check(value, limit)?;
        let Some(template) = selector.template() else {
            return Ok(Self::sealed(selector, None, value.to_owned()));
        };
        let length = templates::rendered_len(template, value)
            .filter(|length| *length <= limit.max_query_bytes())
            .ok_or(RequestError::QueryTooLarge)?;
        let query = templates::render(template, value, length);
        Ok(Self::sealed(selector, Some(value.to_owned()), query))
    }

    fn sealed(selector: RequestSelector, parameter: Option<String>, query: String) -> Self {
        let query_digest = sha256_label(query.as_bytes());
        Self {
            selector,
            parameter,
            query,
            query_digest,
        }
    }
}

impl fmt::Debug for PreparedGeneratedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedGeneratedRequest")
            .field("selector", &self.selector)
            .finish_non_exhaustive()
    }
}

fn sha256_label(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in &digest {
        out.push(char::from(DIGITS[usize::from(*byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(*byte & 0x0f)]));
    }
    out
}

fn refused<E: de::Error>() -> E {
    E::custom("not a schema-v1 generated provider request")
}

/// A parameter that passed its field checks, borrowed from the bounded
/// document unless it had escapes, or the typed, input-free field refusal.
type Checked<'de> = Result<Cow<'de, str>, GeneratedProviderRequestError>;

/// Reads one JSON object with the wrapped visitor.
struct Object<V>(V);

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for Object<V> {
    type Value = V::Value;

    fn deserialize<D: de::Deserializer<'de>>(self, input: D) -> Result<V::Value, D::Error> {
        input.deserialize_map(self.0)
    }
}

/// Reads one JSON string with the wrapped visitor.
struct Str<V>(V);

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for Str<V> {
    type Value = V::Value;

    fn deserialize<D: de::Deserializer<'de>>(self, input: D) -> Result<V::Value, D::Error> {
        input.deserialize_str(self.0)
    }
}

/// Matches a decoded string against a closed set without copying it.
struct Lookup<'k, T>(&'k [(&'k str, T)]);

impl<'de, T: Copy> Visitor<'de> for Lookup<'_, T> {
    type Value = Option<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        let found = self.0.iter().find(|(name, _)| *name == value);
        Ok(found.map(|(_, item)| *item))
    }
}

#[derive(Clone, Copy)]
enum TopKey {
    Schema,
    Selector,
}

const TOP_KEYS: [(&str, TopKey); 2] = [
    ("schemaVersion", TopKey::Schema),
    ("selector", TopKey::Selector),
];
const SCHEMA_KEYS: [(&str, ()); 1] = [(SCHEMA_VERSION, ())];

/// The document: exactly `schemaVersion` and `selector`, each once. Yields
/// whether the schema is supported, the selector and its checked parameter.
struct DocumentSeed(QueryByteLimit);

impl<'de> Visitor<'de> for DocumentSeed {
    type Value = (bool, RequestSelector, Checked<'de>);

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a request object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let (mut schema, mut selected) = (None, None);
        while let Some(key) = map.next_key_seed(Str(Lookup(&TOP_KEYS)))? {
            match key {
                Some(TopKey::Schema) if schema.is_none() => {
                    schema = Some(map.next_value_seed(Str(Lookup(&SCHEMA_KEYS)))?.is_some());
                }
                Some(TopKey::Selector) if selected.is_none() => {
                    selected = Some(map.next_value_seed(Object(SelectorSeed(self.0)))?);
                }
                _ => return Err(refused()),
            }
        }
        match (schema, selected) {
            (Some(schema), Some((selector, value))) => Ok((schema, selector, value)),
            _ => Err(refused()),
        }
    }
}

/// The `selector` object: exactly one known selector key.
struct SelectorSeed(QueryByteLimit);

impl<'de> Visitor<'de> for SelectorSeed {
    type Value = (RequestSelector, Checked<'de>);

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object with exactly one known selector")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let key = map.next_key_seed(Str(Lookup(&SELECTOR_KEYS)))?;
        let selector = key.flatten().ok_or_else(refused::<A::Error>)?;
        let value = map.next_value_seed(Object(Parameter(selector, self.0)))?;
        if map.next_key::<IgnoredAny>()?.is_some() {
            return Err(refused());
        }
        Ok((selector, value))
    }
}

/// A selector's parameter object: exactly that selector's field, whose string
/// value passes [`RequestSelector::check`] before it is copied. An escaped
/// value decoded into the parser's scratch buffer is copied only on success;
/// a refusal is returned in-band so the rest of the document is still checked.
#[derive(Clone, Copy)]
struct Parameter(RequestSelector, QueryByteLimit);

impl<'de> Visitor<'de> for Parameter {
    type Value = Checked<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object with exactly one string parameter")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let field = [(self.0.field(), ())];
        let key = map.next_key_seed(Str(Lookup(&field)))?;
        key.flatten().ok_or_else(refused::<A::Error>)?;
        let value = map.next_value_seed(Str(self))?;
        if map.next_key::<IgnoredAny>()?.is_some() {
            return Err(refused());
        }
        Ok(value)
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(self.0.check(value, self.1).map(|()| Cow::Borrowed(value)))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        let checked = self.0.check(value, self.1);
        Ok(checked.map(|()| Cow::Owned(value.to_owned())))
    }
}

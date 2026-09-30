# oxrdfxml provenance

Source preparation only. This directory is not a dependency selection, advisory closure, or published patch. Root `Cargo.toml`, root `Cargo.lock` and the HTTP files are unchanged and not selected by this import.

## Base

- Crate: `oxrdfxml` 0.2.3 (published release), with `sparesults` 0.3.3, at Oxigraph release commit `4f8e1d890eee73062eaf809b779ed1b276aa11d4`.
- Parser and XML replacement taken from `822b7c9462dea8b525fed3cb8150bc3e9b1c243b`.
- The deprecated `RdfXmlParser::unchecked()` API is restored (it forwards to `lenient()`).
- Manifest dependencies: `quick-xml` exactly `=0.41.0` and `oxrdf` exactly `=0.3.3`. No other dependency requirement was changed.

## Upstream port not used

Upstream commit `e115a6a8dd9213fdf89a20cb72494ab333878218` (quick-xml 0.41 port) cannot replace this source directly: it depends on `OxString`, which does not exist in oxrdf 0.3.3. The 31-error probe from that attempt is preserved as a finding. No new upstream research was done.

## Local changes

Mechanical module decomposition and the scoped lexical correction below keep every authored source file at most 500 lines:

- `src/parser.rs` is now a module index plus public re-exports. Its content lives in `src/parser/`: `builder`, `reader`, `async_reader`, `slice`, `prefixes`, `state`, `events`, `node`, `property`, `literal`, `entities`, `version`.
- `src/serializer.rs` keeps the public serializer types. The internal writer moved to `src/serializer/writer.rs` and the unit tests to `src/serializer/tests.rs`.
- `parse_start_event` was split into coherent helpers (`parse_literal_start_event`, `parse_start_attributes`, `build_property_elt`) with unchanged behavior and error order. `InnerRdfXmlWriter::new` holds the body of the former `RdfXmlSerializer::inner_writer`.
- Private visibility became `pub(super)` where the split required it. Public API, feature gates, docs, doctests and licenses are unchanged, except that four intra-doc links now use explicit `crate::` paths: `RdfXmlParser::for_reader`, `RdfXmlParser::for_tokio_async_reader`, `RdfXmlParser::for_slice` and `ReaderRdfXmlParser::prefixes`.
- `Cargo.toml` is the Cargo-normalized published 0.2.3 manifest with exact `quick-xml =0.41.0`, exact `oxrdf =0.3.3` and an explicit `[workspace]` table added. `Cargo.lock` came from offline Cargo resolution and must not be edited by hand.
- `LICENSE-APACHE`, `LICENSE-MIT`, `README.md`, `src/error.rs`, `src/lib.rs` and `src/utils.rs` are unchanged from the prepared source.

## Tests

- The prepared source has no parser unit tests. There is no `src/parser/tests.rs` file, and `src/parser.rs` declares no `mod tests`. A `src/parser/tests.rs` path appearing in a review scope is an absent optional path, not adopted source.
- All three unit tests live in `src/serializer/tests.rs`: `test_split_iri`, `test_custom_rdf_ns` and `test_custom_empty_ns`.
- The doctests are in `README.md`, the `src/parser/` modules (`builder`, `reader`, `async_reader`, `slice`) and `src/serializer.rs`.

## Status

September 30 lexical correction: the shared serializer emits every literal CR
as an XML character reference after XML escaping. Raw XML CR/CRLF parsing still
normalizes according to XML; literal character references remain distinct.
Registered `src/serializer/lexical_tests.rs` covers slice/reader/async parsing,
sync/async serialization, typed/language/directional literals, quoted triples,
entity-looking text and boundary/interior whitespace. Original three serializer
tests remain unchanged. This correction does not select the root dependency or
claim advisory closure.

This file records provenance only. It does not claim that any build, test, doctest or format check has run.

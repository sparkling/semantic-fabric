# sparesults provenance

Source preparation only. This import does not select root dependencies, close
security advisories, issue admission, or publish a patch.

## Sources

- Published `sparesults` 0.3.3, alongside `oxrdfxml` 0.2.3, from Oxigraph release
  `4f8e1d890eee73062eaf809b779ed1b276aa11d4`.
- XML compatibility source from `822b7c9462dea8b525fed3cb8150bc3e9b1c243b`.
- Cargo-normalized published manifest retains existing features and requirements
  except exact `quick-xml =0.41.0`, exact `oxrdf =0.3.3`, and an explicit
  `[workspace]` table. `Cargo.lock` comes from offline Cargo resolution.
- MIT and Apache-2.0 license texts are preserved verbatim.

## Local changes

The prepared compatibility source is split into real child modules to keep each
authored file below 500 lines. Public paths remain available through re-exports;
private visibility changes only support access between parent and child modules.

- `csv`: writers, TSV encoding, readers, and the original nine unit tests.
- `json`: writers, readers, state handling, and term handling.
- `xml`: writers, readers, state handling, and term handling.
- `parser`: synchronous readers, asynchronous readers, and slice readers.

Existing documentation, doctests, feature gates, and assertions are retained.
Formatting follows rustfmt. Other source files are unchanged from prepared input.

September 30 local correction preserves literal boundary whitespace after XML
character-reference decoding. Non-literal trimming is unchanged. Registered
reader, slice, async and serializer roundtrip regressions cover exact lexical
values; this correction is not an upstream release or root dependency acceptance.

September 30 follow-up correction encodes every literal CR as `&#13;` at the
shared serializer seam, including interior CR and CRLF. XML escaping occurs
before insertion of CR references, preserving literal ampersands and
entity-looking text without escaping generated references again. The borrowed
path remains available when no escaping or boundary encoding is needed. Existing
boundary-whitespace encoding and parser XML newline normalization are retained.
Registered regressions cover exact lexical values and independently specified
serialized bodies for plain, typed and valid language literals, repeated CR,
metacharacters, Unicode, and boundary/interior combinations. Reader, slice and
async paths also cover raw XML CR/CRLF normalization to LF; sync and async writer
bytes are compared directly. Existing CDATA rejection assertions remain intact.

The newer upstream quick-xml port
`e115a6a8dd9213fdf89a20cb72494ab333878218` was not selected: it requires
`OxString`, unavailable in the retained oxrdf 0.3.3 dependency. The failed
compatibility probe remains negative evidence; it is not an accepted replacement.

Build and test acceptance belongs to source-bound delivery receipts, not this
provenance note. Root Cargo dependency selection and advisory closure remain
separate work.

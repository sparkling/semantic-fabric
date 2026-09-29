//! Per-dialect IRI percent-encoding of a runtime column value.
use super::*;

/// Percent-encode `col_sql`'s runtime value EXACTLY the way `sf_core::ir::
/// Template::expand`'s `encode_iri` arm does: RFC3987 *iunreserved* consists
/// of `ALPHA / DIGIT / "-" / "." / "_" / "~" / ucschar`. Other valid scalars
/// become uppercase percent-encoded UTF-8 bytes, never hexadecimal code points.
/// The shared core range table excludes private-use, C1 and noncharacters.
///
/// **History: why this is not a flat `REPLACE` chain.** An earlier version
/// nested one `REPLACE` call per encodable byte — `REPLACE` being ANSI-
/// portable, the obvious building block. That fails for two INDEPENDENT
/// reasons, both found empirically against [`Dialect::emit_via_ast`] (the
/// `sqlparser` AST round-trip every emitted statement goes through):
/// 1. **SQLite** has a hard recursion-depth ceiling (`sqlparser`'s
///    `DEFAULT_REMAINING_DEPTH`) — a flat chain over the full 62-byte set
///    (deeper than the empirically measured ~41-44-level ceiling inside a
///    realistic WHERE clause) errors "recursion limit exceeded" outright.
/// 2. **PostgreSQL** is far worse: not a lower ceiling but EXPONENTIAL
///    parse time in nesting depth, well before any hard limit fires
///    (measured: depth 8 ≈ 14ms, depth 12 ≈ 99ms, depth 20 ≈ over 16
///    SECONDS) — general to nested-function-call parsing in `sqlparser`'s
///    PG dialect (reproduced with a single-argument `UPPER(...)` chain, not
///    just `REPLACE`), so no flat chain wide enough for full coverage is
///    viable there at any practical depth.
///
/// **SQLite now uses a query-local native function instead** (see
/// [`percent_encode_col_sqlite`]); the per-character SQL below remains the
/// PostgreSQL and MySQL design.
///
/// **The fix: per-character SQL, not per-character SQL TEXT NESTING.** Each
/// dialect gets its OWN O(1)-parse-depth encoder — a single `WITH RECURSIVE`
/// (or, for PostgreSQL, `unnest(...) WITH ORDINALITY`) that iterates the
/// STRING'S OWN characters/bytes as ROWS, classifies each with one `CASE`,
/// and reassembles via an ORDER-preserving aggregate (`group_concat`/
/// `string_agg`/`GROUP_CONCAT`, all `... ORDER BY ...` — SQLite 3.44+, this
/// project's bundled 3.46.0 confirmed; PostgreSQL and MySQL support it
/// natively). Parse depth is CONSTANT regardless of the encode-set size or
/// the runtime string length — confirmed fast (single-digit milliseconds)
/// against the SAME realistic OR-IS-NULL-wrapped, multi-column WHERE clause
/// that broke the flat-chain design, for all three dialects.
///
/// **Byte- vs. character-oriented, per dialect — not interchangeable.**
/// SQLite's/MySQL's plain `LENGTH()`/`SUBSTRING()` on a TEXT argument are
/// NOT reliable byte-accurate iterators (SQLite's is character-counting and
/// silently truncates at an embedded NUL, exactly like a C string; MySQL's
/// `LENGTH()` is byte-oriented but its plain `SUBSTRING()` is CHARACTER-
/// oriented — an internally inconsistent pairing that walks past a
/// multi-byte character's true end) — both confirmed by direct, deliberate
/// probing before this design was settled on, both fixed by an explicit
/// `CAST(... AS BLOB)` (SQLite) / `CAST(... AS BINARY)` (MySQL) so every
/// function in the chain is consistently byte-oriented; a non-ASCII
/// multi-byte character is then walked and reassembled ONE RAW BYTE AT A
/// TIME. A byte passes through only when its complete UTF-8 scalar is in
/// RFC3987 ucschar; other valid scalars are percent-encoded byte by byte.
/// Invalid UTF-8 errors rather than becoming a valid escaped IRI. An isolated
/// intermediate byte cast is not independently valid UTF-8, but the final result
/// is. PostgreSQL's `text` is different on both counts: it cannot contain a
/// NUL byte at all (the server rejects it outright — confirmed live,
/// `ERROR: invalid byte sequence for encoding "UTF8": 0x00` — so there is
/// no NUL case to handle), and `string_to_array(text, NULL)` natively splits
/// by CHARACTER (not byte), which is the natural, already-decoded unit
/// there — `ascii(ch)` gives the correct code point for classification
/// (including non-ASCII), so no BLOB-equivalent cast is needed.
///
/// **NULL-propagation, correctly this time.** `sf_core::term::generate_into`
/// treats "any referenced column is NULL" as "this variable is UNBOUND" for
/// that row (`null_in_template_yields_no_term`, sf-core), so a NULL column
/// must render as SQL NULL — but the natural per-character aggregate
/// (`group_concat`/`string_agg`/`GROUP_CONCAT`) returns NULL over ZERO
/// input rows REGARDLESS of why there were zero rows: a genuinely NULL
/// column (nothing to iterate) and a genuinely EMPTY, non-NULL string
/// (also nothing to iterate, but should encode to `""`, not NULL) are
/// otherwise indistinguishable through the aggregate alone — confirmed by
/// direct probing: an early version without the NULL-vs-empty split below
/// wrongly rendered EACH of "NULL" and "empty string" as if the OTHER,
/// AND wrongly emitted a bare stray `%` for the empty-string case (the
/// iterator's own "at least one row" base case still fired past the end of
/// a zero-length string). Every implementation below is therefore the SAME
/// three-part shape: `CASE WHEN col IS NULL THEN NULL ELSE COALESCE(
/// <per-character aggregate>, '') END` — the outer CASE separates "no
/// value" from "empty value" (COALESCE alone cannot), and COALESCE only
/// then normalizes the (now unambiguous) zero-character case to `''`.
///
/// Every literal in these templates (hex-range bounds, the `-._~` char
/// list, dialect keywords) is a fixed SQL-syntax constant under this
/// module's own control, not query- or mapping-supplied data — inlining
/// them is the "fixed engine constant… part of the trusted skeleton" rule
/// this file's `LIKE ESCAPE '\'` rendering already uses (`render_cond`'s
/// `StrMatch` arm), not a departure from ADR-0010 R1 (which governs values
/// that originate from the SPARQL query or the mapping, neither of which
/// this function ever touches — the ONLY runtime input is the column
/// reference itself, already-resolved SQL text, not a bound value).
pub(super) fn percent_encode_col(col_sql: &str, dialect: Dialect) -> Result<String> {
    Ok(match dialect {
        Dialect::Sqlite => percent_encode_col_sqlite(col_sql),
        Dialect::MySql => percent_encode_col_mysql(col_sql),
        Dialect::Postgres => percent_encode_col_postgres(col_sql),
        other => {
            return Err(Error::Unsupported(format!(
                "IRI-template percent-encoding is not implemented for {other:?} → 501 \
                 (never a silently wrong un-encoded comparison)"
            )))
        }
    })
}

/// Pay the encoder expansion before building it. Each dialect's output is an
/// affine function of the column expression: a fixed body plus a constant
/// number of repetitions, calibrated once from the encoder itself.
pub(super) fn percent_encode_col_controlled(
    col_sql: &str,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    if dialect == Dialect::Sqlite {
        // The native encoder is a query-local lexical key function.
        catalog
            .lexical_keys
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    static SHAPES: std::sync::OnceLock<[(usize, usize); 3]> = std::sync::OnceLock::new();
    let shapes = SHAPES.get_or_init(|| {
        [Dialect::Sqlite, Dialect::MySql, Dialect::Postgres].map(|dialect| {
            let fixed = percent_encode_col("", dialect).map_or(0, |sql| sql.len());
            let repeats = percent_encode_col("x", dialect).map_or(0, |sql| sql.len() - fixed);
            (fixed, repeats)
        })
    });
    let (fixed, repeats) = match dialect {
        Dialect::Sqlite => shapes[0],
        Dialect::MySql => shapes[1],
        Dialect::Postgres => shapes[2],
        _ => (0, 0),
    };
    work.product(col_sql.len(), repeats)
        .map_err(source_control::validation_error)?;
    work.charge(fixed + 1)
        .map_err(source_control::validation_error)?;
    percent_encode_col(col_sql, dialect)
}

/// SQLite: one call to the query-local native `__sf_percent_encode_v1`, which
/// applies `sf_core::ir::encoding::percent_encode_iri` itself (installed and
/// removed with the lexical decoder keys; the emitter raises that flag). The
/// earlier per-byte `WITH RECURSIVE` template was ~13 KB per column reference,
/// so identity queries produced hundreds of kilobytes of SQL per request.
/// `CAST(... AS TEXT)` lets SQLite spell numbers before encoding.
pub(super) fn percent_encode_col_sqlite(col: &str) -> String {
    format!("__sf_percent_encode_v1(CAST({col} AS TEXT))")
}

/// MySQL: `CAST(... AS BINARY)` throughout — MySQL's `LENGTH()` is
/// byte-oriented but its plain `SUBSTRING()` is CHARACTER-oriented (a
/// confirmed-live, internally inconsistent pairing this cast reconciles,
/// mirroring the SQLite BLOB cast for the identical class of unit
/// mismatch); the pass-through branch and the `%XX` branch are BOTH kept as
/// `BINARY` inside `GROUP_CONCAT` so the aggregate never implicitly
/// re-interprets an in-flight (possibly standalone-invalid) byte as text,
/// and the FINAL aggregated result is converted back to `utf8mb4` once, at
/// the very end (mirrors the SQLite per-byte-cast pattern: only the fully
/// reassembled result needs to be valid UTF-8, confirmed live).
///
/// `JSON_TABLE(... FOR ORDINALITY)` supplies a row per byte. This is
/// deliberately not a correlated recursive CTE: MySQL 8.4 materializes such
/// a CTE using one outer row's length when the expression is projected for
/// several rows, which silently truncates/pads other values. `JSON_TABLE` is
/// implicitly lateral in MySQL and evaluates its document per outer row.
/// The `SET_VAR` optimizer hint requests `group_concat_max_len = 1,000,000`
/// for this query only; MySQL's 1024-byte default otherwise silently truncates
/// the result. The input guard does not trust the hint: it takes the minimum of
/// the hard 333,333-byte profile bound, the statement-observed
/// `@@SESSION.group_concat_max_len / 3`, and the statement-observed
/// `(@@SESSION.max_allowed_packet - 4096) / 3`. One source byte can expand to
/// three output bytes, and the 4-KiB packet reserve covers protocol/query
/// framing. Above that conservative dynamic bound the JSON document is
/// deliberately invalid, so execution fails before `GROUP_CONCAT` can return a
/// truncated IRI.
pub(super) const MYSQL_GROUP_CONCAT_MAX_LEN: usize = 1_000_000;
pub(super) const MYSQL_PERCENT_ENCODE_MAX_INPUT_BYTES: usize = MYSQL_GROUP_CONCAT_MAX_LEN / 3;
pub(super) const MYSQL_PACKET_RESERVE_BYTES: usize = 4_096;

pub(super) fn percent_encode_col_mysql(col: &str) -> String {
    // Bind the BINARY cast once as `pre.b` and reference that short alias
    // everywhere below, instead of re-embedding the (potentially long) column
    // expression at each of the ~40 byte-range/length checks. This is a pure
    // SQL-text-size optimization: it does not change which bytes are read or
    // how they are classified, only how many times the source expression is
    // spelled out. `CAST(NULL AS BINARY) IS NULL`, so the null check is
    // unaffected; `pre` is always exactly one row, so this remains a scalar
    // expression usable anywhere `{col}` was used directly before.
    let ucschar = encoding_ucschar::byte_member("pre.b", Dialect::MySql);
    let valid_utf8 = encoding_ucschar::valid_non_ascii_byte("pre.b", Dialect::MySql);
    format!(
        "(SELECT CASE WHEN pre.b IS NULL THEN NULL ELSE COALESCE((\
SELECT /*+ SET_VAR(group_concat_max_len = {group_limit}) */ \
CONVERT(CAST(GROUP_CONCAT(\
CASE \
WHEN HEX(SUBSTRING(pre.b, n, 1)) BETWEEN '30' AND '39' \
OR HEX(SUBSTRING(pre.b, n, 1)) BETWEEN '41' AND '5A' \
OR HEX(SUBSTRING(pre.b, n, 1)) BETWEEN '61' AND '7A' \
OR HEX(SUBSTRING(pre.b, n, 1)) IN ('2D', '2E', '5F', '7E') \
OR {ucschar} \
THEN SUBSTRING(pre.b, n, 1) \
WHEN HEX(SUBSTRING(pre.b, n, 1)) >= '80' AND NOT {valid_utf8} \
THEN JSON_EXTRACT('semantic-fabric-invalid-utf8', '$') \
ELSE CAST(CONCAT('%', HEX(SUBSTRING(pre.b, n, 1))) AS BINARY) \
END ORDER BY n SEPARATOR ''\
) AS BINARY) USING utf8mb4)\
FROM JSON_TABLE(\
CASE WHEN LENGTH(pre.b) = 0 THEN '[]' \
WHEN LENGTH(pre.b) > LEAST(\
{max_input}, \
GREATEST(CAST(@@SESSION.group_concat_max_len AS SIGNED), 0) DIV 3, \
GREATEST(CAST(@@SESSION.max_allowed_packet AS SIGNED) - {packet_reserve}, 0) DIV 3\
) \
THEN 'semantic-fabric-percent-encoding-input-limit' \
ELSE CONCAT('[0', REPEAT(',0', LENGTH(pre.b) - 1), ']') END, \
'$[*]' COLUMNS (n FOR ORDINALITY)\
) AS sfpe\
), '') END \
FROM (SELECT CAST({col} AS BINARY) AS b) AS pre)",
        group_limit = MYSQL_GROUP_CONCAT_MAX_LEN,
        max_input = MYSQL_PERCENT_ENCODE_MAX_INPUT_BYTES,
        packet_reserve = MYSQL_PACKET_RESERVE_BYTES,
    )
}

/// PostgreSQL: character-oriented (`string_to_array(text, NULL)` natively
/// splits by character — the correct unit for `text`, which is always
/// well-formed and cannot carry an embedded NUL at all, confirmed live).
/// `ascii(ch)` gives the numeric code point for classification (correct
/// for non-ASCII too, unlike a collation-dependent text comparison against
/// `chr(128)` would be). `unnest(...) WITH ORDINALITY` supplies the
/// position `string_agg(... ORDER BY ord)` reassembles by.
pub(super) fn percent_encode_col_postgres(col: &str) -> String {
    let ucschar = encoding_ucschar::codepoint_member("ascii(ch)");
    format!(
        "(SELECT CASE WHEN {col}::text IS NULL THEN NULL ELSE COALESCE((\
SELECT string_agg(\
CASE \
WHEN ascii(ch) BETWEEN 48 AND 57 \
OR ascii(ch) BETWEEN 65 AND 90 \
OR ascii(ch) BETWEEN 97 AND 122 \
OR ascii(ch) IN (45, 46, 95, 126) \
OR {ucschar} \
THEN ch \
ELSE (SELECT string_agg('%' || UPPER(LPAD(TO_HEX(get_byte(convert_to(ch, 'UTF8'), n)), 2, '0')), '' ORDER BY n) \
FROM generate_series(0, octet_length(ch) - 1) AS bytes(n)) \
END, '' ORDER BY ord\
) FROM unnest(string_to_array({col}::text, NULL)) WITH ORDINALITY AS t(ch, ord)\
), '') END)"
    )
}

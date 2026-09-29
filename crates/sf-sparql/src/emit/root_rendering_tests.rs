//! Root rendering, identifier resolution and SubPlan accounting checks.
use super::*;

#[test]
fn subplan_sql_pays_query_text_scans_and_parser_passes() {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let consumed = |padding: usize| {
        let maps = sf_mapping::parse_r2rml(&format!(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
                <#m> rr:logicalTable [rr:sqlQuery "SELECT v AS o FROM items WHERE '{}' <> ''"];
                rr:subject <http://ex/s>;
                rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "o"]]."#,
            "x".repeat(padding)
        ))
        .unwrap();
        let plan = crate::parse_and_translate(
            "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
            &maps,
            Dialect::Sqlite,
        )
        .unwrap();
        let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
        emit_subplan_sql_controlled(
            &plan,
            Dialect::Sqlite,
            &ColumnCatalog::default(),
            SourceWork::new(Some(&control)),
        )
        .unwrap();
        control.consumed(QueryCharge::SourceWork)
    };
    // Query text is scanned per referencing column and re-parsed with the SQL.
    let (short, long) = (consumed(0), consumed(1000));
    assert!(long >= short + 2 * 1000, "short {short} long {long}");
}

#[test]
fn postgres_placeholder_rebase_preserves_authored_bytes() {
    let prefix = "SELECT 'é$1', \"$2\", $$δ$3$$, $tag$🍀$4$tag$, E'it\\'s $5', table$6 /* $7 /* $8 */ */ -- $9\n";
    let sql = format!("{prefix}$1, $20");
    assert_eq!(
        rebase_placeholders(&sql, Dialect::Postgres, 2).unwrap(),
        format!("{prefix}$3, $22")
    );
    assert_eq!(rebase_placeholders(&sql, Dialect::MySql, 2).unwrap(), sql);
    assert!(rebase_placeholders("SELECT $1", Dialect::Postgres, usize::MAX).is_err());
}

/// A `LIKE` pushdown renders `ESCAPE '\'`, binds its pattern as a parameter,
/// and round-trips through the AST (ADR-0020 §2 / ADR-0010 R1).
#[test]
fn like_renders_escape_and_binds_pattern() {
    let cond = SqlCond::StrMatch {
        col: ColRef::new(0, "name"),
        op: StrMatchOp::Like,
        param: "%a\\%b%".to_owned(),
    };
    let e = emit_branch(&branch_with(cond), Dialect::Sqlite).unwrap();
    let up = e.sql.to_uppercase();
    assert!(up.contains("LIKE") && up.contains("ESCAPE"), "{}", e.sql);
    assert!(e.sql.contains('?'), "bound placeholder: {}", e.sql);
    assert!(
        !e.sql.contains("a%b"),
        "value must not be inlined: {}",
        e.sql
    );
    assert_eq!(e.params, vec!["%a\\%b%".to_owned()]);
}

/// Identifier resolution (SQL:2008 folding): an exact match wins (a case-exact /
/// delimited identifier), else a unique ASCII-case-insensitive match (a regular
/// identifier the DBMS folded), else the identifier as written.
#[test]
fn resolve_col_prefers_exact_then_case_insensitive() {
    let cols = vec!["studentid".to_owned(), "ID".to_owned(), "Name".to_owned()];
    // Regular `StudentId` → no exact, single CI match → the folded actual name.
    assert_eq!(resolve_col("StudentId", Some(&cols)), "studentid");
    // Delimited/case-exact `Name` → exact match wins (never folded away).
    assert_eq!(resolve_col("Name", Some(&cols)), "Name");
    // `ID` exact match.
    assert_eq!(resolve_col("ID", Some(&cols)), "ID");
    // No such column → emitted as written (the source surfaces the error).
    assert_eq!(resolve_col("missing", Some(&cols)), "missing");
    // Unknown source → as written.
    assert_eq!(resolve_col("Whatever", None), "Whatever");
}

/// A branch emitted with a catalog resolves its regular-identifier column
/// reference to the folded column the source actually exposes.
#[test]
fn emit_branch_with_resolves_folded_identifier() {
    let mut b = Branch::single(Scan {
        alias: 0,
        source: (LogicalSource::Table("Student".to_owned())).into(),
    });
    b.where_conds
        .push(SqlCond::IsNotNull(ColRef::new(0, "StudentId")));
    let mut catalog = ColumnCatalog::default();
    catalog.insert(
        &LogicalSource::Table("Student".to_owned()),
        vec!["studentid".to_owned()],
    );
    let e = emit_branch_with(&b, Dialect::Postgres, &catalog).unwrap();
    assert!(e.sql.contains("\"studentid\""), "{}", e.sql);
    assert!(!e.sql.contains("\"StudentId\""), "{}", e.sql);
    // Reconstruction still keys on the raw IR identifier (position-based read).
    assert_eq!(&*e.projection[0].column, "StudentId");
}

#[test]
fn live_validation_preserves_physical_row_identifier_exceptions() {
    let mut branch = Branch::single(Scan {
        alias: 0,
        source: (LogicalSource::Table("no_pk".to_owned())).into(),
    });
    branch.bindings.insert(
        "s".to_owned(),
        TermDef::Derived {
            term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
            alias: 0,
        },
    );
    branch.path = Some(PathClosure {
        alias: 1,
        kind: PathKind::One,
        hop: HopExpr::Pred(crate::iq::HopRelation {
            source: LogicalSource::Table("no_pk".to_owned()),
            subj_col: "rowid".into(),
            obj_col: "value".into(),
        }),
    });
    let mut catalog = ColumnCatalog::default();
    catalog.insert(
        &LogicalSource::Table("no_pk".to_owned()),
        vec!["value".to_owned()],
    );

    assert!(
        validate_live_columns(std::slice::from_ref(&branch), Dialect::Sqlite, &catalog).is_ok()
    );
    assert!(
        validate_live_columns(std::slice::from_ref(&branch), Dialect::Postgres, &catalog).is_ok()
    );
    assert!(validate_live_columns(&[branch], Dialect::MySql, &catalog).is_err());
}

#[test]
fn postgres_rowid_rewrite_stops_at_a_derived_query_boundary() {
    let mut table = Branch::single(Scan {
        alias: 0,
        source: (LogicalSource::Table("no_pk".to_owned())).into(),
    });
    table
        .where_conds
        .push(SqlCond::IsNotNull(ColRef::new(0, "rowid")));
    let table_sql = emit_branch(&table, Dialect::Postgres).unwrap().sql;
    assert!(table_sql.contains("(t0.ctid)::TEXT"), "{table_sql}");

    let mut query = Branch::single(Scan {
        alias: 0,
        source: (LogicalSource::Query(
            "SELECT (sfs0.ctid)::text AS rowid FROM no_pk sfs0".to_owned(),
        ))
        .into(),
    });
    query
        .where_conds
        .push(SqlCond::IsNotNull(ColRef::new(0, "rowid")));
    let query_sql = emit_branch(&query, Dialect::Postgres).unwrap().sql;
    assert!(query_sql.contains("t0.\"rowid\""), "{query_sql}");
    assert!(!query_sql.contains("t0.ctid"), "{query_sql}");
}

#[test]
fn translate_time_rowid_rendering_is_base_table_only() {
    assert_eq!(
        render_immediate_source_column("sfs0", "rowid", None, Dialect::Postgres),
        "(sfs0.ctid)::text"
    );
    assert_eq!(
        render_immediate_source_column(
            "sfs0",
            "rowid",
            Some("SELECT 7 AS rowid"),
            Dialect::Postgres,
        ),
        "sfs0.rowid"
    );
    assert_eq!(
        render_immediate_source_column(
            "sfs0",
            "rowid",
            Some("SELECT 7 AS \"rowid\""),
            Dialect::Postgres,
        ),
        "sfs0.\"rowid\""
    );

    let template = vec![
        sf_core::ir::Segment::Literal("urn:row:".into()),
        sf_core::ir::Segment::Column("rowid".into()),
    ];
    let raw = sf_sql::source_work::SourceWork::new(None);
    let table_sql = render_template_inline(
        &template,
        false,
        Dialect::Postgres,
        &ColumnCatalog::default(),
        |column| render_immediate_source_column("sfs0", column, None, Dialect::Postgres),
        raw,
    )
    .unwrap();
    assert!(table_sql.contains("(sfs0.ctid)::text"), "{table_sql}");
    let query_sql = render_template_inline(
        &template,
        false,
        Dialect::Postgres,
        &ColumnCatalog::default(),
        |column| {
            render_immediate_source_column(
                "sfs0",
                column,
                Some("SELECT 7 AS rowid"),
                Dialect::Postgres,
            )
        },
        raw,
    )
    .unwrap();
    assert!(query_sql.contains("sfs0.rowid"), "{query_sql}");
    assert!(!query_sql.contains("ctid"), "{query_sql}");
}

/// PostgreSQL regex pushdown renders `~` / `~*` with a numbered, bound param.
#[test]
fn pg_regex_renders_operator_and_binds_pattern() {
    let e = emit_branch(
        &branch_with(SqlCond::StrMatch {
            col: ColRef::new(0, "name"),
            op: StrMatchOp::RegexMatchI,
            param: "^a.*".to_owned(),
        }),
        Dialect::Postgres,
    )
    .unwrap();
    assert!(e.sql.contains("~*"), "{}", e.sql);
    assert!(
        e.sql.contains("$1"),
        "numbered bound placeholder: {}",
        e.sql
    );
    assert!(
        !e.sql.contains("^a"),
        "pattern must not be inlined: {}",
        e.sql
    );
    assert_eq!(e.params, vec!["^a.*".to_owned()]);
}

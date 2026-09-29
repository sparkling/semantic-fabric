//! Branch emission entry points and the governed AST canonicalization boundary.
use super::*;

/// A branch rendered to one parameterised SQL `SELECT`.
pub struct EmittedBranch {
    pub sql: String,
    /// Prepare-only same-IR SQL without engine-added collation decorations.
    /// Never executed; preserves SQLite native result decoding through COLLATE.
    pub metadata_sql: Option<String>,
    /// Engine-generated decoder call, never inferred from authored SQL text.
    pub sqlite_character_keys: bool,
    /// Query-owned lexical decoder and literal numeric comparison callbacks.
    pub sqlite_lexical_keys: bool,
    /// The result-set schema: column `i` is `projection[i]` (positional — the
    /// reconstruction reads by position, not by the cosmetic `AS c{i}` label).
    pub projection: Vec<ColRef>,
    /// Bound parameter values, in placeholder order.
    pub params: Vec<String>,
}

/// Render one branch with no column-name resolution (the dialect-neutral path,
/// used where a live catalog is unavailable — every identifier emitted as written).
pub fn emit_branch(b: &Branch, dialect: Dialect) -> Result<EmittedBranch> {
    emit_branch_with(b, dialect, &ColumnCatalog::default())
}

/// Render one branch, resolving each column reference against `catalog` so a
/// mapping's regular identifiers bind to the columns the live source exposes after
/// its identifier folding (see the module docs).
pub fn emit_branch_with(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<EmittedBranch> {
    emit_branch_with_modifiers(b, dialect, catalog, BranchModifiers::stored(b))
}

pub(crate) fn emit_branch_with_modifiers(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
) -> Result<EmittedBranch> {
    emit_branch_controlled(
        b,
        dialect,
        catalog,
        modifiers,
        sf_sql::source_work::SourceWork::new(None),
    )
}

/// Synchronous entry point, preserved exactly for the raw/offline API and
/// every existing test call site: bridges the async emission chain via the
/// crate's existing `block_on` shim (`exec_core::driver`). When `work` has no
/// isolation capability (the common case for these callers) nothing in the
/// async chain ever actually awaits, so this is a plain, immediate call, not
/// a real suspension.
pub(crate) fn emit_branch_controlled(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    crate::exec_core::block_on(emit_branch_binding_view(
        b,
        &BindingView::Direct(&b.bindings),
        dialect,
        catalog,
        modifiers,
        work,
    ))
}

pub(crate) async fn emit_branch_binding_view(
    b: &Branch,
    bindings: &BindingView<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    let mut emission_catalog = catalog.clone();
    emission_catalog.character_keys = Default::default();
    emission_catalog.lexical_keys = Default::default();
    let catalog = &emission_catalog;
    let mut emitted = emit_branch_inner(b, bindings, dialect, catalog, modifiers, work).await?;
    if dialect == Dialect::Sqlite && !catalog.suppress_path_collation {
        let mut metadata_catalog = catalog.clone();
        metadata_catalog.suppress_path_collation = true;
        let metadata =
            emit_branch_inner(b, bindings, dialect, &metadata_catalog, modifiers, work).await?;
        if metadata.projection != emitted.projection || metadata.params != emitted.params {
            return Err(Error::Sql(
                "path metadata twin changed projection or parameters".into(),
            ));
        }
        if metadata.sql != emitted.sql {
            emitted.metadata_sql = Some(metadata.sql);
        }
    }
    emitted.sqlite_character_keys = catalog
        .character_keys
        .load(std::sync::atomic::Ordering::Relaxed);
    emitted.sqlite_lexical_keys = catalog
        .lexical_keys
        .load(std::sync::atomic::Ordering::Relaxed);
    Ok(emitted)
}

/// Parse and canonically re-render `skeleton` (ADR-0010 §A), governed by the
/// isolated SQL-canonicalization peer when `work`'s control carries a
/// `ParserRuntime` capability (see `QueryControl::capability`); otherwise the
/// exact prior in-process `Dialect::emit_via_ast` behavior (raw/offline/CLI/
/// test callers, matching `crate::parse_query`'s own `parse_in_scope`
/// fallback precedent). `source_control::sql_parse`'s prepay charge always
/// applies first, independent of which path renders the SQL. The isolated
/// path's own request/result frame costs are charged prospectively inside
/// the supervisor round trip itself (`parser_isolation::supervisor::
/// sql_canonicalize::canonicalize`), at the exact point each length becomes
/// known -- never approximated or charged after the fact here. Async so the
/// isolated path's bounded child-I/O steps can cooperatively yield instead of
/// blocking the caller's own thread; the synchronous raw/offline API
/// (`emit_branch_controlled`) bridges this with the crate's existing
/// `block_on` shim, unaffected by any of this when no capability is present.
pub(super) async fn emit_via_ast_governed(
    dialect: Dialect,
    skeleton: &str,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    source_control::sql_parse(skeleton, work).map_err(source_control::validation_error)?;
    if let Some(control) = work.control() {
        if let Some(result) =
            parser_isolation::runtime::canonicalize_sql_in_scope(dialect, skeleton, control, work)
                .await
        {
            return result;
        }
    }
    dialect
        .emit_via_ast(skeleton)
        .map_err(|e| Error::Sql(e.to_string()))
}

#[cfg(feature = "runtime-identity-evidence")]
pub(crate) fn exercise_raw_sql_fallback_for_evidence(
    control: &dyn sf_core::query_control::QueryControl,
) -> Result<String> {
    crate::exec_core::block_on(emit_via_ast_governed(
        Dialect::Sqlite,
        "SELECT 1 AS c0",
        sf_sql::source_work::SourceWork::new(Some(control)),
    ))
}

pub(super) async fn emit_branch_inner(
    b: &Branch,
    bindings: &BindingView<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    emit_branch_keys(
        b,
        bindings,
        dialect,
        catalog,
        modifiers.distinct,
        modifiers,
        work,
    )
    .await
}

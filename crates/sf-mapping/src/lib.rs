//! `sf-mapping` — parse R2RML (and Direct Mapping as auto-generated R2RML) into
//! the `sf-core` mapping IR (ADR-0003 R1; the `sf-mapping` row of ADR-0006).
//!
//! Turtle is read with `oxttl` (RDF 1.2; ADR-0004 / ADR-0019). This crate is the
//! single place mapping documents are parsed — the virtualiser (`sf-sparql`)
//! consumes the IR and never re-parses (ADR-0003 R1). RDF terms stay `oxrdf`
//! types end to end (ADR-0003 R2). Scope is R2RML-only: no RML reference
//! formulation / heterogeneous-source generality (ADR-0002).

pub mod direct_mapping;
pub mod r2rml;

pub use direct_mapping::direct_mapping;
pub use r2rml::parse_r2rml;

use sf_core::{Result, SourceId, SourceMapping};
use sf_sql::TableSchema;

/// Parse R2RML and associate the unchanged IR with one pre-admission source ID.
pub fn parse_r2rml_for_source(turtle: &str, source_id: SourceId) -> Result<SourceMapping> {
    parse_r2rml(turtle).map(|maps| SourceMapping::new(source_id, maps))
}

/// Generate Direct Mapping IR and associate it with one pre-admission source ID.
pub fn direct_mapping_for_source(
    tables: &[TableSchema],
    base_iri: &str,
    source_id: SourceId,
) -> Result<SourceMapping> {
    direct_mapping(tables, base_iri).map(|maps| SourceMapping::new(source_id, maps))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPING: &str = r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#items> rr:logicalTable [ rr:tableName "items" ];
            rr:subjectMap [ rr:template "http://example.com/items/{id}" ].
    "#;

    #[test]
    fn source_aware_parser_is_a_sidecar_over_the_existing_ir() {
        let source_id = SourceId::new(3).unwrap();
        let source_mapping = parse_r2rml_for_source(MAPPING, source_id).unwrap();
        let plain = parse_r2rml(MAPPING).unwrap();

        assert_eq!(source_mapping.source_id(), source_id);
        assert_eq!(source_mapping.len(), plain.len());
        assert_eq!(source_mapping.triples_maps()[0].id, plain[0].id);
        assert!(matches!(
            (&source_mapping.triples_maps()[0].source, &plain[0].source),
            (sf_core::ir::LogicalSource::Table(source), sf_core::ir::LogicalSource::Table(plain))
                if source == plain
        ));
    }

    #[test]
    fn source_aware_direct_mapping_preserves_the_generated_ir() {
        let table = TableSchema::new("items");
        let source_id = SourceId::new(4).unwrap();
        let source_mapping =
            direct_mapping_for_source(&[table], "http://example.com/base/", source_id).unwrap();

        assert_eq!(source_mapping.source_id(), source_id);
        assert_eq!(source_mapping.len(), 1);
        assert!(matches!(
            &source_mapping.triples_maps()[0].source,
            sf_core::ir::LogicalSource::Table(table) if table == "items"
        ));
    }
}

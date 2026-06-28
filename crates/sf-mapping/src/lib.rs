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

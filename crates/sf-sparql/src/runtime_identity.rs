//! Deterministic identity for the immutable inputs to one compiler binding.

use std::collections::HashMap;
use std::fmt;

use sf_core::ir::{LogicalSource, ObjectMap, Segment, TermMap, TermSpec, TermType, TriplesMap};
use sf_core::SourceMapping;
use sf_sql::{Dialect, TableSchema};
use sha2::{Digest, Sha256};

use crate::{ColumnTypeAuthority, ConstraintAuthority, Tbox};

const IDENTITY_VERSION: &[u8] = b"semantic-fabric/runtime-identity/v1";

macro_rules! digest_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Eq, Hash, PartialEq)]
        pub struct $name([u8; 32]);

        impl $name {
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "("))?;
                write_hex(formatter, &self.0)?;
                formatter.write_str(")")
            }
        }
    };
}

digest_type!(OntologyDigest);
digest_type!(MappingDigest);
digest_type!(StructuralSchemaDigest);
digest_type!(TypeSchemaDigest);
digest_type!(SchemaDigest);
digest_type!(ConstraintPolicyDigest);
digest_type!(CapabilityDigest);

/// Content identity carried by cache keys and compiled plans.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompileDigests {
    ontology: OntologyDigest,
    mapping: MappingDigest,
    structural_schema: StructuralSchemaDigest,
    type_schema: TypeSchemaDigest,
    schema: SchemaDigest,
    constraint_policy: ConstraintPolicyDigest,
    capability: CapabilityDigest,
}

impl CompileDigests {
    pub const fn ontology(self) -> OntologyDigest {
        self.ontology
    }

    pub const fn mapping(self) -> MappingDigest {
        self.mapping
    }

    pub const fn structural_schema(self) -> StructuralSchemaDigest {
        self.structural_schema
    }

    pub const fn type_schema(self) -> TypeSchemaDigest {
        self.type_schema
    }

    pub const fn schema(self) -> SchemaDigest {
        self.schema
    }

    pub const fn constraint_policy(self) -> ConstraintPolicyDigest {
        self.constraint_policy
    }

    pub const fn capability(self) -> CapabilityDigest {
        self.capability
    }

    pub(crate) fn from_inputs(
        mapping: &SourceMapping,
        tbox: &Tbox,
        schema: &[TableSchema],
        dialect: Dialect,
        constraint_authority: ConstraintAuthority,
        column_type_authority: ColumnTypeAuthority,
    ) -> Self {
        let ontology = ontology_digest(tbox);
        let mapping = mapping_digest(mapping);
        let structural_schema = structural_schema_digest(schema);
        let type_schema = type_schema_digest(schema);
        let schema = schema_digest(schema, structural_schema, type_schema);
        let constraint_policy =
            constraint_policy_digest(constraint_authority, column_type_authority);
        let capability = capability_digest(dialect);
        Self {
            ontology,
            mapping,
            structural_schema,
            type_schema,
            schema,
            constraint_policy,
            capability,
        }
    }
}

fn ontology_digest(tbox: &Tbox) -> OntologyDigest {
    let mut out = CanonicalHasher::new(b"ontology");
    encode_multimap(&mut out, &tbox.sub_classes);
    encode_multimap(&mut out, &tbox.sub_properties);

    let mut inverses: Vec<_> = tbox.inverses.iter().collect();
    inverses.sort_unstable_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(right.1)));
    out.len(inverses.len());
    for (predicate, inverse) in inverses {
        out.text(predicate);
        out.text(inverse);
    }

    let mut symmetric: Vec<_> = tbox.symmetric.iter().map(String::as_str).collect();
    symmetric.sort_unstable();
    out.len(symmetric.len());
    for predicate in symmetric {
        out.text(predicate);
    }
    OntologyDigest(out.finish())
}

fn encode_multimap(out: &mut CanonicalHasher, values: &HashMap<String, Vec<String>>) {
    let mut entries: Vec<_> = values.iter().collect();
    entries.sort_unstable_by_key(|(key, _)| key.as_str());
    out.len(entries.len());
    for (key, members) in entries {
        out.text(key);
        let mut members: Vec<_> = members.iter().map(String::as_str).collect();
        members.sort_unstable();
        out.len(members.len());
        for member in members {
            out.text(member);
        }
    }
}

fn mapping_digest(mapping: &SourceMapping) -> MappingDigest {
    let mut out = CanonicalHasher::new(b"mapping");
    out.u64(mapping.source_id().index() as u64);
    out.len(mapping.triples_maps().len());
    for triples_map in mapping.triples_maps() {
        encode_triples_map(&mut out, triples_map);
    }
    MappingDigest(out.finish())
}

fn encode_triples_map(out: &mut CanonicalHasher, triples_map: &TriplesMap) {
    out.text(&triples_map.id);
    match &triples_map.source {
        LogicalSource::Table(table) => {
            out.tag(0);
            out.text(table);
        }
        LogicalSource::Query(query) => {
            out.tag(1);
            out.text(query);
        }
    }
    encode_term_map(out, &triples_map.subject.term);
    out.len(triples_map.subject.classes.len());
    for class in &triples_map.subject.classes {
        out.text(class.as_str());
    }
    encode_term_maps(out, &triples_map.subject.graphs);
    out.len(triples_map.predicate_object_maps.len());
    for predicate_object in &triples_map.predicate_object_maps {
        encode_term_maps(out, &predicate_object.predicates);
        out.len(predicate_object.objects.len());
        for object in &predicate_object.objects {
            match object {
                ObjectMap::Term(term) => {
                    out.tag(0);
                    encode_term_map(out, term);
                }
                ObjectMap::Ref(reference) => {
                    out.tag(1);
                    out.text(&reference.parent_triples_map);
                    out.len(reference.joins.len());
                    for join in &reference.joins {
                        out.text(&join.child);
                        out.text(&join.parent);
                    }
                }
            }
        }
        encode_term_maps(out, &predicate_object.graphs);
    }
}

fn encode_term_maps(out: &mut CanonicalHasher, maps: &[TermMap]) {
    out.len(maps.len());
    for map in maps {
        encode_term_map(out, map);
    }
}

fn encode_term_map(out: &mut CanonicalHasher, map: &TermMap) {
    match map {
        TermMap::Constant(term) => {
            out.tag(0);
            out.text(&term.to_string());
        }
        TermMap::Column(column, spec) => {
            out.tag(1);
            out.text(column);
            encode_term_spec(out, spec);
        }
        TermMap::Template(template, spec) => {
            out.tag(2);
            out.len(template.segments().len());
            for segment in template.segments() {
                match segment {
                    Segment::Literal(value) => {
                        out.tag(0);
                        out.text(value);
                    }
                    Segment::Column(value) => {
                        out.tag(1);
                        out.text(value);
                    }
                }
            }
            encode_term_spec(out, spec);
        }
    }
}

fn encode_term_spec(out: &mut CanonicalHasher, spec: &TermSpec) {
    out.tag(match spec.term_type {
        TermType::Iri => 0,
        TermType::BlankNode => 1,
        TermType::Literal => 2,
    });
    out.optional_text(spec.datatype.as_ref().map(|value| value.as_str()));
    out.optional_text(spec.language.as_deref());
    out.optional_text(spec.base.as_deref());
}

fn structural_schema_digest(schema: &[TableSchema]) -> StructuralSchemaDigest {
    let mut out = CanonicalHasher::new(b"structural-schema");
    out.len(schema.len());
    for table in schema {
        out.text(&table.name);
        out.len(table.columns.len());
        for column in &table.columns {
            out.text(&column.name);
            out.boolean(column.not_null);
        }
        encode_strings(&mut out, &table.primary_key);
        out.len(table.unique.len());
        for unique in &table.unique {
            encode_strings(&mut out, unique);
        }
        out.len(table.foreign_keys.len());
        for foreign_key in &table.foreign_keys {
            encode_strings(&mut out, &foreign_key.columns);
            out.text(&foreign_key.parent_table);
            encode_strings(&mut out, &foreign_key.parent_columns);
        }
        out.len(table.functional_dependencies.len());
        for dependency in &table.functional_dependencies {
            encode_strings(&mut out, &dependency.det);
            encode_strings(&mut out, &dependency.dep);
        }
    }
    StructuralSchemaDigest(out.finish())
}

fn type_schema_digest(schema: &[TableSchema]) -> TypeSchemaDigest {
    let mut out = CanonicalHasher::new(b"type-schema");
    out.len(schema.len());
    for table in schema {
        out.text(&table.name);
        out.len(table.columns.len());
        for column in &table.columns {
            out.text(&column.name);
            out.text(&column.sql_type);
        }
    }
    TypeSchemaDigest(out.finish())
}

fn schema_digest(
    schema: &[TableSchema],
    structural: StructuralSchemaDigest,
    types: TypeSchemaDigest,
) -> SchemaDigest {
    let mut out = CanonicalHasher::new(b"schema");
    out.bytes(structural.as_bytes());
    out.bytes(types.as_bytes());
    out.len(schema.len());
    for table in schema {
        out.optional_u64(table.row_estimate);
        out.len(table.columns.len());
        for column in &table.columns {
            out.optional_u64(column.distinct_estimate);
        }
    }
    SchemaDigest(out.finish())
}

fn constraint_policy_digest(
    constraints: ConstraintAuthority,
    column_types: ColumnTypeAuthority,
) -> ConstraintPolicyDigest {
    let mut out = CanonicalHasher::new(b"constraint-policy");
    out.tag(match constraints {
        ConstraintAuthority::Unverified => 0,
    });
    out.tag(match column_types {
        ColumnTypeAuthority::Unverified => 0,
    });
    ConstraintPolicyDigest(out.finish())
}

fn capability_digest(dialect: Dialect) -> CapabilityDigest {
    let mut out = CanonicalHasher::new(b"compiler-capabilities");
    out.text(dialect_name(dialect));
    out.boolean(dialect.supports_recursive_paths());
    out.boolean(dialect.like_is_case_sensitive());
    CapabilityDigest(out.finish())
}

fn dialect_name(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Postgres => "postgres",
        Dialect::Sqlite => "sqlite",
        Dialect::MySql => "mysql",
        Dialect::Redshift => "redshift",
        Dialect::DuckDb => "duckdb",
        Dialect::SqlServer => "sqlserver",
        Dialect::Oracle => "oracle",
        Dialect::SapHana => "sap-hana",
        Dialect::MonetDb => "monetdb",
        Dialect::Snowflake => "snowflake",
        Dialect::BigQuery => "bigquery",
        Dialect::Athena => "athena",
        Dialect::Databricks => "databricks",
        Dialect::Trino => "trino",
        Dialect::PrestoDB => "presto-db",
        Dialect::Db2 => "db2",
        Dialect::H2 => "h2",
        Dialect::Spark => "spark",
        Dialect::Dremio => "dremio",
        Dialect::Denodo => "denodo",
        Dialect::Teiid => "teiid",
    }
}

fn encode_strings(out: &mut CanonicalHasher, values: &[String]) {
    out.len(values.len());
    for value in values {
        out.text(value);
    }
}

struct CanonicalHasher(Sha256);

impl CanonicalHasher {
    fn new(domain: &[u8]) -> Self {
        let mut out = Self(Sha256::new());
        out.bytes(IDENTITY_VERSION);
        out.bytes(domain);
        out
    }

    fn tag(&mut self, value: u8) {
        self.0.update([value]);
    }

    fn boolean(&mut self, value: bool) {
        self.tag(u8::from(value));
    }

    fn len(&mut self, value: usize) {
        self.u64(u64::try_from(value).expect("in-memory identity input length fits in u64"));
    }

    fn u64(&mut self, value: u64) {
        self.0.update(value.to_be_bytes());
    }

    fn bytes(&mut self, value: &[u8]) {
        self.len(value.len());
        self.0.update(value);
    }

    fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn optional_text(&mut self, value: Option<&str>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.text(value);
        }
    }

    fn optional_u64(&mut self, value: Option<u64>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.u64(value);
        }
    }

    fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

fn write_hex(formatter: &mut fmt::Formatter<'_>, value: &[u8]) -> fmt::Result {
    for byte in value {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

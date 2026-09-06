//! Versioned inventory for one complete legacy catalogue projection.

use super::legacy_sql::{
    COLUMNS_SQL, EARLIER_RELATION_COLLISIONS_SQL, FOREIGN_KEYS_SQL, KEYS_SQL, NDISTINCT_SQL,
    RELTUPLES_SQL, TABLES_SQL,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum LegacyCatalogueQueryV1 {
    Tables,
    EarlierRelationCollisions,
    Columns,
    Keys,
    ForeignKeys,
    RelationStatistics,
    ColumnStatistics,
    Count,
}

pub(super) const LEGACY_CATALOGUE_QUERY_INVENTORY_V1: [LegacyCatalogueQueryV1; 7] = [
    LegacyCatalogueQueryV1::Tables,
    LegacyCatalogueQueryV1::EarlierRelationCollisions,
    LegacyCatalogueQueryV1::Columns,
    LegacyCatalogueQueryV1::Keys,
    LegacyCatalogueQueryV1::ForeignKeys,
    LegacyCatalogueQueryV1::RelationStatistics,
    LegacyCatalogueQueryV1::ColumnStatistics,
];
const _: () =
    assert!(LEGACY_CATALOGUE_QUERY_INVENTORY_V1.len() == LegacyCatalogueQueryV1::Count as usize);

impl LegacyCatalogueQueryV1 {
    pub(super) const fn sql(self) -> &'static str {
        match self {
            Self::Tables => TABLES_SQL,
            Self::EarlierRelationCollisions => EARLIER_RELATION_COLLISIONS_SQL,
            Self::Columns => COLUMNS_SQL,
            Self::Keys => KEYS_SQL,
            Self::ForeignKeys => FOREIGN_KEYS_SQL,
            Self::RelationStatistics => RELTUPLES_SQL,
            Self::ColumnStatistics => NDISTINCT_SQL,
            Self::Count => panic!("legacy catalogue query count sentinel is not executable"),
        }
    }
}

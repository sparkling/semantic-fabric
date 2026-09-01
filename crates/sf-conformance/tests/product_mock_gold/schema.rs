use std::collections::BTreeSet;

use serde_json::Value;

use super::{array, boolean, string, strings};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationalColumn {
    pub name: String,
    pub store_type: String,
    pub nullable: bool,
}

pub type StyleColumn = RelationalColumn;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationSchema {
    pub name: String,
    pub columns: Vec<RelationalColumn>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreSchema {
    pub name: String,
    pub relations: Vec<RelationSchema>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationalInventory {
    pub stores: Vec<StoreSchema>,
}

impl RelationalInventory {
    pub fn table_count(&self) -> usize {
        self.stores.iter().map(|store| store.relations.len()).sum()
    }

    pub fn column_count(&self) -> usize {
        self.stores
            .iter()
            .flat_map(|store| &store.relations)
            .map(|relation| relation.columns.len())
            .sum()
    }

    pub fn store(&self, name: &str) -> Option<&StoreSchema> {
        self.stores.iter().find(|store| store.name == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StyleForeignKey {
    pub name: String,
    pub child_columns: Vec<String>,
    pub parent_table: String,
    pub parent_columns: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleSchema {
    pub columns: Vec<StyleColumn>,
    pub primary_key: Vec<String>,
    pub foreign_keys: Vec<StyleForeignKey>,
}

const EXPECTED_STORES: [&str; 11] = [
    "CostingPricing",
    "FinanceCostAccounting",
    "LogisticsAllocation",
    "MaterialsBom",
    "MerchandisingAssortment",
    "ProductDesign",
    "ProductionManufacturing",
    "QualityCompliance",
    "SamplingFit",
    "Style360",
    "SupplierSourcing",
];

pub(super) fn parse_inventory(coverage: &Value) -> Result<RelationalInventory, &'static str> {
    let mut stores = array(coverage, "/relationalSchema/stores")?
        .iter()
        .map(parse_store)
        .collect::<Result<Vec<_>, _>>()?;
    stores.sort_by(|left, right| left.name.cmp(&right.name));
    let names: Vec<_> = stores.iter().map(|store| store.name.as_str()).collect();
    if names != EXPECTED_STORES {
        return Err("relational store inventory mismatch");
    }
    let inventory = RelationalInventory { stores };
    if inventory.table_count() != 112 {
        return Err("relational inventory table count mismatch");
    }
    if inventory.column_count() != 598 {
        return Err("relational inventory column count mismatch");
    }
    Ok(inventory)
}

fn parse_store(value: &Value) -> Result<StoreSchema, &'static str> {
    let name = required_name(value, "/store")?;
    let mut relations = array(value, "/relations")?
        .iter()
        .map(parse_relation)
        .collect::<Result<Vec<_>, _>>()?;
    relations.sort_by(|left, right| left.name.cmp(&right.name));
    if has_duplicate(relations.iter().map(|relation| relation.name.as_str())) {
        return Err("relational table inventory contains duplicates");
    }
    Ok(StoreSchema { name, relations })
}

fn parse_relation(value: &Value) -> Result<RelationSchema, &'static str> {
    let name = required_name(value, "/name")?;
    let columns = array(value, "/columns")?
        .iter()
        .map(parse_column)
        .collect::<Result<Vec<_>, _>>()?;
    if columns.is_empty() {
        return Err("relational table has no columns");
    }
    if has_duplicate(columns.iter().map(|column| column.name.as_str())) {
        return Err("relational column inventory contains duplicates");
    }
    Ok(RelationSchema { name, columns })
}

fn parse_column(value: &Value) -> Result<RelationalColumn, &'static str> {
    Ok(RelationalColumn {
        name: required_name(value, "/name")?,
        store_type: required_name(value, "/storeType")?,
        nullable: boolean(value, "/nullable")?,
    })
}

fn required_name(value: &Value, pointer: &str) -> Result<String, &'static str> {
    let name = string(value, pointer)?;
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err("relational schema name is invalid");
    }
    Ok(name.to_owned())
}

fn has_duplicate<'a>(values: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    values.into_iter().any(|value| !seen.insert(value))
}

pub(super) fn parse_style_schema(style: &Value) -> Result<StyleSchema, &'static str> {
    let columns = array(style, "/columns")?
        .iter()
        .map(parse_column)
        .collect::<Result<Vec<_>, &'static str>>()?;
    if columns
        != [
            ("style_number", "text", false),
            ("season_code", "text", false),
            ("design_brief_id", "uuid", false),
            ("status", "text", false),
            ("version", "integer", false),
        ]
        .map(|(name, store_type, nullable)| StyleColumn {
            name: name.to_owned(),
            store_type: store_type.to_owned(),
            nullable,
        })
    {
        return Err("Style column contract mismatch");
    }
    let primary_key = strings(style, "/primaryKey/columns")?;
    if primary_key != ["style_number"] {
        return Err("Style primary key mismatch");
    }
    let mut foreign_keys = array(style, "/foreignKeyDefinitions")?
        .iter()
        .map(|value| {
            Ok(StyleForeignKey {
                name: string(value, "/name")?.to_owned(),
                child_columns: strings(value, "/childColumns")?,
                parent_table: string(value, "/parentTable")?.to_owned(),
                parent_columns: strings(value, "/parentColumns")?,
            })
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    foreign_keys.sort();
    let expected = vec![
        StyleForeignKey {
            name: "FK_style_design_brief".to_owned(),
            child_columns: vec!["design_brief_id".to_owned()],
            parent_table: "design_brief".to_owned(),
            parent_columns: vec!["id".to_owned()],
        },
        StyleForeignKey {
            name: "FK_style_season_ref".to_owned(),
            child_columns: vec!["season_code".to_owned()],
            parent_table: "season_ref".to_owned(),
            parent_columns: vec!["season_code".to_owned()],
        },
    ];
    if foreign_keys != expected {
        return Err("Style foreign key contract mismatch");
    }
    Ok(StyleSchema {
        columns,
        primary_key,
        foreign_keys,
    })
}

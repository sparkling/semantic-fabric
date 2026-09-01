use serde_json::Value;

use super::{array, boolean, string, strings};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleColumn {
    pub name: String,
    pub store_type: String,
    pub nullable: bool,
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

pub(super) fn parse_style_schema(style: &Value) -> Result<StyleSchema, &'static str> {
    let columns = array(style, "/columns")?
        .iter()
        .map(|value| {
            Ok(StyleColumn {
                name: string(value, "/name")?.to_owned(),
                store_type: string(value, "/storeType")?.to_owned(),
                nullable: boolean(value, "/nullable")?,
            })
        })
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

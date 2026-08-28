use std::fmt;
use std::str::FromStr;

use super::model::{ScenarioConfig, MAX_SCENARIOS};

const MAGIC: &str = "sf-performance-scenarios-v1";
pub const MAX_SCENARIO_CONFIG_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

pub fn render_scenarios(scenarios: &[ScenarioConfig]) -> Result<String, ConfigError> {
    validate_order(scenarios)?;
    let mut output = format!("{MAGIC}\n");
    for scenario in scenarios {
        output.push_str(&format!(
            "scenario\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            scenario.id,
            scenario.scale,
            scenario.metric,
            scenario.boundary,
            scenario.unit,
            scenario.warmup_count,
            scenario.sample_count
        ));
    }
    if output.len() > MAX_SCENARIO_CONFIG_BYTES {
        return Err(ConfigError("scenario manifest exceeds byte bound".into()));
    }
    Ok(output)
}

pub fn parse_scenarios(bytes: &[u8]) -> Result<Vec<ScenarioConfig>, ConfigError> {
    if bytes.len() > MAX_SCENARIO_CONFIG_BYTES {
        return Err(ConfigError("scenario manifest exceeds byte bound".into()));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ConfigError("scenario manifest is not UTF-8".into()))?;
    if !text.ends_with('\n') || text.contains('\r') {
        return Err(ConfigError(
            "scenario manifest must use canonical LF termination".into(),
        ));
    }
    let mut lines = text.lines();
    if lines.next() != Some(MAGIC) {
        return Err(ConfigError("invalid scenario manifest header".into()));
    }
    let mut scenarios = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 8 || fields[0] != "scenario" {
            return Err(ConfigError("invalid scenario manifest record".into()));
        }
        scenarios.push(
            ScenarioConfig::new(
                fields[1],
                parse_number(fields[2], "scale")?,
                parse_enum(fields[3])?,
                parse_enum(fields[4])?,
                parse_enum(fields[5])?,
                parse_number(fields[6], "warmup count")?,
                parse_number(fields[7], "sample count")?,
            )
            .map_err(|error| ConfigError(error.to_string()))?,
        );
    }
    validate_order(&scenarios)?;
    if render_scenarios(&scenarios)? != text {
        return Err(ConfigError("scenario manifest is not canonical".into()));
    }
    Ok(scenarios)
}

fn validate_order(scenarios: &[ScenarioConfig]) -> Result<(), ConfigError> {
    if scenarios.is_empty() || scenarios.len() > MAX_SCENARIOS {
        return Err(ConfigError(format!(
            "scenario count must be between 1 and {MAX_SCENARIOS}"
        )));
    }
    for pair in scenarios.windows(2) {
        if pair[0].id >= pair[1].id {
            return Err(ConfigError(
                "scenario ids must be unique and byte-sorted".into(),
            ));
        }
    }
    Ok(())
}

fn parse_enum<T>(value: &str) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    value
        .parse()
        .map_err(|error: T::Err| ConfigError(error.to_string()))
}

fn parse_number<T>(value: &str, label: &str) -> Result<T, ConfigError>
where
    T: FromStr,
{
    value
        .parse()
        .map_err(|_| ConfigError(format!("invalid {label}")))
}

//! Configuration override parsing and application logic.

use toml::value::Table;

use crate::errors::ConfigError;

type Override = (String, toml::Value);

/// Parses an override string. Splits by '=' to get key and raw str value, then parses the str
/// value.
pub(crate) fn parse_override(override_str: &str) -> Result<Override, ConfigError> {
    let (key, value_str) = override_str
        .split_once("=")
        .ok_or(ConfigError::InvalidOverride {
            override_str: override_str.to_string(),
        })?;
    Ok((key.to_string(), parse_value(value_str)))
}

/// Apply override to config table.
pub(crate) fn apply_override(
    path: &str,
    value: toml::Value,
    table: &mut Table,
) -> Result<(), ConfigError> {
    apply_override_inner(path, path, value, table)
}

fn apply_override_inner(
    original_path: &str,
    remaining_path: &str,
    value: toml::Value,
    table: &mut Table,
) -> Result<(), ConfigError> {
    match remaining_path.split_once(".") {
        None => {
            table.insert(remaining_path.to_string(), value);
            Ok(())
        }
        Some((key, rest)) => match table.get_mut(key) {
            Some(toml::Value::Table(t)) => apply_override_inner(original_path, rest, value, t),
            Some(_) => Err(ConfigError::TraverseNonTableAt {
                key: key.to_string(),
                path: original_path.to_string(),
            }),
            None => Err(ConfigError::MissingKey {
                key: key.to_string(),
                path: original_path.to_string(),
            }),
        },
    }
}

/// Parses a string into a toml value. First tries as `i64`, then as `bool` and then defaults to
/// `String`.
fn parse_value(str_value: &str) -> toml::Value {
    str_value
        .parse::<i64>()
        .map(toml::Value::Integer)
        .or_else(|_| str_value.parse::<bool>().map(toml::Value::Boolean))
        .unwrap_or_else(|_| toml::Value::String(str_value.to_string()))
}

//! Configuration override parsing and application logic.

use toml::value::Table;

use crate::errors::ConfigError;

type Override = (String, toml::Value);

/// Parses an override string. Splits by '=' to get key and value, then splits
/// the key by '.' which is the update path.
pub(crate) fn parse_override(override_str: &str) -> Result<Override, ConfigError> {
    let (key, value_str) = override_str
        .split_once("=")
        .ok_or(ConfigError::InvalidOverride(override_str.to_string()))?;
    Ok((key.to_string(), parse_value(value_str)))
}

/// Apply override to config table.
pub(crate) fn apply_override(
    path: &str,
    value: toml::Value,
    table: &mut Table,
) -> Result<(), ConfigError> {
    match path.split_once(".") {
        None => {
            table.insert(path.to_string(), value);
            Ok(())
        }
        Some((key, rest)) => {
            if let Some(t) = table.get_mut(key).and_then(|v| v.as_table_mut()) {
                apply_override(rest, value, t)
            } else if table.contains_key(key) {
                Err(ConfigError::TraverseNonTableAt(key.to_string()))
            } else {
                Err(ConfigError::MissingKey(key.to_string()))
            }
        }
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

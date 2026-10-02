//! Stable human and machine renderers for console read results.

use std::io::Write;

use crate::{ConsoleError, ConsoleResult, ConsoleRow, SourceSchema};

/// Format used for a schema or single record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordFormat {
    /// Stable line-oriented fields for an operator.
    #[default]
    Porcelain,
    /// Pretty JSON for one result.
    Json,
}

/// Renders one source schema.
pub fn render_schema(schema: &SourceSchema, format: RecordFormat) -> ConsoleResult<String> {
    match format {
        RecordFormat::Json => serde_json::to_string_pretty(schema)
            .map_err(|error| ConsoleError::Render(error.to_string())),
        RecordFormat::Porcelain => {
            let mut lines = vec![
                format!("source.name: {}", schema.name),
                format!("source.kind: {}", schema.kind.as_str()),
            ];
            if let Some(key_type) = schema.key_type {
                lines.push(format!("source.key_type: {}", key_type.as_str()));
            }
            for argument in schema.arguments {
                lines.push(format!(
                    "source.argument.{}: {}",
                    argument.name,
                    argument.scalar_type.as_str()
                ));
            }
            for field in schema.value.fields {
                let nullable = if field.nullable { "?" } else { "" };
                let settable = if field.settable { " settable" } else { "" };
                lines.push(format!(
                    "source.field.{}: {}{nullable}{settable}",
                    field.name,
                    field.scalar_type.as_str()
                ));
            }
            for modifier in schema.modifiers {
                let arguments = modifier
                    .arguments
                    .iter()
                    .map(|argument| format!("{}: {}", argument.name, argument.scalar_type.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("source.modifier.{}: ({arguments})", modifier.name));
            }
            Ok(lines.join("\n"))
        }
    }
}

/// Renders one stable scalar row.
pub fn render_record(row: &ConsoleRow, format: RecordFormat) -> ConsoleResult<String> {
    match format {
        RecordFormat::Json => serde_json::to_string_pretty(row)
            .map_err(|error| ConsoleError::Render(error.to_string())),
        RecordFormat::Porcelain => {
            let mut lines = vec![
                format!("record.source: {}", row.source),
                format!("record.key: {}", row.key),
            ];
            lines.extend(
                row.fields
                    .iter()
                    .map(|(name, value)| format!("record.{name}: {value}")),
            );
            Ok(lines.join("\n"))
        }
    }
}

/// Writes a lazy row stream as one compact JSON object per line.
pub fn write_json_lines(
    rows: impl IntoIterator<Item = ConsoleResult<ConsoleRow>>,
    mut writer: impl Write,
) -> ConsoleResult<()> {
    for row in rows {
        serde_json::to_writer(&mut writer, &row?)
            .map_err(|error| ConsoleError::Render(error.to_string()))?;
        writer
            .write_all(b"\n")
            .map_err(|error| ConsoleError::Render(error.to_string()))?;
    }
    Ok(())
}

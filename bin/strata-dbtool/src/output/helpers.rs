//! Helper functions for output formatting
//!
//! This module provides output functions and helper utilities for consistent
//! formatting across all commands.

use std::io::{self, Write};

use serde::Serialize;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use super::traits::Formattable;
use crate::cli::OutputFormat;

/// Output function that handles all formats
///
/// This is the idiomatic way - separate the formatting concern from the data.
/// Types must implement Serialize for JSON and Formattable for porcelain.
pub(crate) fn output<T: Serialize + Formattable>(
    data: &T,
    format: OutputFormat,
) -> Result<(), DisplayedError> {
    output_to(data, format, &mut io::stdout())
}

/// Output function that writes to a specific writer (useful for testing)
pub(crate) fn output_to<T: Serialize + Formattable, W: Write>(
    data: &T,
    format: OutputFormat,
    writer: &mut W,
) -> Result<(), DisplayedError> {
    match format {
        OutputFormat::Porcelain => {
            let porcelain_str = data.format_porcelain();
            writeln!(writer, "{porcelain_str}")
                .internal_error("Failed to write porcelain output")?;
        }
        OutputFormat::Json => {
            let json_str =
                serde_json::to_string_pretty(data).internal_error("Failed to serialize to JSON")?;
            writeln!(writer, "{json_str}").internal_error("Failed to write JSON output")?;
        }
    }
    Ok(())
}

/// Helper function for creating porcelain field output
pub(crate) fn porcelain_field<T: std::fmt::Display>(key: &str, value: T) -> String {
    format!("{key}: {value}")
}

/// Helper function for porcelain boolean formatting
pub(crate) fn porcelain_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde::Serialize;

    use super::*;

    #[derive(Serialize)]
    struct TestData {
        name: String,
        value: i32,
        active: bool,
    }

    // Example struct showing conditional serialization
    #[derive(Serialize)]
    struct ConditionalTestData {
        name: String,
        value: i32,
        #[serde(skip_serializing_if = "Option::is_none")]
        optional_field: Option<String>,
        #[serde(skip_serializing_if = "String::is_empty")]
        empty_string_field: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        empty_array_field: Vec<String>,
        #[serde(skip_serializing_if = "is_false")]
        boolean_field: bool,
        #[serde(skip_serializing_if = "is_default")]
        default_value_field: i32,
    }

    /// Helper function for porcelain optional formatting
    fn porcelain_optional<T: std::fmt::Display>(value: &Option<T>) -> String {
        match value {
            Some(v) => format!("{v}"),
            None => String::new(),
        }
    }

    /// Helper function for porcelain array formatting
    fn porcelain_array<T: std::fmt::Display>(values: &[T]) -> String {
        values
            .iter()
            .map(|v| format!("{v}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn is_false(value: &bool) -> bool {
        !*value
    }

    fn is_default<T: Default + PartialEq>(value: &T) -> bool {
        *value == T::default()
    }

    impl Formattable for TestData {
        fn format_porcelain(&self) -> String {
            format!(
                "{}\n{}\n{}",
                porcelain_field("name", &self.name),
                porcelain_field("value", self.value),
                porcelain_field("active", porcelain_bool(self.active))
            )
        }
    }

    impl Formattable for ConditionalTestData {
        fn format_porcelain(&self) -> String {
            let mut output = Vec::new();
            output.push(porcelain_field("name", &self.name));
            output.push(porcelain_field("value", self.value));

            if let Some(ref opt) = self.optional_field {
                output.push(porcelain_field("optional_field", opt));
            }

            if !self.empty_string_field.is_empty() {
                output.push(porcelain_field(
                    "empty_string_field",
                    &self.empty_string_field,
                ));
            }

            if !self.empty_array_field.is_empty() {
                output.push(porcelain_field(
                    "empty_array_field",
                    porcelain_array(&self.empty_array_field),
                ));
            }

            if self.boolean_field {
                output.push(porcelain_field(
                    "boolean_field",
                    porcelain_bool(self.boolean_field),
                ));
            }

            if self.default_value_field != 0 {
                output.push(porcelain_field(
                    "default_value_field",
                    self.default_value_field,
                ));
            }

            output.join("\n")
        }
    }

    #[test]
    fn test_output_json() {
        let data = TestData {
            name: "test".to_string(),
            value: 42,
            active: true,
        };

        let mut buffer = Cursor::new(Vec::new());
        let result = output_to(&data, OutputFormat::Json, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();
        assert!(output.contains("\"name\": \"test\""));
        assert!(output.contains("\"value\": 42"));
        assert!(output.contains("\"active\": true"));
    }

    #[test]
    fn test_conditional_serialization() {
        // Test with fields that should be skipped
        let data = ConditionalTestData {
            name: "test".to_string(),
            value: 42,
            optional_field: None,
            empty_string_field: String::new(),
            empty_array_field: Vec::new(),
            boolean_field: false,
            default_value_field: 0,
        };

        let mut buffer = Cursor::new(Vec::new());
        let result = output_to(&data, OutputFormat::Json, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();
        // These fields should be present
        assert!(output.contains("\"name\": \"test\""));
        assert!(output.contains("\"value\": 42"));
        // These fields should be skipped
        assert!(!output.contains("optional_field"));
        assert!(!output.contains("empty_string_field"));
        assert!(!output.contains("empty_array_field"));
        assert!(!output.contains("boolean_field"));
        assert!(!output.contains("default_value_field"));

        // Test with fields that should be included
        let data = ConditionalTestData {
            name: "test".to_string(),
            value: 42,
            optional_field: Some("present".to_string()),
            empty_string_field: "not_empty".to_string(),
            empty_array_field: vec!["item".to_string()],
            boolean_field: true,
            default_value_field: 100,
        };

        let mut buffer = Cursor::new(Vec::new());
        let result = output_to(&data, OutputFormat::Json, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();
        // All fields should be present
        assert!(output.contains("\"optional_field\": \"present\""));
        assert!(output.contains("\"empty_string_field\": \"not_empty\""));
        assert!(output.contains("\"empty_array_field\""));
        assert!(output.contains("\"item\""));
        assert!(output.contains("\"boolean_field\": true"));
        assert!(output.contains("\"default_value_field\": 100"));
    }

    #[test]
    fn test_output_porcelain() {
        let data = TestData {
            name: "test".to_string(),
            value: 42,
            active: true,
        };

        let mut buffer = Cursor::new(Vec::new());
        let result = output_to(&data, OutputFormat::Porcelain, &mut buffer);
        assert!(result.is_ok());

        let output = String::from_utf8(buffer.into_inner()).unwrap();
        assert!(output.contains("name: test"));
        assert!(output.contains("value: 42"));
        assert!(output.contains("active: true"));
    }

    #[test]
    fn test_porcelain_helpers() {
        assert_eq!(porcelain_bool(true), "true");
        assert_eq!(porcelain_bool(false), "false");

        let some_value = Some(42);
        let none_value: Option<i32> = None;
        assert_eq!(porcelain_optional(&some_value), "42");
        assert_eq!(porcelain_optional(&none_value), "");

        let values = vec![1, 2, 3];
        assert_eq!(porcelain_array(&values), "1,2,3");

        let str_values = vec!["a", "b", "c"];
        assert_eq!(porcelain_array(&str_values), "a,b,c");

        let empty: Vec<i32> = vec![];
        assert_eq!(porcelain_array(&empty), "");
    }
}

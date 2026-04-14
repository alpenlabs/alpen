//! Writes a report in porcelain or JSON.

use std::io::{self, Write};

use serde::Serialize;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use super::Formattable;
use crate::cli::OutputFormat;

/// Renders `data` to stdout in the requested format.
pub(crate) fn output<T: Serialize + Formattable>(
    data: &T,
    format: OutputFormat,
) -> Result<(), DisplayedError> {
    output_to(data, format, &mut io::stdout())
}

/// Renders `data` to the given writer in the requested format.
pub(crate) fn output_to<T: Serialize + Formattable, W: Write>(
    data: &T,
    format: OutputFormat,
    writer: &mut W,
) -> Result<(), DisplayedError> {
    match format {
        OutputFormat::Porcelain => {
            writeln!(writer, "{}", data.format_porcelain())
                .internal_error("failed to write porcelain output")?;
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(data)
                .internal_error("failed to serialize JSON output")?;
            writeln!(writer, "{json}").internal_error("failed to write JSON output")?;
        }
    }
    Ok(())
}

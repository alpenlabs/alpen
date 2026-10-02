//! Noninteractive frontend for the typed database console.

use std::{
    error::Error,
    fmt,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    result::Result as StdResult,
    str::FromStr,
    sync::Arc,
};

use argh::FromArgs;
use strata_db_console::{
    ConsoleExecutor, ConsoleOutput, ConsoleResult, ConsoleRow, RecordFormat, parse_console_plan,
    render_record, render_schema, write_json_lines,
};
use strata_db_store_sled::{
    SLED_NAME, SledBackend, SledDbConfig, build_console_registry, open_sled_database,
};

const DEFAULT_SCAN_LIMIT: usize = 1_000;

type AppResult<T> = StdResult<T, Box<dyn Error>>;

/// Executes typed, bounded database-console reads while the node is offline.
#[derive(Debug, FromArgs)]
struct Cli {
    /// node data directory containing the OL Sled database
    #[argh(option, short = 'd', default = "PathBuf::from(\"data\")")]
    datadir: PathBuf,

    #[argh(subcommand)]
    command: Command,
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
enum Command {
    Eval(EvalArgs),
}

/// Executes one read-only console program.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "eval")]
struct EvalArgs {
    /// program such as 'scan tasks | filter status == "pending" | take 20'
    #[argh(positional)]
    program: String,

    /// output format: porcelain, json, or jsonl
    #[argh(option, default = "OutputFormat::Porcelain")]
    format: OutputFormat,

    /// maximum records pulled by any scan before filtering
    #[argh(option, default = "DEFAULT_SCAN_LIMIT")]
    scan_limit: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OutputFormat {
    #[default]
    Porcelain,
    Json,
    JsonLines,
}

#[derive(Clone, Copy, Debug)]
struct ParseOutputFormatError;

impl fmt::Display for ParseOutputFormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("must be 'porcelain', 'json', or 'jsonl'")
    }
}

impl Error for ParseOutputFormatError {}

impl FromStr for OutputFormat {
    type Err = ParseOutputFormatError;

    fn from_str(value: &str) -> StdResult<Self, Self::Err> {
        match value {
            "porcelain" => Ok(Self::Porcelain),
            "json" => Ok(Self::Json),
            "jsonl" => Ok(Self::JsonLines),
            _ => Err(ParseOutputFormatError),
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> AppResult<()> {
    let Cli { datadir, command } = argh::from_env();
    match command {
        Command::Eval(args) => evaluate(&datadir, args),
    }
}

fn evaluate(datadir: &Path, args: EvalArgs) -> AppResult<()> {
    let plan = parse_console_plan(&args.program, args.scan_limit)?;
    let backend = open_database(datadir)?;
    let registry = build_console_registry(&backend)?;
    let output = ConsoleExecutor::new(&registry).execute(plan)?;
    write_output(output, args.format, io::stdout().lock())?;
    Ok(())
}

fn open_database(datadir: &Path) -> AppResult<Arc<SledBackend>> {
    let sled = open_sled_database(datadir, SLED_NAME)
        .map_err(|error| io::Error::other(format!("failed to open Sled database: {error}")))?;
    let config = SledDbConfig::new_with_constant_backoff(5, 200);
    SledBackend::new(sled, config)
        .map(Arc::new)
        .map_err(|error| io::Error::other(format!("failed to open Sled backend: {error}")).into())
}

fn write_output(
    output: ConsoleOutput,
    format: OutputFormat,
    mut writer: impl Write,
) -> AppResult<()> {
    match output {
        ConsoleOutput::Schema(schema) => match format {
            OutputFormat::Porcelain => {
                writeln!(
                    writer,
                    "{}",
                    render_schema(&schema, RecordFormat::Porcelain)?
                )?;
            }
            OutputFormat::Json => {
                writeln!(writer, "{}", render_schema(&schema, RecordFormat::Json)?)?;
            }
            OutputFormat::JsonLines => {
                serde_json::to_writer(&mut writer, &schema)?;
                writeln!(writer)?;
            }
        },
        ConsoleOutput::Row(row) => match row {
            Some(row) => match format {
                OutputFormat::Porcelain => {
                    writeln!(writer, "{}", render_record(&row, RecordFormat::Porcelain)?)?;
                }
                OutputFormat::Json => {
                    writeln!(writer, "{}", render_record(&row, RecordFormat::Json)?)?;
                }
                OutputFormat::JsonLines => {
                    serde_json::to_writer(&mut writer, &row)?;
                    writeln!(writer)?;
                }
            },
            None => writeln!(writer, "null")?,
        },
        ConsoleOutput::Rows(rows) => match format {
            OutputFormat::Porcelain => write_porcelain_rows(rows, writer)?,
            OutputFormat::Json => {
                let rows = rows.collect::<ConsoleResult<Vec<_>>>()?;
                serde_json::to_writer_pretty(&mut writer, &rows)?;
                writeln!(writer)?;
            }
            OutputFormat::JsonLines => write_json_lines(rows, writer)?,
        },
        ConsoleOutput::Scalar(value) => match format {
            OutputFormat::Porcelain => writeln!(writer, "{value}")?,
            OutputFormat::Json | OutputFormat::JsonLines => {
                serde_json::to_writer(&mut writer, &value)?;
                writeln!(writer)?;
            }
        },
    }
    Ok(())
}

fn write_porcelain_rows(
    rows: impl IntoIterator<Item = ConsoleResult<ConsoleRow>>,
    mut writer: impl Write,
) -> AppResult<()> {
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            writeln!(writer)?;
        }
        writeln!(writer, "{}", render_record(&row?, RecordFormat::Porcelain)?)?;
    }
    Ok(())
}

//! Frontends for the typed database console.

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
    ConsoleError, ConsoleExecutor, ConsoleOutput, ConsoleProgram, ConsoleRegistry, ConsoleResult,
    ConsoleRow, RecordFormat, WritePreview, WriteSession, parse_console_program, render_record,
    render_schema, write_json_lines,
};
use strata_db_store_sled::{
    SLED_NAME, SledBackend, SledDbConfig, build_console_registry, open_sled_database,
};

const DEFAULT_SCAN_LIMIT: usize = 1_000;
const DEFAULT_PREVIEW_LIMIT: usize = 20;

type AppResult<T> = StdResult<T, Box<dyn Error>>;

/// Inspects and modifies the typed database console while the node is offline.
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
    Repl(ReplArgs),
}

/// Executes one console program, previewing staged writes by default.
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

    /// maximum write changes included in the displayed preview
    #[argh(option, default = "DEFAULT_PREVIEW_LIMIT")]
    preview_limit: usize,

    /// commit an implicitly staged write after producing its preview
    #[argh(switch)]
    commit: bool,
}

/// Runs an interactive console with persistent staged-write state.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "repl")]
struct ReplArgs {
    /// output format: porcelain, json, or jsonl
    #[argh(option, default = "OutputFormat::Porcelain")]
    format: OutputFormat,

    /// maximum records pulled by any scan before filtering
    #[argh(option, default = "DEFAULT_SCAN_LIMIT")]
    scan_limit: usize,

    /// maximum write changes included in the displayed preview
    #[argh(option, default = "DEFAULT_PREVIEW_LIMIT")]
    preview_limit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteStatus {
    Preview,
    Staged,
    Committed,
    Aborted,
}

impl WriteStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Staged => "staged",
            Self::Committed => "committed",
            Self::Aborted => "aborted",
        }
    }
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
        Command::Repl(args) => run_repl(&datadir, args),
    }
}

fn evaluate(datadir: &Path, args: EvalArgs) -> AppResult<()> {
    let program = parse_console_program(&args.program, args.scan_limit)?;
    if args.commit && matches!(&program, ConsoleProgram::Read(_)) {
        return Err(ConsoleError::invalid_input(
            "--commit",
            "requires a setter or modifier program",
        )
        .into());
    }
    let backend = open_database(datadir)?;
    let registry = build_console_registry(&backend)?;
    let stdout = io::stdout();
    let writer = stdout.lock();
    match program {
        ConsoleProgram::Read(plan) => {
            let output = ConsoleExecutor::new(&registry).execute(plan)?;
            write_output(output, args.format, writer)?;
        }
        ConsoleProgram::Write(plan) => {
            let mut session = WriteSession::new(&registry);
            if args.commit {
                plan.stage(&mut session)?;
                let preview = session.commit()?;
                write_write_preview(
                    &preview,
                    WriteStatus::Committed,
                    args.format,
                    args.preview_limit,
                    writer,
                )?;
            } else {
                let preview = plan.stage(&mut session)?;
                write_write_preview(
                    preview,
                    WriteStatus::Preview,
                    args.format,
                    args.preview_limit,
                    writer,
                )?;
            }
        }
    }
    Ok(())
}

fn run_repl(datadir: &Path, args: ReplArgs) -> AppResult<()> {
    let backend = open_database(datadir)?;
    let registry = build_console_registry(&backend)?;
    let mut session = WriteSession::new(&registry);
    let stdin = io::stdin();
    let stdout = io::stdout();

    writeln!(stdout.lock(), "strata-dbconsole; type 'help' for commands")?;
    loop {
        let Some(command) = read_repl_command(&stdin, &stdout)? else {
            if session.staged().is_some() {
                eprintln!("discarding staged write without committing");
            }
            return Ok(());
        };
        if command.is_empty() {
            continue;
        }

        let mut writer = stdout.lock();
        match execute_repl_command(
            &command,
            &registry,
            &mut session,
            args.format,
            args.scan_limit,
            args.preview_limit,
            &mut writer,
        ) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => eprintln!("{error}"),
        }
    }
}

fn read_repl_command(stdin: &io::Stdin, stdout: &io::Stdout) -> io::Result<Option<String>> {
    let mut command = String::new();
    let mut continuation = false;
    loop {
        {
            let mut writer = stdout.lock();
            writer.write_all(if continuation { b"..> " } else { b"db> " })?;
            writer.flush()?;
        }

        let mut line = String::new();
        let read = match stdin.read_line(&mut line) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                writeln!(stdout.lock())?;
                return Ok(Some(String::new()));
            }
            Err(error) => return Err(error),
        };
        if read == 0 {
            return Ok(None);
        }

        let line = line.trim();
        if !command.is_empty() {
            command.push(' ');
        }
        command.push_str(line);
        continuation = line.ends_with('|');
        if !continuation {
            return Ok(Some(command));
        }
    }
}

fn execute_repl_command(
    command: &str,
    registry: &ConsoleRegistry,
    session: &mut WriteSession<'_>,
    format: OutputFormat,
    scan_limit: usize,
    preview_limit: usize,
    mut writer: impl Write,
) -> AppResult<bool> {
    match command {
        "help" => {
            writeln!(
                writer,
                "reads: schema/get/scan/scan_rev\n\
                 writes: get ... | set/modify ...; scan ... | modify ...\n\
                 session: staged, commit, abort, exit"
            )?;
        }
        "staged" => match session.staged() {
            Some(preview) => {
                write_write_preview(preview, WriteStatus::Staged, format, preview_limit, writer)?
            }
            None => writeln!(writer, "no write staged")?,
        },
        "commit" => {
            let preview = session.commit()?;
            write_write_preview(
                &preview,
                WriteStatus::Committed,
                format,
                preview_limit,
                writer,
            )?;
        }
        "abort" => {
            let preview = session.abort().ok_or(ConsoleError::NoStagedWrite)?;
            write_write_preview(
                &preview,
                WriteStatus::Aborted,
                format,
                preview_limit,
                writer,
            )?;
        }
        "exit" | "quit" => {
            if session.staged().is_some() {
                return Err(ConsoleError::WriteAlreadyStaged.into());
            }
            return Ok(true);
        }
        _ => match parse_console_program(command, scan_limit)? {
            ConsoleProgram::Read(plan) => {
                let output = ConsoleExecutor::new(registry).execute(plan)?;
                write_output(output, format, writer)?;
            }
            ConsoleProgram::Write(plan) => {
                let preview = plan.stage(session)?;
                write_write_preview(preview, WriteStatus::Staged, format, preview_limit, writer)?;
            }
        },
    }
    Ok(false)
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

fn write_write_preview(
    preview: &WritePreview,
    status: WriteStatus,
    format: OutputFormat,
    preview_limit: usize,
    mut writer: impl Write,
) -> AppResult<()> {
    let total_changes = preview.changes.len();
    let shown_changes = total_changes.min(preview_limit);
    let omitted_changes = total_changes - shown_changes;
    let changes = &preview.changes[..shown_changes];
    match format {
        OutputFormat::Porcelain => {
            writeln!(writer, "write.status: {}", status.as_str())?;
            writeln!(writer, "write.operation: {}", preview.operation)?;
            writeln!(writer, "write.total_changes: {total_changes}")?;
            writeln!(writer, "write.shown_changes: {shown_changes}")?;
            writeln!(writer, "write.omitted_changes: {omitted_changes}")?;
            for (index, change) in changes.iter().enumerate() {
                writeln!(
                    writer,
                    "write.change.{index}.source: {}",
                    change.before.source
                )?;
                writeln!(writer, "write.change.{index}.key: {}", change.before.key)?;
                for (name, value) in &change.before.fields {
                    writeln!(writer, "write.change.{index}.before.{name}: {value}")?;
                }
                for (name, value) in &change.after.fields {
                    writeln!(writer, "write.change.{index}.after.{name}: {value}")?;
                }
            }
        }
        format @ (OutputFormat::Json | OutputFormat::JsonLines) => {
            let output = serde_json::json!({
                "status": status.as_str(),
                "operation": preview.operation,
                "total_changes": total_changes,
                "shown_changes": shown_changes,
                "omitted_changes": omitted_changes,
                "changes": changes,
            });
            if format == OutputFormat::Json {
                serde_json::to_writer_pretty(&mut writer, &output)?;
            } else {
                serde_json::to_writer(&mut writer, &output)?;
            }
            writeln!(writer)?;
        }
    }
    Ok(())
}

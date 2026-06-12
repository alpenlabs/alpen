//! Prover performance evaluation.

use std::{error::Error, process};

use sp1_sdk::utils::setup_logger;
#[cfg(feature = "sp1")]
use strata_sp1_guest_builder as _;
#[cfg(feature = "sp1")]
use zkaleido_sp1_host as _;

pub mod args;
pub mod format;
pub mod github;
pub mod programs;

use anyhow::Result;
use args::{parse_mode, parse_programs, validate_mode_programs, EvalArgs, PerfMode};
use format::{format_header, format_results_for_mode, ProofSummary};
use github::{format_github_message, post_to_github_pr};
#[cfg(feature = "sp1")]
use zkaleido::ExecutionSummary;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    setup_logger();
    let args: EvalArgs = argh::from_env();

    // Parse programs
    let programs = parse_programs(&args.programs).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        process::exit(1);
    });
    let mode = parse_mode(&args.mode).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        process::exit(1);
    });
    validate_mode_programs(mode, &programs).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        process::exit(1);
    });

    let mut results_text = vec![format_header(&args)];

    #[cfg(feature = "sp1")]
    {
        let mut execute_reports: Vec<(String, ExecutionSummary)> = Vec::new();
        let mut prove_reports: Vec<(String, ProofSummary)> = Vec::new();

        match mode {
            PerfMode::Execute => {
                execute_reports = programs::run_sp1_execute_programs(&programs).await;
            }
            PerfMode::Prove => {
                prove_reports = programs::run_sp1_prove_programs(&programs).await;
            }
        }

        results_text.push(format_results_for_mode(
            mode,
            &execute_reports,
            &prove_reports,
            "SP1".to_owned(),
        ));
    }

    // Print results
    println!("{}", results_text.join("\n"));

    if args.post_to_gh {
        // Post to GitHub PR
        let message = format_github_message(&results_text);
        post_to_github_pr(&args, &message).await?;
    }

    Ok(())
}

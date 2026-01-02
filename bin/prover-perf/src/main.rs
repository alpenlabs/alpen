//! Prover performance evaluation.

use std::error::Error;

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
use args::EvalArgs;
use clap::Parser;
use format::{format_header, format_results};
use github::{format_github_message, post_to_github_pr};
use zkaleido::PerformanceReport;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    setup_logger();
    let args = EvalArgs::parse();

    let mut results_text = vec![format_header(&args)];

    #[cfg(feature = "sp1")]
    {
        let sp1_reports = programs::run_sp1_programs(&args.programs);
        results_text.push(format_results(&sp1_reports, "SP1".to_owned()));
        if !sp1_reports.iter().all(|r| r.success) {
            use std::process;

            println!("Some SP1 programs failed. Please check the results below.");
            process::exit(1);
        }
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

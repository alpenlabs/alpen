//! Prover performance evaluation.

use std::error::Error;

use sp1_sdk::utils::setup_logger;
#[cfg(feature = "sp1")]
use strata_zkvm_hosts::sp1::checkpoint_host;
#[cfg(feature = "sp1")]
use zkaleido_sp1_host::SP1HostConfig;

pub mod args;
mod checkpoint;
pub mod format;
pub mod github;

use anyhow::Result;
use args::EvalArgs;
#[cfg(feature = "sp1")]
use format::format_checkpoint_result;
use format::format_header;
use github::{format_github_message, post_to_github_pr};
use tokio::task::spawn_blocking;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    setup_logger();
    let args: EvalArgs = argh::from_env();

    if !args.programs.trim().eq_ignore_ascii_case("checkpoint") {
        return Err(format!(
            "unsupported program: {}; expected checkpoint",
            args.programs
        )
        .into());
    }

    let mut results_text = vec![format_header(&args)];

    #[cfg(feature = "sp1")]
    {
        let host = checkpoint_host(SP1HostConfig::default()).await;
        let summary = spawn_blocking(move || {
            // SP1Host's Debug uses zero-padded bytes32(); program_id() uses bytes32_raw(),
            // which can panic for valid key hashes with additional leading zeros.
            println!("Checkpoint guest artifact: {host:?}");
            checkpoint::gen_perf_report(&**host)
        })
        .await?;
        results_text.push(format_checkpoint_result(&summary));
    }

    println!("{}", results_text.join("\n"));

    if args.post_to_gh {
        // Post to GitHub PR
        let message = format_github_message(&results_text);
        post_to_github_pr(&args, &message).await?;
    }

    Ok(())
}

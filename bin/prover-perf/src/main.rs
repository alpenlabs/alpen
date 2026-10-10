//! Prover performance evaluation.

use anyhow::Result;
use clap::Parser;
use sp1_sdk::utils::setup_logger;
#[cfg(feature = "sp1")]
use zkaleido::ZkVm;
use zkaleido_perf_report::{render_report, ZkVmResults};
#[cfg(feature = "sp1")]
use zkaleido_sp1_host as _;

pub mod args;
pub mod programs;

use args::{parse_programs, EvalArgs};

/// Identifies this repository's sticky perf comment on a PR, so a run patches its own report
/// instead of one posted by another tool.
const COMMENT_MARKER: &str = "strata-provers-perf";

#[tokio::main]
async fn main() -> Result<()> {
    setup_logger();
    let args = EvalArgs::parse();

    let programs = parse_programs(&args.programs).map_err(anyhow::Error::msg)?;

    // Resolve the reporting target first, so a misconfiguration fails before the guests run.
    let reporter = args
        .github
        .as_ref()
        .map(|github| github.reporter(COMMENT_MARKER))
        .transpose()?;

    // Resolve the baseline anchor before running the guests. If a PR merges into the base branch
    // during the run, a later lookup would compare against changes this run never measured.
    let mut baseline_lookup_failed = false;
    let baseline_anchor = match &reporter {
        Some(reporter) => match reporter.resolve_baseline_anchor().await {
            Ok(anchor) => anchor,
            Err(err) => {
                eprintln!("warning: failed to resolve baseline anchor: {err:#}");
                baseline_lookup_failed = true;
                None
            }
        },
        None => None,
    };

    let mut results: Vec<ZkVmResults> = Vec::new();

    #[cfg(feature = "sp1")]
    results.push(ZkVmResults::new(
        ZkVm::SP1,
        programs::run_sp1_programs(&programs).await,
    ));

    // Without a baseline the report only loses its deltas, so a fetch failure must not block
    // posting it.
    let baseline = match (&reporter, &baseline_anchor) {
        (Some(reporter), Some(anchor)) => match reporter.fetch_baseline(anchor).await {
            Ok(baseline) => baseline,
            Err(err) => {
                eprintln!("warning: failed to fetch baseline report: {err:#}");
                baseline_lookup_failed = true;
                None
            }
        },
        _ => None,
    };

    println!(
        "{}",
        render_report(
            &results,
            baseline.as_ref().map(|baseline| &baseline.payload)
        )
    );

    if let Some(reporter) = reporter {
        reporter
            .post_report(
                &results,
                baseline.as_ref(),
                baseline_lookup_failed,
                baseline_anchor.as_ref(),
            )
            .await?;
    }

    Ok(())
}

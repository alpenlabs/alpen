use std::str::FromStr;

mod alpen_acct;
mod alpen_chunk;
mod checkpoint;

#[cfg(feature = "sp1")]
use zkaleido::ExecutionSummary;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GuestProgram {
    AlpenAcct,
    AlpenChunk,
    Checkpoint,
}

impl FromStr for GuestProgram {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "alpen-acct" => Ok(GuestProgram::AlpenAcct),
            "alpen-chunk" => Ok(GuestProgram::AlpenChunk),
            "checkpoint" => Ok(GuestProgram::Checkpoint),
            _ => Err(format!("unknown program: {s}")),
        }
    }
}

/// Runs SP1 programs and pairs each program's name with its
/// [`ExecutionSummary`] (cycles, gas, public values).
#[cfg(feature = "sp1")]
pub async fn run_sp1_programs(programs: &[GuestProgram]) -> Vec<(String, ExecutionSummary)> {
    use strata_zkvm_hosts::sp1::{alpen_acct_host, alpen_chunk_host, checkpoint_host};
    use zkaleido_sp1_host::SP1HostConfig;
    let mut reports = Vec::with_capacity(programs.len());
    for program in programs {
        let cfg = SP1HostConfig::default();
        let report = match program {
            GuestProgram::AlpenAcct => {
                alpen_acct::gen_perf_report(&**alpen_acct_host(cfg).await)
            }
            GuestProgram::AlpenChunk => {
                alpen_chunk::gen_perf_report(&**alpen_chunk_host(cfg).await)
            }
            GuestProgram::Checkpoint => {
                checkpoint::gen_perf_report(&**checkpoint_host(cfg).await)
            }
        };
        reports.push(report);
    }
    reports
}

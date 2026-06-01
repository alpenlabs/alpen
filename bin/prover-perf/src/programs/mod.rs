use std::{str::FromStr, time::Duration};

mod alpen_acct;
mod alpen_chunk;
mod checkpoint;

#[cfg(feature = "sp1")]
use zkaleido::ExecutionSummary;
use zkaleido::ProofReceiptWithMetadata;

#[derive(Debug)]
pub struct ProofReport {
    pub receipt: ProofReceiptWithMetadata,
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GuestProgram {
    AlpenAcct,
    AlpenChunk,
    Checkpoint,
    CheckpointCapacity,
}

impl FromStr for GuestProgram {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "alpen-acct" => Ok(GuestProgram::AlpenAcct),
            "alpen-chunk" => Ok(GuestProgram::AlpenChunk),
            "checkpoint" => Ok(GuestProgram::Checkpoint),
            "checkpoint-capacity" => Ok(GuestProgram::CheckpointCapacity),
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
            GuestProgram::AlpenAcct => alpen_acct::gen_perf_report(&**alpen_acct_host(cfg).await),
            GuestProgram::AlpenChunk => {
                alpen_chunk::gen_perf_report(&**alpen_chunk_host(cfg).await)
            }
            GuestProgram::Checkpoint => checkpoint::gen_perf_report(&**checkpoint_host(cfg).await),
            GuestProgram::CheckpointCapacity => {
                reports.extend(checkpoint::gen_capacity_perf_reports(
                    &**checkpoint_host(cfg).await,
                ));
                continue;
            }
        };
        reports.push(report);
    }
    reports
}

/// Runs SP1 proving for supported programs and pairs each program's name with its proof receipt.
#[cfg(feature = "sp1")]
pub async fn prove_sp1_programs(programs: &[GuestProgram]) -> Vec<(String, ProofReport)> {
    use strata_zkvm_hosts::sp1::checkpoint_host;
    use zkaleido_sp1_host::SP1HostConfig;

    let mut reports = Vec::with_capacity(programs.len());
    for program in programs {
        let cfg = SP1HostConfig::default();
        match program {
            GuestProgram::Checkpoint => {
                reports.push(checkpoint::prove_perf_report(&**checkpoint_host(cfg).await));
            }
            GuestProgram::CheckpointCapacity => {
                reports.extend(checkpoint::prove_capacity_reports(
                    &**checkpoint_host(cfg).await,
                ));
            }
            GuestProgram::AlpenAcct | GuestProgram::AlpenChunk => {
                panic!("SP1 proving mode is only wired for checkpoint programs");
            }
        }
    }
    reports
}

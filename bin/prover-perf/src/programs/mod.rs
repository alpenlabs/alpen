use std::{str::FromStr, time::Instant};

mod alpen_acct;
mod alpen_chunk;
mod checkpoint;

#[cfg(feature = "sp1")]
use zkaleido::{ExecutionSummary, ProofReceiptWithMetadata};

#[cfg(feature = "sp1")]
use crate::format::ProofSummary;

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
pub async fn run_sp1_execute_programs(
    programs: &[GuestProgram],
) -> Vec<(String, ExecutionSummary)> {
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
        };
        reports.push(report);
    }
    reports
}

#[cfg(feature = "sp1")]
pub async fn run_sp1_prove_programs(programs: &[GuestProgram]) -> Vec<(String, ProofSummary)> {
    use strata_zkvm_hosts::sp1::checkpoint_host;
    use zkaleido_sp1_host::SP1HostConfig;

    let mut reports = Vec::with_capacity(programs.len());
    for program in programs {
        let total_started_at = Instant::now();
        let (name, receipt, prepare_duration, prove_duration): (
            String,
            ProofReceiptWithMetadata,
            _,
            _,
        ) = match program {
            GuestProgram::AlpenAcct | GuestProgram::AlpenChunk => {
                unreachable!("prove mode rejects unsupported programs before execution")
            }
            GuestProgram::Checkpoint => {
                let prepare_started_at = Instant::now();
                let input = checkpoint::prepare_input();

                let cfg = SP1HostConfig::default();
                let host = checkpoint_host(cfg).await;
                let prepare_duration = prepare_started_at.elapsed();
                let prove_started_at = Instant::now();
                let (name, receipt) = checkpoint::prove_with_input(&input, &**host);
                let prove_duration = prove_started_at.elapsed();

                (name, receipt, prepare_duration, prove_duration)
            }
        };
        reports.push((
            name,
            ProofSummary {
                prepare_duration,
                prove_duration,
                total_duration: total_started_at.elapsed(),
                proof_bytes: receipt.receipt().proof().as_bytes().len(),
                proof_type: format!("{:?}", receipt.metadata().proof_type()),
            },
        ));
    }
    reports
}

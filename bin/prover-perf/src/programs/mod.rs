use std::str::FromStr;

mod checkpoint;
mod checkpoint_new;
mod evm_ee;

use crate::PerformanceReport;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GuestProgram {
    EvmEeStf,
    CheckpointV0,
    CheckpointV1,
}

impl FromStr for GuestProgram {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "evm-ee-stf" => Ok(GuestProgram::EvmEeStf),
            "checkpoint-v0" => Ok(GuestProgram::CheckpointV0),
            "checkpoint-v1" => Ok(GuestProgram::CheckpointV1),
            _ => Err(format!("unknown program: {s}")),
        }
    }
}

/// Runs SP1 programs to generate reports.
///
/// Generates [`PerformanceReport`] for each invocation.
#[cfg(feature = "sp1")]
pub fn run_sp1_programs(programs: &[GuestProgram]) -> Vec<PerformanceReport> {
    use strata_zkvm_hosts::sp1::{CHECKPOINT_HOST, CHECKPOINT_NEW_HOST, EVM_EE_STF_HOST};
    programs
        .iter()
        .map(|program| match program {
            GuestProgram::EvmEeStf => evm_ee::gen_perf_report(&**EVM_EE_STF_HOST),
            GuestProgram::CheckpointV0 => checkpoint::gen_perf_report(&**CHECKPOINT_HOST),
            GuestProgram::CheckpointV1 => checkpoint_new::gen_perf_report(&**CHECKPOINT_NEW_HOST),
        })
        .collect()
}

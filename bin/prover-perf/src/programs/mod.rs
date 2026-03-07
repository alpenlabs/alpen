use std::str::FromStr;

mod checkpoint;
mod checkpoint_new;
mod evm_ee;

use crate::PerformanceReport;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GuestProgram {
    EvmEeStf,
    Checkpoint,
    CheckpointNew,
}

impl FromStr for GuestProgram {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "evm-ee-stf" => Ok(GuestProgram::EvmEeStf),
            "checkpoint" => Ok(GuestProgram::Checkpoint),
            "checkpoint-new" => Ok(GuestProgram::CheckpointNew),
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
            GuestProgram::Checkpoint => checkpoint::gen_perf_report(&**CHECKPOINT_HOST),
            GuestProgram::CheckpointNew => checkpoint_new::gen_perf_report(&**CHECKPOINT_NEW_HOST),
        })
        .collect()
}

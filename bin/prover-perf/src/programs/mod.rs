use std::str::FromStr;

use clap::ValueEnum;

mod checkpoint;
mod cl_stf;
mod evm_ee;

use crate::PerformanceReport;

#[derive(Debug, Clone, ValueEnum)]
#[non_exhaustive]
pub enum GuestProgram {
    EvmEeStf,
    EvmEe,
    ClStf,
    Checkpoint,
}

impl FromStr for GuestProgram {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "evm-ee-stf" => Ok(GuestProgram::EvmEeStf),
            "evm-ee" => Ok(GuestProgram::EvmEe),
            "cl-stf" => Ok(GuestProgram::ClStf),
            "checkpoint" => Ok(GuestProgram::Checkpoint),
            // Add more matches
            _ => Err(format!("unknown program: {s}")),
        }
    }
}

/// Runs SP1 programs to generate reports.
///
/// Generates [`PerformanceReport`] for each invocation.
#[cfg(feature = "sp1")]
pub fn run_sp1_programs(programs: &[GuestProgram]) -> Vec<PerformanceReport> {
    use strata_zkvm_hosts::sp1::{CHECKPOINT_HOST, CL_STF_HOST, EVM_EE_HOST, EVM_EE_STF_HOST};
    programs
        .iter()
        .map(|program| match program {
            GuestProgram::EvmEeStf => evm_ee::gen_perf_report(&**EVM_EE_STF_HOST),
            GuestProgram::EvmEe => {
                let mut report = evm_ee::gen_perf_report(&**EVM_EE_HOST);
                report.name = format!("{} (new)", report.name);
                report
            }
            GuestProgram::ClStf => {
                cl_stf::gen_perf_report(&**CL_STF_HOST, evm_ee::proof_with_vk(&**EVM_EE_STF_HOST))
            }
            GuestProgram::Checkpoint => checkpoint::gen_perf_report(
                &**CHECKPOINT_HOST,
                cl_stf::proof_with_vk(&**CL_STF_HOST, &**EVM_EE_STF_HOST),
            ),
        })
        .collect()
}

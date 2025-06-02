use std::str::FromStr;

use clap::ValueEnum;

mod btc_blockscan;
mod checkpoint;
mod cl_stf;
mod evm_ee;

use crate::PerformanceReport;

#[derive(Debug, Clone, ValueEnum)]
#[non_exhaustive]
pub enum GuestProgram {
    BtcBlockscan,
    EvmEeStf,
    ClStf,
    Checkpoint,
}

impl FromStr for GuestProgram {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "btc-blockscan" => Ok(GuestProgram::BtcBlockscan),
            "evm-ee-stf" => Ok(GuestProgram::EvmEeStf),
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
    use strata_zkvm_hosts::sp1::{
        BTC_BLOCKSPACE_HOST, CHECKPOINT_HOST, CL_STF_HOST, EVM_EE_STF_HOST,
    };
    programs
        .iter()
        .map(|program| match program {
            GuestProgram::BtcBlockscan => btc_blockscan::gen_perf_report(&*BTC_BLOCKSPACE_HOST),
            GuestProgram::EvmEeStf => evm_ee::gen_perf_report(&*EVM_EE_STF_HOST),
            GuestProgram::ClStf => cl_stf::gen_perf_report(
                &*CL_STF_HOST,
                evm_ee::proof_with_vk(&*EVM_EE_STF_HOST),
                Some(btc_blockscan::proof_with_vk(&*BTC_BLOCKSPACE_HOST)),
            ),
            GuestProgram::Checkpoint => checkpoint::gen_perf_report(
                &*CHECKPOINT_HOST,
                cl_stf::proof_with_vk(&*CL_STF_HOST, &*EVM_EE_STF_HOST, &*BTC_BLOCKSPACE_HOST),
            ),
        })
        .collect()
}

/// Runs Risc0 programs to generate reports.
///
/// Generates [`PerformanceReport`] for each invocation.
#[cfg(feature = "risc0")]
pub fn run_risc0_programs(programs: &[GuestProgram]) -> Vec<PerformanceReport> {
    use strata_zkvm_hosts::risc0::{
        BTC_BLOCKSPACE_HOST, CHECKPOINT_HOST, CL_STF_HOST, EVM_EE_STF_HOST,
    };
    programs
        .iter()
        .map(|program| match program {
            GuestProgram::BtcBlockscan => btc_blockscan::gen_perf_report(&*BTC_BLOCKSPACE_HOST),
            GuestProgram::EvmEeStf => evm_ee::gen_perf_report(&*EVM_EE_STF_HOST),
            GuestProgram::ClStf => cl_stf::gen_perf_report(
                &*CL_STF_HOST,
                evm_ee::proof_with_vk(&*EVM_EE_STF_HOST),
                Some(btc_blockscan::proof_with_vk(&*BTC_BLOCKSPACE_HOST)),
            ),
            GuestProgram::Checkpoint => checkpoint::gen_perf_report(
                &*CHECKPOINT_HOST,
                cl_stf::proof_with_vk(&*CL_STF_HOST, &*EVM_EE_STF_HOST, &*BTC_BLOCKSPACE_HOST),
            ),
        })
        .collect()
}

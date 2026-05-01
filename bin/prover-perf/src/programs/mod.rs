use std::str::FromStr;

mod checkpoint;

use crate::PerformanceReport;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GuestProgram {
    Checkpoint,
}

impl FromStr for GuestProgram {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "checkpoint" => Ok(GuestProgram::Checkpoint),
            _ => Err(format!("unknown program: {s}")),
        }
    }
}

/// Runs SP1 programs to generate reports.
///
/// Generates [`PerformanceReport`] for each invocation.
#[cfg(feature = "sp1")]
pub fn run_sp1_programs(programs: &[GuestProgram]) -> Vec<PerformanceReport> {
    use strata_zkvm_hosts::sp1::CHECKPOINT_HOST;
    programs
        .iter()
        .map(|program| match program {
            GuestProgram::Checkpoint => checkpoint::gen_perf_report(&**CHECKPOINT_HOST),
        })
        .collect()
}

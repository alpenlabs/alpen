use strata_proofimpl_checkpoint_new::{
    program::CheckpointProgram, test_utils::prepare_checkpoint_input,
};
use tracing::info;
use zkaleido::{PerformanceReport, ZkVmHostPerf, ZkVmProgramPerf};

pub(crate) fn gen_perf_report(host: &impl ZkVmHostPerf) -> PerformanceReport {
    info!("Generating performance report for Checkpoint New");
    let input = prepare_checkpoint_input();
    CheckpointProgram::perf_report(&input, host).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_new_native_execution() {
        let input = prepare_checkpoint_input();
        let output = CheckpointProgram::execute(&input).unwrap();
        dbg!(output);
    }
}

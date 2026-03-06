use strata_checkpoint_types::BatchInfo;
use strata_identifiers::L2BlockCommitment;
use strata_proofimpl_checkpoint::program::CheckpointProgram;
use zkaleido::{PerformanceReport, ZkVmHostPerf, ZkVmProgramPerf};

fn prepare_input() -> BatchInfo {
    let l2 = L2BlockCommitment::null();
    BatchInfo::new(0, Default::default(), (l2, l2))
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHostPerf) -> PerformanceReport {
    let input = prepare_input();
    CheckpointProgram::perf_report(&input, host).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_native_execution() {
        let input = prepare_input();
        let output = CheckpointProgram::execute(&input).unwrap();
        dbg!(output);
    }
}

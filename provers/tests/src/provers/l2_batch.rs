use strata_proofimpl_cl_agg::{ClAggInput, ClAggProver};
use strata_zkvm::{ZkVmHost, ZkVmResult};

use super::{cl::ClProofGenerator, ProofGenerator};

#[derive(Clone)]
pub struct L2BatchProofGenerator<H: ZkVmHost> {
    cl_proof_generator: ClProofGenerator<H>,
    host: H,
}

impl<H: ZkVmHost> L2BatchProofGenerator<H> {
    pub fn new(cl_proof_generator: ClProofGenerator<H>, host: H) -> Self {
        Self {
            cl_proof_generator,
            host,
        }
    }
}

impl<H: ZkVmHost> ProofGenerator<ClAggProver> for L2BatchProofGenerator<H> {
    type Input = (u64, u64);

    fn get_input(&self, heights: &(u64, u64)) -> ZkVmResult<ClAggInput> {
        let (start_height, end_height) = *heights;
        let mut batch = Vec::new();

        for block_num in start_height..=end_height {
            let cl_proof = self.cl_proof_generator.get_proof(&block_num)?;
            batch.push(cl_proof);
        }

        let cl_stf_vk = self.cl_proof_generator.get_host().get_verification_key();
        Ok(ClAggInput { batch, cl_stf_vk })
    }

    fn get_proof_id(&self, heights: &(u64, u64)) -> String {
        let (start_height, end_height) = *heights;
        format!("l2_batch_{}_{}", start_height, end_height)
    }

    fn get_host(&self) -> impl ZkVmHost {
        self.host.clone()
    }
}

#[cfg(test)]
mod test {
    use strata_zkvm::ZkVmHost;

    use super::*;

    fn test_proof<H: ZkVmHost>(cl_agg_prover: L2BatchProofGenerator<H>) {
        let _ = cl_agg_prover.get_proof(&(1, 3)).unwrap();
    }

    #[test]
    #[cfg(not(any(feature = "risc0", feature = "sp1")))]
    fn test_native() {
        use crate::provers::TEST_NATIVE_GENERATORS;
        test_proof(TEST_NATIVE_GENERATORS.l2_batch());
    }

    #[test]
    #[cfg(feature = "risc0")]
    fn test_risc0() {
        use crate::provers::TEST_RISC0_GENERATORS;
        test_proof(TEST_RISC0_GENERATORS.l2_batch());
    }

    #[test]
    #[cfg(feature = "sp1")]
    fn test_sp1() {
        use crate::provers::TEST_SP1_GENERATORS;
        test_proof(TEST_SP1_GENERATORS.l2_batch());
    }
}

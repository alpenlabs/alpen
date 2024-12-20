use bitcoin::Block;
use strata_proofimpl_btc_blockspace::{logic::BlockspaceProofInput, prover::BtcBlockspaceProver};
use strata_test_utils::l2::gen_params;
use strata_zkvm::{ZkVmHost, ZkVmResult};

use super::ProofGenerator;

#[derive(Clone)]
pub struct BtcBlockProofGenerator<H: ZkVmHost> {
    host: H,
}

impl<H: ZkVmHost> BtcBlockProofGenerator<H> {
    pub fn new(host: H) -> Self {
        Self { host }
    }
}

impl<H: ZkVmHost> ProofGenerator<BtcBlockspaceProver> for BtcBlockProofGenerator<H> {
    type Input = Block;
    fn get_input(&self, block: &Block) -> ZkVmResult<BlockspaceProofInput> {
        let params = gen_params();
        let rollup_params = params.rollup();
        let input = BlockspaceProofInput {
            block: block.clone(),
            rollup_params: rollup_params.clone(),
        };
        Ok(input)
    }

    fn get_proof_id(&self, block: &Block) -> String {
        format!("btc_block_{}", block.block_hash())
    }

    fn get_host(&self) -> impl ZkVmHost {
        self.host.clone()
    }
}

#[cfg(test)]
mod test {
    use strata_test_utils::bitcoin::get_btc_chain;

    use super::*;

    fn test_proof<H: ZkVmHost>(generator: BtcBlockProofGenerator<H>) {
        let btc_chain = get_btc_chain();
        let block = btc_chain.get_block(40321);

        let _ = generator.get_proof(block).unwrap();
    }

    #[test]
    #[cfg(not(any(feature = "risc0", feature = "sp1")))]
    fn test_native() {
        use crate::provers::TEST_NATIVE_GENERATORS;
        test_proof(TEST_NATIVE_GENERATORS.btc_blockspace());
    }

    #[test]
    #[cfg(feature = "risc0")]
    fn test_risc0() {
        use crate::provers::TEST_RISC0_GENERATORS;
        test_proof(TEST_RISC0_GENERATORS.btc_blockspace());
    }

    #[test]
    #[cfg(feature = "sp1")]
    fn test_sp1() {
        use crate::provers::TEST_SP1_GENERATORS;
        test_proof(TEST_SP1_GENERATORS.btc_blockspace());
    }
}

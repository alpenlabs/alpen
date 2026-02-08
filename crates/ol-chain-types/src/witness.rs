use strata_ol_chainstate_types::Chainstate;

use crate::L2Block;

#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ClBlockWitness {
    pub chainstate: Chainstate,
    pub block: L2Block,
}

impl ClBlockWitness {
    pub fn new(chainstate: Chainstate, block: L2Block) -> Self {
        Self { chainstate, block }
    }
}

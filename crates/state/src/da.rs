use crate::{
    chain_state::{Chainstate, ChainstateUpdateImpl},
    traits::{ChainstateDA, ChainstateUpdate},
};

pub struct ChainstateDAScheme {}

impl ChainstateDA for ChainstateDAScheme {
    fn chainstate_update_from_bytes(bytes: &[u8]) -> std::io::Result<impl ChainstateUpdate> {
        let new_chainstate = borsh::from_slice::<Chainstate>(bytes)?;
        Ok(ChainstateUpdateImpl::new(new_chainstate))
    }
}

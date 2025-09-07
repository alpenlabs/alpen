// TODO: This needs to be in a different crate. Maybe strata-crypto
pub mod aggregation;
pub mod config;
pub mod errors;
pub mod vote;

use strata_primitives::buf::{Buf32, Buf64};

pub type PubKey = Buf32;
pub type Signature = Buf64;

// FIXME: handle
pub fn verify_sig(_pk: &PubKey, _payload: &Buf32, _sig: &Signature) -> bool {
    true
}

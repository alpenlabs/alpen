//! Collection of generic internal data types that are used widely.

// TODO import address types
// TODO import generic account types

// Re-export identifier types from strata-identifiers
pub use strata_identifiers::{
    buf, epoch, hash, impl_buf_wrapper, l1 as l1_identifiers, ol, ol as l2, BitcoinBlockHeight,
    Buf20, Buf32, Buf64, EpochCommitment, EvmEeBlockCommitment, ExecBlockCommitment,
    L1BlockCommitment, L1BlockId, L2BlockCommitment, L2BlockId, OLBlockCommitment, OLBlockId,
};

pub mod block_credential;
pub mod constants;
pub mod crypto;
pub mod errors;
pub mod indexed;
pub mod keys;
pub mod l1;
pub mod operator;
pub mod params;
pub mod prelude;
pub mod proof;
pub mod roles;
pub mod serde_helpers;
pub mod sorted_vec;
pub mod utils;

pub use bitcoin_bosd;

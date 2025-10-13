//! Collection of generic internal data types that are used widely.

// TODO import address types
// TODO import generic account types

// Re-export identifier types from strata-identifiers
pub use strata_identifiers::{
    create_evm_extra_payload, impl_buf_wrapper, BitcoinBlockHeight, Buf20, Buf32, Buf64, CredRule,
    EpochCommitment, EvmEeBlockCommitment, EVMExtraPayload, ExecBlockCommitment, L1BlockCommitment,
    L1BlockId, L1Height, L2BlockCommitment, L2BlockId, OLBlockCommitment, OLBlockId,
};

// Re-export identifier modules for convenience
pub use strata_identifiers::{buf, epoch, hash};
pub use strata_identifiers::exec as evm_exec;
pub use strata_identifiers::ol as l2;

// Re-export crypto types
pub mod crypto {
    pub use strata_crypto::schnorr::*;
    pub use strata_crypto::RollupVerifyingKey;
}
pub use strata_crypto::RollupVerifyingKey;

pub mod block_credential;
pub mod constants;
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

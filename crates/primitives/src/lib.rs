//! Collection of generic internal data types that are used widely.

// TODO import address types
// TODO import generic account types

// Re-export identifier types from strata-identifiers
pub use strata_identifiers::{
    create_evm_extra_payload, impl_buf_wrapper, BitcoinBlockHeight, Buf20, Buf32, Buf64, CredRule,
    EVMExtraPayload, EpochCommitment, EvmEeBlockCommitment, ExecBlockCommitment, L1BlockCommitment,
    L1BlockId, L1Height, L2BlockCommitment, L2BlockId, OLBlockCommitment, OLBlockId,
};

// Create module aliases that re-export from identifiers
pub mod buf {
    pub use strata_identifiers::{Buf20, Buf32, Buf64};
}
pub mod epoch {
    pub use strata_identifiers::EpochCommitment;
}
pub mod hash {
    pub use strata_identifiers::hash::*;
}
pub mod evm_exec {
    pub use strata_identifiers::{
        create_evm_extra_payload, EVMExtraPayload, EvmEeBlockCommitment, ExecBlockCommitment,
    };
}
pub mod l2 {
    pub use strata_identifiers::{L2BlockCommitment, L2BlockId, OLBlockCommitment, OLBlockId};
}
pub mod ol {
    pub use strata_identifiers::{L2BlockCommitment, L2BlockId, OLBlockCommitment, OLBlockId};
}

// Re-export crypto types
pub mod crypto {
    pub use strata_crypto::schnorr::*;
}

pub mod block_credential;
pub mod constants;
pub mod errors;
pub mod indexed;
pub mod l1;
pub mod prelude;
pub mod proof;
pub mod roles;
pub mod serde_helpers;
pub mod sorted_vec;
pub mod utils;

pub use bitcoin_bosd;
pub use serde_helpers::serde_hex_bytes::*;

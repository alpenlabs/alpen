//! Bridge types for the Strata protocol.
//!
//! This crate contains types related to bridge operations, including operator management,
//! bridge messages, and bridge state management.

mod bridge;
mod bridge_ops;
mod bridge_state;
mod constants;
mod operator;
mod relay;

// Re-export commonly used types
pub use bridge::{
    Musig2PartialSignature, Musig2PubNonce, Musig2SecNonce, OperatorPartialSig, PublickeyTable,
    TxSigningData,
};
pub use bridge_ops::{DepositIntent, WithdrawalBatch, WithdrawalIntent};
pub use bridge_state::{
    CreatedState, DepositEntry, DepositState, DepositsTable, DispatchCommand, DispatchedState,
    FulfilledState, OperatorEntry, OperatorTable, WithdrawOutput,
};
pub use constants::WITHDRAWAL_DENOMINATION;
pub use operator::{OperatorIdx, OperatorKeyProvider, OperatorPubkeys, StubOpKeyProv};
pub use relay::{
    verify_bridge_msg_sig, BridgeMessage, BridgeMsgId, MessageSigner, Scope, VerifyError,
};

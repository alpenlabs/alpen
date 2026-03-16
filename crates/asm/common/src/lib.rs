//! The crate provides common types and traits for building blocks for defining
//! and interacting with subprotocols in an ASM (Anchor State Machine) framework.

mod aux;
mod errors;
mod log;
mod manifest;
mod mmr;
mod msg;
mod spec;
mod state;
mod subprotocol;
mod tx;

#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use aux::*;
pub use errors::*;
pub use log::*;
pub use manifest::*;
pub use mmr::*;
pub use msg::*;
pub use spec::*;
pub use ssz_generated::ssz::{
    self as ssz,
    aux::{
        AuxData, AuxDataRef, AuxRequests, AuxRequestsRef, BitcoinTxid, BitcoinTxidRef,
        ManifestHashRange, ManifestHashRangeRef, RawBitcoinTx, RawBitcoinTxRef,
        VerifiableManifestHash, VerifiableManifestHashRef,
    },
    pow::{
        BtcParams, BtcParamsRef, BtcWork, BtcWorkRef, HeaderVerificationState,
        HeaderVerificationStateRef, TimestampStore, TimestampStoreRef,
    },
    state::{
        AnchorState, AnchorStateRef, AsmHistoryAccumulatorState, AsmHistoryAccumulatorStateRef,
        ChainViewState, ChainViewStateRef, SectionState, SectionStateRef,
    },
};
pub use subprotocol::*;
use tracing as _;
pub use tx::*;

// Re-export the logging module
pub mod logging;

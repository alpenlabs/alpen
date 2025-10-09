mod address;
mod asm_manifest;
pub mod constants;
pub mod errors;
mod legacy;
mod operator;
mod output;
mod params;
mod proof;
mod psbt;
mod pubkey;
mod raw_tx;
mod transaction;
mod txid;

pub use address::BitcoinAddress;
pub use asm_manifest::{AsmLog, AsmManifest};
pub use errors::ParseError;
pub use legacy::{
    DaCommitment, DepositInfo, DepositRequestInfo, DepositSpendInfo,
    L1BlockManifest, L1HeaderRecord, L1Tx, ProtocolOperation, WithdrawalFulfillmentInfo,
};
pub use operator::OperatorPubkeys;
pub use output::OutputRef;
pub use params::BtcParams;
pub use proof::{L1TxInclusionProof, L1TxProof, L1WtxProof, TxIdComputable, TxIdMarker, WtxIdMarker};
pub use psbt::BitcoinPsbt;
pub use pubkey::{BitcoinScriptBuf, XOnlyPk};
pub use raw_tx::RawBitcoinTx;
pub use transaction::{BitcoinTxOut, Outpoint, TaprootSpendPath};
pub use txid::BitcoinTxid;

// re-exports
pub use strata_identifiers::L1BlockId;

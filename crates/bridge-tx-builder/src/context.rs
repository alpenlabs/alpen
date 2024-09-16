//! Provides utilities for building bitcoin transactions for the bridge client by wrapping around
//! [`rust-bitcoin`](bitcoin). These utilities are common to both the bridge-in and bridge-out
//! processes.

use alpen_express_primitives::bridge::PublickeyTable;
use bitcoin::Network;
use musig2::secp256k1::XOnlyPublicKey;

use crate::prelude::get_aggregated_pubkey;

/// Provides methods that allows access to components required to build a transaction in the context
/// of a bitcoin MuSig2 address.
///
/// Please refer to MuSig2 in
/// [BIP 327](https://github.com/bitcoin/bips/blob/master/bip-0327.mediawiki).
pub trait BuildContext {
    /// Get the aggregated MuSig2 x-only pubkey used in the spending condition of the multisig.
    fn aggregated_pubkey(&self) -> XOnlyPublicKey;

    /// Get the bitcoin network for which the builder constructs transactions.
    fn network(&self) -> &Network;
}

/// A builder for raw transactions related to the bridge.
#[derive(Debug, Clone)]
pub struct TxBuildContext {
    /// A table that maps bridge operator indexes to their respective x-only Schnorr pubkeys.
    aggregated_pubkey: XOnlyPublicKey,

    /// The network to build the transactions for.
    network: Network,
}

impl TxBuildContext {
    /// Create a new [`TxBuilder`] with the context required to build transactions of various
    /// [`TxKind`].
    pub fn new(operator_pubkeys: PublickeyTable, network: Network) -> Self {
        let aggregated_pubkey = get_aggregated_pubkey(operator_pubkeys);

        Self {
            aggregated_pubkey,
            network,
        }
    }
}

impl BuildContext for TxBuildContext {
    /// Get the ordered set operator pubkeys.
    fn aggregated_pubkey(&self) -> XOnlyPublicKey {
        self.aggregated_pubkey
    }

    /// Get the bitcoin network for which the builder constructs transactions.
    fn network(&self) -> &Network {
        &self.network
    }
}

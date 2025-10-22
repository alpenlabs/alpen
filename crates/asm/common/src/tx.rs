use bitcoin::Transaction;
use strata_l1_txfmt::TagDataRef;

pub type L1TxIndex = u32;

/// A wrapper containing a reference to a Bitcoin [`Transaction`] together with its
/// parsed SPS-50 payload.
///
/// This struct bundles:
/// 1. `tx`: the original Bitcoin transaction containing the SPS-50 tag in its first output, and
/// 2. `tag`: the extracted [`TagDataRef`], representing the subprotocol's transaction type and any
///    auxiliary data.
#[derive(Debug)]
pub struct TxInputRef<'t> {
    tx: &'t Transaction,
    tag: TagDataRef<'t>,
    /// Index of the transaction within the containing L1 block.
    l1_tx_index: L1TxIndex,
}

impl<'t> TxInputRef<'t> {
    /// Create a new `TxInput` referencing the given `Transaction`.
    pub fn new(tx: &'t Transaction, tag: TagDataRef<'t>, l1_tx_index: L1TxIndex) -> Self {
        TxInputRef {
            tx,
            tag,
            l1_tx_index,
        }
    }

    /// Gets the inner transaction.
    pub fn tx(&self) -> &Transaction {
        self.tx
    }

    /// Returns a reference to the parsed SPS-50 tag payload for this transaction,
    /// which contains the subprotocol-specific transaction type and auxiliary data.
    pub fn tag(&self) -> &TagDataRef<'t> {
        &self.tag
    }

    /// Returns the position of this transaction within the L1 block.
    pub fn l1_tx_index(&self) -> L1TxIndex {
        self.l1_tx_index
    }
}

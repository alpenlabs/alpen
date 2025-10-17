use bitcoin::Transaction;
use strata_l1_txfmt::TagDataRef;

/// Index of the originating L1 transaction within the block.
///
/// A Bitcoin block today carries at most a few thousand transactions, so we keep this compact.
pub type RequesterL1Index = u16;

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
    index: RequesterL1Index,
}

impl<'t> TxInputRef<'t> {
    /// Create a new `TxInput` referencing the given `Transaction`.
    pub fn new(tx: &'t Transaction, tag: TagDataRef<'t>, index: RequesterL1Index) -> Self {
        TxInputRef { tx, tag, index }
    }

    /// create a new `TxInput` referencing the given `Transaction` with index 0 for tests
    pub fn new_for_test(tx: &'t Transaction, tag: TagDataRef<'t>) -> Self {
        TxInputRef { tx, tag, index: 0 }
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

    /// Returns the index of the underlying L1 transaction within the block.
    pub fn index(&self) -> RequesterL1Index {
        self.index
    }
}

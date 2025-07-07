use bitcoin::Transaction;
use strata_l1_txfmt::TagDataRef;

/// A wrapper containing a reference to a Bitcoin [`Transaction`] together with its
/// parsed SPS-50 payload.
///
/// This struct bundles:
/// 1. `tx`: the original Bitcoin transaction containing the SPS-50 tag in its first output, and
/// 2. `tag`: the extracted [`TagDataRef`], representing the subprotocol's transaction type and any
///    auxiliary data.
#[derive(Debug)]
pub struct TxInput<'t> {
    tx: &'t Transaction,
    tag: TagDataRef<'t>,
}

impl<'t> TxInput<'t> {
    /// Create a new `TxInput` referencing the given `Transaction`.
    pub fn new(tx: &'t Transaction, tag: TagDataRef<'t>) -> Self {
        TxInput { tx, tag }
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
}

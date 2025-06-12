use bitcoin::Transaction;

/// A parsed SPS-50 tag payload (excluding the "ALPN" magic and subprotocol ID),
/// containing the subprotocol-specific transaction type and any auxiliary data.
///
/// This struct represents everything in the OP_RETURN after the first 6 bytes:
/// 1. Byte 0: subprotocol-defined transaction type
/// 2. Bytes 1…: auxiliary payload (type-specific)
#[derive(Debug)]
pub struct TagPayload<'p> {
    /// The transaction type as defined by the SPS-50 subprotocol.
    tx_type: u8,

    /// The remaining, type-specific payload for this transaction.
    auxiliary_data: &'p [u8],
}

impl<'p> TagPayload<'p> {
    /// Constructs a new `Sps50TagPayload`.
    pub fn new(tx_type: u8, auxiliary_data: &'p [u8]) -> Self {
        Self {
            tx_type,
            auxiliary_data,
        }
    }

    /// Returns the subprotocol-defined transaction type.
    pub fn tx_type(&self) -> u8 {
        self.tx_type
    }

    /// Returns the auxiliary data slice associated with this tag.
    pub fn aux_data(&self) -> &[u8] {
        self.auxiliary_data
    }
}

/// A wrapper containing a reference to a Bitcoin `Transaction` together with its
/// parsed SPS-50 payload.
///
/// This struct bundles:
/// 1. `tx`: the original Bitcoin transaction containing the SPS-50 tag in its first output, and
/// 2. `tag`: the extracted `TagPayload`, representing the subprotocol’s transaction type and any
///    auxiliary data.
#[derive(Debug)]
pub struct TxInput<'t> {
    tx: &'t Transaction,
    tag: TagPayload<'t>,
}

impl<'t> TxInput<'t> {
    /// Create a new `TxInput` referencing the given `Transaction`.
    pub fn new(tx: &'t Transaction, sps_50_info: TagPayload<'t>) -> Self {
        TxInput {
            tx,
            tag: sps_50_info,
        }
    }

    /// Gets the inner transaction.
    pub fn tx(&self) -> &Transaction {
        self.tx
    }

    /// Returns a reference to the parsed SPS-50 tag payload for this transaction,
    /// which contains the subprotocol-specific transaction type and auxiliary data.
    pub fn tag(&self) -> &TagPayload<'t> {
        &self.tag
    }
}

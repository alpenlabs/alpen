use strata_primitives::{
    indexed::Indexed,
    l1::{DaCommitment, DepositRequestInfo, ProtocolOperation},
};

/// Container for the different kinds of messages that we could extract from a L1 tx.
#[derive(Clone, Debug)]
pub struct L1TxMessages {
    /// Protocol consensus operations relevant to STF.
    protocol_ops: Vec<ProtocolOperation>,

    /// Deposit requests which the node stores for non-stf related bookkeeping.
    deposit_reqs: Vec<DepositRequestInfo>,

    /// DA entries which the node stores for state reconstruction.  These MUST
    /// reflect messages found in `ProtocolOperation`.
    da_entries: Vec<DaEntry>,
}

impl L1TxMessages {
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(
        protocol_ops: Vec<ProtocolOperation>,
        deposit_reqs: Vec<DepositRequestInfo>,
        da_entries: Vec<DaEntry>,
    ) -> Self {
        Self {
            protocol_ops,
            deposit_reqs,
            da_entries,
        }
    }

    #[expect(clippy::missing_const_for_fn)]
    pub fn protocol_ops(&self) -> &[ProtocolOperation] {
        &self.protocol_ops
    }

    #[expect(clippy::missing_const_for_fn)]
    pub fn deposit_reqs(&self) -> &[DepositRequestInfo] {
        &self.deposit_reqs
    }

    #[expect(clippy::missing_const_for_fn)]
    pub fn da_entries(&self) -> &[DaEntry] {
        &self.da_entries
    }

    pub fn into_parts(
        self,
    ) -> (
        Vec<ProtocolOperation>,
        Vec<DepositRequestInfo>,
        Vec<DaEntry>,
    ) {
        (self.protocol_ops, self.deposit_reqs, self.da_entries)
    }
}

/// DA commitment and blob retrieved from L1 transaction.
#[derive(Clone, Debug)]
pub struct DaEntry {
    #[allow(unused)]
    commitment: DaCommitment,

    #[allow(unused)]
    blob_buf: Vec<u8>,
}

impl DaEntry {
    /// Creates a new `DaEntry` instance without checking that the commitment
    /// actually corresponds to the blob.
    pub const fn new_unchecked(commitment: DaCommitment, blob_buf: Vec<u8>) -> Self {
        Self {
            commitment,
            blob_buf,
        }
    }

    /// Creates a new instance for a blob, generating the commitment.
    pub fn new(blob: Vec<u8>) -> Self {
        let commitment = DaCommitment::from_buf(&blob);
        Self::new_unchecked(commitment, blob)
    }

    /// Creates a new instance from an iterator over contiguous chunks of bytes.
    ///
    /// This is intended to be used when extracting data from an in-situ bitcoin
    /// tx, which has a requirement that data is in 520 byte chunks.
    pub fn from_chunks<'a>(chunks: impl Iterator<Item = &'a [u8]>) -> Self {
        // I'm not sure if I can just like `.flatten().copied().collect()` this
        // efficiently how it looks like you can.
        let mut buf = Vec::new();
        chunks.for_each(|chunk| buf.extend_from_slice(chunk));

        Self::new(buf)
    }

    pub const fn commitment(&self) -> &DaCommitment {
        &self.commitment
    }

    #[expect(clippy::missing_const_for_fn)]
    pub fn blob_buf(&self) -> &[u8] {
        &self.blob_buf
    }

    pub fn into_blob_buf(self) -> Vec<u8> {
        self.blob_buf
    }
}

/// Indexed tx entry with some messages.
pub type RelevantTxEntry = Indexed<L1TxMessages>;

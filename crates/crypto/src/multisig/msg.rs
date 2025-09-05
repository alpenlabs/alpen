use strata_primitives::buf::Buf32;

/// A multisig payload comprising an operation plus a nonce, ready for hashing and signing.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MultisigPayload {
    op: Buf32,
    nonce: u64,
}

impl MultisigPayload {
    /// Create a new multisig payload.
    pub fn new(op: Buf32, nonce: u64) -> Self {
        Self { op, nonce }
    }

    /// Borrow the multisig operation.
    pub fn op(&self) -> &Buf32 {
        &self.op
    }

    /// The nonce associated with this payload.
    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Consume and return the inner `(MultisigOp, u64)`.
    pub fn into_inner(self) -> (Buf32, u64) {
        (self.op, self.nonce)
    }
}

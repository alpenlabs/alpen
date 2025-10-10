use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// ASM execution manifest containing encoded logs and header data.
///
/// This type represents the output of ASM execution and contains encoded data
/// that doesn't require decoding Bitcoin types, making it suitable for use in
/// contexts that cannot depend on the bitcoin crate (e.g., ledger-types).
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AsmManifest {
    /// Encoded ASM logs as raw bytes
    pub encoded_logs: Vec<u8>,

    /// Encoded header as raw bytes (not decoded bitcoin::Header)
    pub encoded_header: Vec<u8>,

    /// Additional encoded data from ASM execution
    pub encoded_data: Vec<u8>,

    // TODO: Add other fields as needed based on actual ASM execution output
}

impl AsmManifest {
    pub fn new(encoded_logs: Vec<u8>, encoded_header: Vec<u8>, encoded_data: Vec<u8>) -> Self {
        Self {
            encoded_logs,
            encoded_header,
            encoded_data,
        }
    }

    pub fn encoded_logs(&self) -> &[u8] {
        &self.encoded_logs
    }

    pub fn encoded_header(&self) -> &[u8] {
        &self.encoded_header
    }

    pub fn encoded_data(&self) -> &[u8] {
        &self.encoded_data
    }
}

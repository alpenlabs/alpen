use arbitrary::Arbitrary;
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Witness, Txid, opcodes, hashes::Hash};

/// Aggressive script fuzzing - tests all edge cases
#[derive(Arbitrary, Debug, Clone)]
pub enum FuzzScriptType {
    Empty,
    OpReturn,
    /// Arbitrary script bytes - can be invalid, oversized, malformed
    Arbitrary(Vec<u8>),
    /// Standard P2PKH-like structure but with fuzzed keys
    P2PKH([u8; 20]),
    /// Standard P2WPKH-like structure but with fuzzed keys
    P2WPKH([u8; 20]),
    /// Standard P2SH-like structure
    P2SH([u8; 20]),
    /// Nested scripts - can create pathological cases
    Nested { outer: Box<FuzzScriptType>, inner: Vec<u8> },
    /// Push a specific amount of data (tests push size limits)
    PushData(Vec<u8>),
}

impl FuzzScriptType {
    pub fn to_script(&self) -> ScriptBuf {
        match self {
            Self::Empty => ScriptBuf::new(),
            Self::OpReturn => ScriptBuf::new_op_return([0x42]),
            Self::Arbitrary(bytes) => ScriptBuf::from(bytes.clone()),
            Self::P2PKH(hash) => {
                ScriptBuf::builder()
                    .push_opcode(opcodes::all::OP_DUP)
                    .push_opcode(opcodes::all::OP_HASH160)
                    .push_slice(hash)
                    .push_opcode(opcodes::all::OP_EQUALVERIFY)
                    .push_opcode(opcodes::all::OP_CHECKSIG)
                    .into_script()
            }
            Self::P2WPKH(hash) => {
                ScriptBuf::builder()
                    .push_opcode(opcodes::all::OP_PUSHBYTES_0)
                    .push_slice(hash)
                    .into_script()
            }
            Self::P2SH(hash) => {
                ScriptBuf::builder()
                    .push_opcode(opcodes::all::OP_HASH160)
                    .push_slice(hash)
                    .push_opcode(opcodes::all::OP_EQUAL)
                    .into_script()
            }
            Self::Nested { outer, inner } => {
                let mut script = outer.to_script();
                // Append inner bytes - can create malformed scripts
                let mut bytes = script.to_bytes();
                bytes.extend_from_slice(inner);
                ScriptBuf::from(bytes)
            }
            Self::PushData(data) => {
                if data.is_empty() {
                    ScriptBuf::new()
                } else {
                    // For arbitrary data, construct script directly from bytes
                    // This allows testing pathological cases
                    let mut script_bytes = Vec::new();
                    // Add a push opcode if data is small enough
                    if data.len() <= 75 {
                        script_bytes.push(data.len() as u8);
                        script_bytes.extend_from_slice(data);
                    } else {
                        // Just append the data as-is (malformed but tests parser)
                        script_bytes.extend_from_slice(data);
                    }
                    ScriptBuf::from(script_bytes)
                }
            }
        }
    }
}

/// Aggressive OP_RETURN script generation - doesn't use ParseConfig encoder
/// This decouples script production from consumption
#[derive(Arbitrary, Debug, Clone)]
pub enum ArbitraryOpReturnScript {
    /// Empty script (not actually OP_RETURN)
    Empty,
    /// Pure OP_RETURN with no data
    JustOpReturn,
    /// OP_RETURN with arbitrary bytes (might not match expected format)
    OpReturnWithData(Vec<u8>),
    /// Multiple OP_RETURNs (invalid but tests parser robustness)
    MultipleOpReturns(Vec<Vec<u8>>),
    /// OP_RETURN at wrong position in script
    OpReturnAtEnd(Vec<u8>),
    /// Malformed: pushes before OP_RETURN
    PushThenOpReturn { prefix: Vec<u8>, data: Vec<u8> },
    /// Almost valid format but with bit flips
    AlmostValid { magic: [u8; 4], subprotocol: u8, tx_type: u8, aux: Vec<u8> },
    /// Arbitrary script that's not OP_RETURN at all
    NotOpReturn(Vec<u8>),
}

impl ArbitraryOpReturnScript {
    pub fn to_script(&self) -> ScriptBuf {
        match self {
            Self::Empty => ScriptBuf::new(),
            Self::JustOpReturn => {
                ScriptBuf::from(vec![opcodes::all::OP_RETURN.to_u8()])
            }
            Self::OpReturnWithData(data) => {
                let mut script_bytes = vec![opcodes::all::OP_RETURN.to_u8()];
                if !data.is_empty() {
                    // Construct malformed push - just append data without proper push opcode
                    // This tests parser robustness to malformed OP_RETURN
                    if data.len() <= 75 {
                        script_bytes.push(data.len() as u8); // OP_PUSHBYTES_N
                    } else if data.len() <= 255 {
                        script_bytes.push(0x4c); // OP_PUSHDATA1
                        script_bytes.push(data.len() as u8);
                    } else {
                        // Just append - malformed but tests parser
                        script_bytes.push(0x4d); // OP_PUSHDATA2
                        script_bytes.extend_from_slice(&(data.len() as u16).to_le_bytes());
                    }
                    script_bytes.extend_from_slice(data);
                }
                ScriptBuf::from(script_bytes)
            }
            Self::MultipleOpReturns(datas) => {
                let mut script_bytes = Vec::new();
                for data in datas {
                    script_bytes.push(opcodes::all::OP_RETURN.to_u8());
                    if !data.is_empty() {
                        if data.len() <= 75 {
                            script_bytes.push(data.len() as u8);
                        }
                        script_bytes.extend_from_slice(data);
                    }
                }
                ScriptBuf::from(script_bytes)
            }
            Self::OpReturnAtEnd(prefix) => {
                let mut script_bytes = Vec::new();
                script_bytes.extend_from_slice(prefix);
                script_bytes.push(opcodes::all::OP_RETURN.to_u8());
                ScriptBuf::from(script_bytes)
            }
            Self::PushThenOpReturn { prefix, data } => {
                let mut script_bytes = Vec::new();
                if !prefix.is_empty() {
                    if prefix.len() <= 75 {
                        script_bytes.push(prefix.len() as u8);
                    }
                    script_bytes.extend_from_slice(prefix);
                }
                script_bytes.push(opcodes::all::OP_RETURN.to_u8());
                if !data.is_empty() {
                    if data.len() <= 75 {
                        script_bytes.push(data.len() as u8);
                    }
                    script_bytes.extend_from_slice(data);
                }
                ScriptBuf::from(script_bytes)
            }
            Self::AlmostValid { magic, subprotocol, tx_type, aux } => {
                // Construct something that looks like valid format but might have issues
                let mut payload = Vec::new();
                payload.extend_from_slice(magic);
                payload.push(*subprotocol);
                payload.push(*tx_type);
                payload.extend_from_slice(aux);

                let mut script_bytes = vec![opcodes::all::OP_RETURN.to_u8()];
                if payload.len() <= 75 {
                    script_bytes.push(payload.len() as u8);
                } else if payload.len() <= 255 {
                    script_bytes.push(0x4c);
                    script_bytes.push(payload.len() as u8);
                }
                script_bytes.extend_from_slice(&payload);
                ScriptBuf::from(script_bytes)
            }
            Self::NotOpReturn(bytes) => ScriptBuf::from(bytes.clone()),
        }
    }
}

/// Aggressive transaction input fuzzing
#[derive(Arbitrary, Debug, Clone)]
pub struct FuzzTxInput {
    /// Fuzz the outpoint
    pub txid: [u8; 32],
    pub vout: u32,

    /// Fuzz script_sig
    pub script_sig_type: FuzzScriptType,

    /// Fuzz sequence
    pub sequence: u32,

    /// Fuzz witness - this is critical for SegWit fuzzing
    pub witness_count: u8,
    pub witness_items: [Vec<u8>; 5],
}

impl FuzzTxInput {
    pub fn to_txin(&self) -> TxIn {
        let outpoint = OutPoint {
            txid: Txid::from_slice(&self.txid).unwrap_or(Txid::all_zeros()),
            vout: self.vout,
        };

        // Construct witness with variable item count
        let mut witness = Witness::new();
        let count = (self.witness_count as usize).min(self.witness_items.len());
        for i in 0..count {
            witness.push(&self.witness_items[i]);
        }

        TxIn {
            previous_output: outpoint,
            script_sig: self.script_sig_type.to_script(),
            sequence: Sequence::from_consensus(self.sequence),
            witness,
        }
    }
}

/// Create a truly minimal/empty input (for fallback cases)
pub fn create_dummy_input() -> TxIn {
    TxIn {
        previous_output: OutPoint::null(),
        script_sig: ScriptBuf::new(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    }
}

/// Aggressive transaction output fuzzing
#[derive(Arbitrary, Debug, Clone)]
pub struct FuzzTxOutput {
    /// Fuzz amount - including edge cases
    pub amount: u64,
    pub script_type: FuzzScriptType,
    /// Sometimes create dust outputs
    pub force_dust: bool,
    /// Sometimes create MAX_MONEY violations
    pub force_overflow: bool,
}

impl FuzzTxOutput {
    pub fn to_txout(&self) -> TxOut {
        let amount = if self.force_dust {
            // Dust threshold is typically 546 sats
            self.amount % 600
        } else if self.force_overflow {
            // Close to or over MAX_MONEY (21M BTC = 2.1e15 sats)
            u64::MAX
        } else {
            self.amount
        };

        TxOut {
            value: Amount::from_sat(amount),
            script_pubkey: self.script_type.to_script(),
        }
    }
}

//! Bridge transaction data structures
//!
//! This module contains the core data structures for bridge operations,
//! adapted from the mock-bridge implementation for use in python-utils.

use bdk_wallet::bitcoin::{
    consensus, script::PushBytesBuf, Amount, OutPoint, Psbt, ScriptBuf, TapNodeHash, TxOut, Txid,
    XOnlyPublicKey,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DepositRequestData {
    /// The deposit request transaction outpoints from the users.
    pub deposit_request_outpoint: OutPoint,

    /// The stake index that will be tied to this deposit.
    ///
    /// This is required in order to make sure that the at withdrawal time, deposit UTXOs are
    /// assigned in the same order that the stake transactions were linked during setup time
    ///
    /// # Note
    ///
    /// The stake index must be encoded in 4-byte big-endian.
    pub stake_index: u32,

    /// The execution environment address to mint the equivalent tokens to.
    /// As of now, this is just the 20-byte EVM address.
    pub ee_address: Vec<u8>,

    /// The amount in bitcoins that the user is sending.
    ///
    /// This amount should be greater than the bridge denomination for the deposit to be
    /// confirmed on bitcoin. The excess amount is used as miner fees for the Deposit Transaction.
    pub total_amount: Amount,

    /// The [`XOnlyPublicKey`] in the Deposit Request Transaction (DRT) as provided by the
    /// user in their `OP_RETURN` output.
    pub x_only_public_key: XOnlyPublicKey,

    /// The original script_pubkey in the Deposit Request Transaction (DRT) output used to sanity
    /// check computation internally i.e., whether the known information (n/n script spend path,
    /// [`static@UNSPENDABLE_INTERNAL_KEY`]) + the [`Self::take_back_leaf_hash`] yields the same
    /// P2TR address.
    pub original_script_pubkey: ScriptBuf,
}

/// Deposit Transaction structure with PSBT and metadata
#[derive(Debug, Clone)]
pub(crate) struct DepositTx {
    psbt: Psbt,
    prevouts: Vec<TxOut>,
    witnesses: Vec<TaprootWitness>,
}

/// Withdrawal fulfillment transaction metadata
#[derive(Debug, Clone)]
pub(crate) struct WithdrawalMetadata {
    /// The tag used to mark the withdrawal metadata transaction
    pub tag: [u8; 4],

    /// The index of the operator
    pub operator_idx: u32,

    /// The index of the deposit
    pub deposit_idx: u32,

    /// The txid of the deposit UTXO
    pub deposit_txid: Txid,
}

/// Deposit transaction metadata for OP_RETURN
#[derive(Debug, Clone)]
pub(crate) struct DepositTxMetadata {
    pub stake_index: u32,
    pub ee_address: Vec<u8>,
    pub takeback_hash: TapNodeHash,
    pub input_amount: Amount,
}

/// Auxiliary data for deposit transactions
#[derive(Debug, Clone)]
pub(crate) struct AuxiliaryData {
    pub tag: String,
    pub metadata: DepositTxMetadata,
}

/// Taproot witness types adapted from mock-bridge
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaprootWitness {
    /// Use the keypath spend
    #[allow(dead_code)]
    Key,

    /// Use the script path spend with script and control block
    #[allow(dead_code)]
    Script {
        script_buf: ScriptBuf,
        control_block: bdk_wallet::bitcoin::taproot::ControlBlock,
    },

    /// Use the keypath spend tweaked with some known hash
    Tweaked { tweak: TapNodeHash },
}

// DepositTx implementations
impl DepositTx {
    pub(crate) fn new(psbt: Psbt, prevouts: Vec<TxOut>, witnesses: Vec<TaprootWitness>) -> Self {
        Self {
            psbt,
            prevouts,
            witnesses,
        }
    }

    pub(crate) fn psbt(&self) -> &Psbt {
        &self.psbt
    }

    pub(crate) fn psbt_mut(&mut self) -> &mut Psbt {
        &mut self.psbt
    }

    pub(crate) fn prevouts(&self) -> &[TxOut] {
        &self.prevouts
    }

    pub(crate) fn witnesses(&self) -> &[TaprootWitness] {
        &self.witnesses
    }
}

// WithdrawalMetadata implementations
impl WithdrawalMetadata {
    pub(crate) fn new(
        tag: [u8; 4],
        operator_idx: u32,
        deposit_idx: u32,
        deposit_txid: Txid,
    ) -> Self {
        Self {
            tag,
            operator_idx,
            deposit_idx,
            deposit_txid,
        }
    }

    pub(crate) fn op_return_data(&self) -> Vec<u8> {
        let op_id_prefix: [u8; 4] = self.operator_idx.to_be_bytes();
        let deposit_id_prefix: [u8; 4] = self.deposit_idx.to_be_bytes();
        let deposit_txid_data = consensus::encode::serialize(&self.deposit_txid);
        [
            &self.tag[..],
            &op_id_prefix[..],
            &deposit_id_prefix[..],
            &deposit_txid_data[..],
        ]
        .concat()
    }

    pub(crate) fn op_return_script(&self) -> PushBytesBuf {
        let data = self.op_return_data();
        let mut push_data = PushBytesBuf::new();

        push_data
            .extend_from_slice(&data)
            .expect("metadata should be within push data limits");

        push_data
    }
}

// AuxiliaryData implementations
impl AuxiliaryData {
    pub(crate) fn new(tag: String, metadata: DepositTxMetadata) -> Self {
        Self { tag, metadata }
    }

    pub(crate) fn to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(self.tag.as_bytes());
        buf.extend_from_slice(&self.metadata.stake_index.to_be_bytes());
        buf.extend_from_slice(&self.metadata.ee_address);
        buf.extend_from_slice(self.metadata.takeback_hash.as_ref());
        buf.extend_from_slice(&self.metadata.input_amount.to_sat().to_be_bytes());
        buf
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bdk_wallet::bitcoin::{hashes::Hash, Transaction, TxIn, Witness};

    use super::*;

    fn create_test_deposit_request_data() -> DepositRequestData {
        DepositRequestData {
            deposit_request_outpoint: OutPoint::from_str(
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef:0",
            )
            .unwrap(),
            stake_index: 42,
            ee_address: vec![
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
                0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
            ],
            total_amount: Amount::from_sat(100000),
            x_only_public_key: XOnlyPublicKey::from_str(
                "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .unwrap(),
            original_script_pubkey: ScriptBuf::new(),
        }
    }

    #[test]
    fn test_deposit_request_data_creation() {
        let data = create_test_deposit_request_data();

        assert_eq!(data.stake_index, 42);
        assert_eq!(data.ee_address.len(), 20);
        assert_eq!(data.total_amount, Amount::from_sat(100000));
        assert_eq!(data.original_script_pubkey, ScriptBuf::new());
    }

    #[test]
    fn test_deposit_tx_creation_and_accessors() {
        let transaction = Transaction {
            version: bdk_wallet::bitcoin::transaction::Version::TWO,
            lock_time: bdk_wallet::bitcoin::locktime::absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::from_str(
                    "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef:0",
                )
                .unwrap(),
                script_sig: ScriptBuf::new(),
                sequence: bdk_wallet::bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(100000),
                script_pubkey: ScriptBuf::new(),
            }],
        };

        let psbt = Psbt::from_unsigned_tx(transaction).unwrap();
        let prevouts = vec![TxOut {
            value: Amount::from_sat(200000),
            script_pubkey: ScriptBuf::new(),
        }];
        let witnesses = vec![TaprootWitness::Key];

        let deposit_tx = DepositTx::new(psbt.clone(), prevouts.clone(), witnesses.clone());

        // Test accessors
        assert_eq!(deposit_tx.psbt().unsigned_tx.input.len(), 1);
        assert_eq!(deposit_tx.prevouts().len(), 1);
        assert_eq!(deposit_tx.witnesses().len(), 1);
        assert_eq!(deposit_tx.prevouts()[0].value, Amount::from_sat(200000));
    }

    #[test]
    fn test_withdrawal_metadata_creation() {
        let tag = [0x01, 0x02, 0x03, 0x04];
        let operator_idx = 123;
        let deposit_idx = 456;
        let deposit_txid =
            Txid::from_str("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                .unwrap();

        let metadata = WithdrawalMetadata::new(tag, operator_idx, deposit_idx, deposit_txid);

        assert_eq!(metadata.tag, tag);
        assert_eq!(metadata.operator_idx, operator_idx);
        assert_eq!(metadata.deposit_idx, deposit_idx);
        assert_eq!(metadata.deposit_txid, deposit_txid);
    }

    #[test]
    fn test_withdrawal_metadata_op_return_data() {
        let tag = [0xAB, 0xCD, 0xEF, 0x12];
        let operator_idx = 0x12345678;
        let deposit_idx = 0x87654321;
        let deposit_txid =
            Txid::from_str("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890")
                .unwrap();

        let metadata = WithdrawalMetadata::new(tag, operator_idx, deposit_idx, deposit_txid);
        let op_return_data = metadata.op_return_data();

        // Verify structure: tag(4) + operator_idx(4) + deposit_idx(4) + txid(32) = 44 bytes
        assert_eq!(op_return_data.len(), 44);

        // Verify tag
        assert_eq!(&op_return_data[0..4], &tag);

        // Verify operator index (big-endian)
        assert_eq!(&op_return_data[4..8], &[0x12, 0x34, 0x56, 0x78]);

        // Verify deposit index (big-endian)
        assert_eq!(&op_return_data[8..12], &[0x87, 0x65, 0x43, 0x21]);

        // Verify txid serialization
        let serialized_txid = consensus::encode::serialize(&deposit_txid);
        assert_eq!(&op_return_data[12..], &serialized_txid);
    }

    #[test]
    fn test_withdrawal_metadata_op_return_script() {
        let metadata = WithdrawalMetadata::new(
            [0x01, 0x02, 0x03, 0x04],
            123,
            456,
            Txid::from_str("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
                .unwrap(),
        );

        let script_data = metadata.op_return_script();
        let expected_data = metadata.op_return_data();

        // Verify the script contains the expected data
        assert_eq!(script_data.as_bytes(), &expected_data);
        assert!(script_data.len() <= 75); // OP_PUSHDATA limit
    }

    #[test]
    fn test_deposit_tx_metadata_creation() {
        let metadata = DepositTxMetadata {
            stake_index: 100,
            ee_address: vec![0x11, 0x22, 0x33],
            takeback_hash: TapNodeHash::from_byte_array([0x55; 32]),
            input_amount: Amount::from_sat(50000),
        };

        assert_eq!(metadata.stake_index, 100);
        assert_eq!(metadata.ee_address, vec![0x11, 0x22, 0x33]);
        assert_eq!(metadata.input_amount, Amount::from_sat(50000));
    }

    #[test]
    fn test_auxiliary_data_serialization() {
        let metadata = DepositTxMetadata {
            stake_index: 0x12345678,
            ee_address: vec![0xAA, 0xBB, 0xCC],
            takeback_hash: TapNodeHash::from_byte_array([0x77; 32]),
            input_amount: Amount::from_sat(0x123456789ABCDEF0),
        };

        let aux_data = AuxiliaryData::new("TEST".to_string(), metadata);
        let serialized = aux_data.to_vec();

        // Verify structure: tag(4) + stake_index(4) + ee_address(3) + takeback_hash(32) + amount(8)
        let expected_len = 4 + 4 + 3 + 32 + 8;
        assert_eq!(serialized.len(), expected_len);

        // Verify tag
        assert_eq!(&serialized[0..4], b"TEST");

        // Verify stake index (big-endian)
        assert_eq!(&serialized[4..8], &[0x12, 0x34, 0x56, 0x78]);

        // Verify ee_address
        assert_eq!(&serialized[8..11], &[0xAA, 0xBB, 0xCC]);

        // Verify takeback hash
        assert_eq!(&serialized[11..43], &[0x77; 32]);

        // Verify amount (big-endian)
        assert_eq!(
            &serialized[43..51],
            &[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]
        );
    }

    #[test]
    fn test_taproot_witness_variants() {
        let key_witness = TaprootWitness::Key;
        let tweaked_witness = TaprootWitness::Tweaked {
            tweak: TapNodeHash::from_byte_array([0x42; 32]),
        };

        // Test equality for simple variants
        assert_eq!(key_witness, TaprootWitness::Key);
        assert_eq!(
            tweaked_witness,
            TaprootWitness::Tweaked {
                tweak: TapNodeHash::from_byte_array([0x42; 32])
            }
        );

        // Test inequality
        assert_ne!(key_witness, tweaked_witness);

        // Test different tweaks
        let different_tweaked = TaprootWitness::Tweaked {
            tweak: TapNodeHash::from_byte_array([0x43; 32]),
        };
        assert_ne!(tweaked_witness, different_tweaked);
    }

    #[test]
    fn test_large_auxiliary_data() {
        let large_ee_address = vec![0xFF; 100]; // Larger than typical 20-byte address
        let metadata = DepositTxMetadata {
            stake_index: u32::MAX,
            ee_address: large_ee_address.clone(),
            takeback_hash: TapNodeHash::from_byte_array([0xFF; 32]),
            input_amount: Amount::from_sat(u64::MAX),
        };

        let aux_data = AuxiliaryData::new("LARGE_TAG_TEST".to_string(), metadata);
        let serialized = aux_data.to_vec();

        // Should handle large data without panic
        assert!(serialized.len() > 100);
        assert_eq!(&serialized[0..14], b"LARGE_TAG_TEST");
        assert_eq!(&serialized[18..118], &large_ee_address);
    }
}

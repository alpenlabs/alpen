/// Utilities for testing counter-proof functionality.
pub mod mock_transaction {
    use bitcoin::{
        absolute::LockTime,
        blockdata::{
            opcodes::all::OP_RETURN,
            script::{Builder, PushBytesBuf},
        },
        hashes::Hash,
        key::Secp256k1,
        secp256k1,
        sighash::{Prevouts, SighashCache},
        transaction::Version,
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
    };
    use secp256k1::{Keypair, Message, Scalar, SecretKey, XOnlyPublicKey};

    const SCALAR_BYTES_LEN: usize = 32;
    const DEPOSIT_INDEX_BYTES: usize = 4;

    fn create_deposit_scalar(deposit_index: u32) -> Scalar {
        let mut scalar_bytes = [0u8; SCALAR_BYTES_LEN];
        scalar_bytes[..DEPOSIT_INDEX_BYTES].copy_from_slice(&deposit_index.to_le_bytes());
        Scalar::from_le_bytes(scalar_bytes).expect("invalid deposit index scalar")
    }

    /// Builder for creating mock Taproot transactions for testing
    #[derive(Debug)]
    pub struct MockTransactionBuilder {
        master_key: Option<[u8; 32]>,
        deposit_index: Option<u32>,
        input_value: Amount,
        sequence: Sequence,
        lock_time: LockTime,
        version: Version,
        mock_txid_byte: u8,
        op_return_data: Vec<u8>,
    }

    impl MockTransactionBuilder {
        /// Creates a new instance of the builder
        pub fn new() -> Self {
            Self {
                master_key: None,
                deposit_index: None,
                input_value: Amount::from_sat(1000),
                sequence: Sequence::from_consensus(0xFFFFFFFD),
                lock_time: LockTime::ZERO,
                version: Version::TWO,
                mock_txid_byte: 0x11,
                op_return_data: vec![0x42u8; 292],
            }
        }

        /// Sets the master private key for the transaction
        pub fn master_key(mut self, master_key: [u8; 32]) -> Self {
            self.master_key = Some(master_key);
            self
        }

        /// Sets the deposit index for the transaction
        pub fn deposit_index(mut self, deposit_index: u32) -> Self {
            self.deposit_index = Some(deposit_index);
            self
        }

        /// Sets the OP_RETURN data for the transaction
        pub fn op_return_data(mut self, data: Vec<u8>) -> Self {
            self.op_return_data = data;
            self
        }

        /// Builds the mock transaction
        pub fn build(self) -> (Transaction, Vec<TxOut>) {
            let master_key = self.master_key.expect("master_key must be set");
            let deposit_index = self.deposit_index.expect("deposit_index must be set");

            let signing_keypair = derive_deposit_keypair(&master_key, deposit_index);

            // Create prevout
            let prevout = TxOut {
                value: self.input_value,
                script_pubkey: ScriptBuf::new(), // Empty for Taproot key-spend
            };
            let prevouts = vec![prevout];

            // Create input
            let mock_txid = Txid::from_slice(&[self.mock_txid_byte; 32])
                .expect("32 bytes should create valid txid");

            let input = TxIn {
                previous_output: OutPoint {
                    txid: mock_txid,
                    vout: 0,
                },
                script_sig: ScriptBuf::new(),
                sequence: self.sequence,
                witness: Witness::new(),
            };

            // Create outputs
            let outputs = vec![create_op_return_output(&self.op_return_data)];

            let mut transaction = Transaction {
                version: self.version,
                lock_time: self.lock_time,
                input: vec![input],
                output: outputs,
            };

            // Sign the transaction
            let secp = Secp256k1::new();
            let mut sighash_cache = SighashCache::new(&transaction);

            let sighash = sighash_cache
                .taproot_key_spend_signature_hash(
                    0,
                    &Prevouts::All(&prevouts),
                    bitcoin::TapSighashType::Default,
                )
                .expect("taproot sighash error");

            let message = Message::from(sighash);
            let signature = secp.sign_schnorr(&message, &signing_keypair);

            let mut witness = Witness::new();
            witness.push(signature.as_ref());
            transaction.input[0].witness = witness;

            (transaction, prevouts)
        }
    }

    impl Default for MockTransactionBuilder {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Derives master x-only public key from master secret key bytes
    pub fn derive_master_pubkey(master_secret_bytes: &[u8; 32]) -> XOnlyPublicKey {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(master_secret_bytes).expect("invalid master secret key");
        let kp = Keypair::from_secret_key(&secp, &sk);
        XOnlyPublicKey::from_keypair(&kp).0
    }

    fn derive_deposit_keypair(master_secret_bytes: &[u8; 32], deposit_index: u32) -> Keypair {
        let secp = Secp256k1::new();

        let master_secret =
            SecretKey::from_slice(master_secret_bytes).expect("invalid master secret");

        let master_keypair = Keypair::from_secret_key(&secp, &master_secret);
        let tweak = create_deposit_scalar(deposit_index);

        master_keypair
            .add_xonly_tweak(&secp, &tweak)
            .expect("deposit index is invalid tweak for this keypair")
    }

    fn create_op_return_output(data: &[u8]) -> TxOut {
        let mut push_data = PushBytesBuf::new();
        push_data
            .extend_from_slice(data)
            .expect("data should fit in push data");

        let script = Builder::new()
            .push_opcode(OP_RETURN)
            .push_slice(push_data)
            .into_script();

        TxOut {
            value: Amount::ZERO,
            script_pubkey: script,
        }
    }
}

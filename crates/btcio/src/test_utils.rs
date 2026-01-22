use std::{collections::BTreeMap, str::FromStr};

use bitcoin::{
    absolute::{Height, LockTime},
    bip32::Xpriv,
    block::Header,
    consensus::{self, deserialize},
    hashes::Hash,
    key::{Parity, UntweakedKeypair},
    taproot::{ControlBlock, LeafVersion, TaprootMerkleBranch},
    transaction::Version,
    Address, Amount, Block, BlockHash, Network, Psbt, ScriptBuf, SignedAmount, TapNodeHash,
    Transaction, TxOut, Txid, Work, XOnlyPublicKey,
};
use bitcoind_async_client::{
    corepc_types::{
        model::{
            Bip125Replaceable, GetAddressInfo, GetBlockchainInfo, GetMempoolInfo, GetRawMempool,
            GetRawMempoolVerbose, GetRawTransaction, GetRawTransactionVerbose, GetTransaction,
            GetTxOut, ListTransactions, ListUnspent, ListUnspentItem, MempoolAcceptance,
            PsbtBumpFee, SignRawTransaction, SubmitPackage, SubmitPackageTxResult,
            TestMempoolAccept, WalletCreateFundedPsbt, WalletProcessPsbt,
        },
        v29::{ImportDescriptors, ImportDescriptorsResult},
    },
    traits::{Broadcaster, Reader, Signer, Wallet},
    types::{
        CreateRawTransactionArguments, CreateRawTransactionInput, CreateRawTransactionOutput,
        ImportDescriptorInput, ListUnspentQueryOptions, PreviousTransactionOutput,
        PsbtBumpFeeOptions, SighashType, WalletCreateFundedPsbtOptions,
    },
    ClientResult,
};
use musig2::secp256k1::SECP256K1;
use rand::{rngs::OsRng, RngCore};
use strata_csm_types::L1Payload;
use strata_l1_envelope_fmt::builder::build_envelope_script;
use strata_l1_txfmt::{ParseConfig, TagDataRef};

use crate::writer::builder::{build_reveal_transaction, EnvelopeError};

/// A test implementation of a Bitcoin client.
#[derive(Debug, Clone)]
pub struct TestBitcoinClient {
    /// Confirmations of a given transaction.
    pub confs: u64,
    /// Which height a transaction was included in.
    pub included_height: u64,
}

impl TestBitcoinClient {
    pub fn new(confs: u64) -> Self {
        Self {
            confs,
            // Use arbitrary value, make configurable as necessary
            included_height: 100,
        }
    }
}

const TEST_BLOCKSTR: &str = "000000207d862a78fcb02ab24ebd154a20b9992af6d2f0c94d3a67b94ad5a0009d577e70769f3ff7452ea5dd469d7d99f200d083d020f1585e4bd9f52e9d66b23891a9c6c4ea5e66ffff7f200000000001020000000001010000000000000000000000000000000000000000000000000000000000000000ffffffff04025f0200ffffffff02205fa01200000000160014d7340213b180c97bd55fedd7312b7e17389cf9bf0000000000000000266a24aa21a9ede2f61c3f71d1defd3fa999dfa36953755c690689799962b48bebd836974e8cf90120000000000000000000000000000000000000000000000000000000000000000000000000";

/// A test transaction.
///
/// # Note
///
/// Taken from
/// [`rust-bitcoin` test](https://docs.rs/bitcoin/0.32.1/src/bitcoin/blockdata/transaction.rs.html#1638).
pub const SOME_TX: &str = "0100000001a15d57094aa7a21a28cb20b59aab8fc7d1149a3bdbcddba9c622e4f5f6a99ece010000006c493046022100f93bb0e7d8db7bd46e40132d1f8242026e045f03a0efe71bbb8e3f475e970d790221009337cd7f1f929f00cc6ff01f03729b069a7c21b59b1736ddfee5db5946c5da8c0121033b9b137ee87d5a812d6f506efdd37f0affa7ffc310711c06c7f3e097c9447c52ffffffff0100e1f505000000001976a9140389035a9225b3839e2bbf32d826a1e222031fd888ac00000000";

impl Reader for TestBitcoinClient {
    async fn estimate_smart_fee(&self, _conf_target: u16) -> ClientResult<u64> {
        Ok(3)
    }

    async fn get_block_header(&self, _hash: &BlockHash) -> ClientResult<Header> {
        let block: Block = deserialize(&hex::decode(TEST_BLOCKSTR).unwrap()).unwrap();
        Ok(block.header)
    }

    async fn get_block(&self, _hash: &BlockHash) -> ClientResult<Block> {
        let block: Block = deserialize(&hex::decode(TEST_BLOCKSTR).unwrap()).unwrap();
        Ok(block)
    }

    async fn get_block_height(&self, _hash: &BlockHash) -> ClientResult<u64> {
        Ok(100)
    }

    async fn get_block_header_at(&self, _height: u64) -> ClientResult<Header> {
        let block: Block = deserialize(&hex::decode(TEST_BLOCKSTR).unwrap()).unwrap();
        Ok(block.header)
    }

    async fn get_block_at(&self, _height: u64) -> ClientResult<Block> {
        let block: Block = deserialize(&hex::decode(TEST_BLOCKSTR).unwrap()).unwrap();
        Ok(block)
    }

    async fn get_block_count(&self) -> ClientResult<u64> {
        Ok(100)
    }

    // get_block_hash returns the block hash of the block at the given height
    async fn get_block_hash(&self, _h: u64) -> ClientResult<BlockHash> {
        let block: Block = deserialize(&hex::decode(TEST_BLOCKSTR).unwrap()).unwrap();
        Ok(block.block_hash())
    }

    async fn get_blockchain_info(&self) -> ClientResult<GetBlockchainInfo> {
        Ok(GetBlockchainInfo {
            chain: Network::Regtest,
            blocks: 100,
            headers: 100,
            best_block_hash: BlockHash::all_zeros(),
            difficulty: 1.0,
            median_time: 10 * 60,
            verification_progress: 1.0,
            initial_block_download: false,
            chain_work: Work::from_be_bytes([0; 32]),
            size_on_disk: 1_000_000,
            pruned: false,
            prune_height: None,
            automatic_pruning: None,
            prune_target_size: None,
            bits: None,
            target: None,
            time: None,
            signet_challenge: None,
            warnings: vec![],
            softforks: BTreeMap::new(),
        })
    }

    async fn get_current_timestamp(&self) -> ClientResult<u32> {
        Ok(1_000)
    }

    async fn get_raw_mempool(&self) -> ClientResult<GetRawMempool> {
        Ok(GetRawMempool(vec![]))
    }

    /// Gets a raw transaction by its [`Txid`].
    async fn get_raw_transaction_verbosity_zero(
        &self,
        _txid: &Txid,
    ) -> ClientResult<GetRawTransaction> {
        let some_tx: Transaction = consensus::encode::deserialize_hex(SOME_TX).unwrap();
        Ok(GetRawTransaction(some_tx))
    }

    async fn get_raw_transaction_verbosity_one(
        &self,
        _txid: &Txid,
    ) -> ClientResult<GetRawTransactionVerbose> {
        let some_tx: Transaction = consensus::encode::deserialize_hex(SOME_TX).unwrap();
        let block_hash = BlockHash::all_zeros();
        Ok(GetRawTransactionVerbose {
            transaction: some_tx,
            block_hash: Some(block_hash),
            confirmations: Some(1),
            transaction_time: None,
            block_time: None,
            in_active_chain: Some(true),
        })
    }

    async fn get_tx_out(
        &self,
        _txid: &Txid,
        _vout: u32,
        _include_mempool: bool,
    ) -> ClientResult<GetTxOut> {
        Ok(GetTxOut {
            best_block: BlockHash::all_zeros(),
            confirmations: 1,
            tx_out: TxOut {
                value: Amount::from_btc(1.0).unwrap(),
                // Taken from mainnet txid
                // e35e3357cac58a56dab78fa3c544f52f091561ff84428da28bdc5c49fc4c5ffc
                script_pubkey: ScriptBuf::from_hex("001478a93a5b649de9deabd9494ae9bc41f3c9c13837")
                    .unwrap(),
            },
            coinbase: false,
            address: Some(
                "bc1q0z5n5kmynh5aa27ef99wn0zp70yuzwph68my2c"
                    .parse::<Address<_>>()
                    .unwrap(),
            ),
        })
    }

    async fn network(&self) -> ClientResult<Network> {
        Ok(Network::Regtest)
    }

    async fn get_raw_mempool_verbose(&self) -> ClientResult<GetRawMempoolVerbose> {
        Ok(GetRawMempoolVerbose(BTreeMap::new()))
    }

    async fn get_mempool_info(&self) -> ClientResult<GetMempoolInfo> {
        Ok(GetMempoolInfo {
            size: 0,
            bytes: 0,
            usage: 0,
            max_mempool: 0,
            mempool_min_fee: None,
            loaded: None,
            total_fee: None,
            min_relay_tx_fee: None,
            incremental_relay_fee: None,
            unbroadcast_count: Some(0),
            full_rbf: None,
        })
    }
}

impl Broadcaster for TestBitcoinClient {
    // send_raw_transaction sends a raw transaction to the network
    async fn send_raw_transaction(&self, _tx: &Transaction) -> ClientResult<Txid> {
        Ok(Txid::from_slice(&[1u8; 32]).unwrap())
    }
    async fn test_mempool_accept(&self, _tx: &Transaction) -> ClientResult<TestMempoolAccept> {
        let some_tx: Transaction = consensus::encode::deserialize_hex(SOME_TX).unwrap();
        Ok(TestMempoolAccept {
            results: vec![MempoolAcceptance {
                txid: some_tx.compute_txid(),
                allowed: true,
                reject_reason: None,
                vsize: None,
                fees: None,
                wtxid: None,
                reject_details: None,
            }],
        })
    }

    async fn submit_package(&self, _txs: &[Transaction]) -> ClientResult<SubmitPackage> {
        let some_tx: Transaction = consensus::encode::deserialize_hex(SOME_TX).unwrap();
        let wtxid = some_tx.compute_wtxid();
        let vsize = some_tx.vsize();
        let tx_results = BTreeMap::from([(
            wtxid,
            SubmitPackageTxResult {
                txid: some_tx.compute_txid(),
                other_wtxid: None,
                vsize: Some(vsize as u32),
                fees: None,
                error: None,
            },
        )]);
        Ok(SubmitPackage {
            package_msg: "success".to_string(),
            tx_results,
            replaced_transactions: vec![],
        })
    }
}

impl Wallet for TestBitcoinClient {
    async fn get_new_address(&self) -> ClientResult<Address> {
        // taken from https://bitcoin.stackexchange.com/q/91222
        let addr = "bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw"
            .parse::<Address<_>>()
            .unwrap()
            .assume_checked();
        Ok(addr)
    }

    async fn get_transaction(&self, txid: &Txid) -> ClientResult<GetTransaction> {
        let some_tx = consensus::encode::deserialize_hex(SOME_TX).unwrap();
        Ok(GetTransaction {
            tx: some_tx,
            amount: SignedAmount::from_btc(100.0).unwrap(),
            confirmations: self.confs as i64,
            generated: None,
            trusted: None,
            txid: *txid,
            wtxid: None,
            replaced_by_txid: None,
            replaces_txid: None,
            comment: None,
            to: None,
            time: 0,
            bip125_replaceable: Bip125Replaceable::No,
            details: vec![],
            fee: None,
            block_hash: Some(BlockHash::all_zeros()),
            block_height: Some(self.included_height as u32),
            block_index: Some(0),
            block_time: Some(1_000),
            wallet_conflicts: vec![],
            mempool_conflicts: Some(vec![]),
            time_received: 0,
            parent_descriptors: Some(vec![]),
            decoded: None,
            last_processed_block: None,
        })
    }

    async fn list_unspent(
        &self,
        _min_conf: Option<u32>,
        _max_conf: Option<u32>,
        _addresses: Option<&[Address]>,
        _include_unsafe: Option<bool>,
        _query_options: Option<ListUnspentQueryOptions>,
    ) -> ClientResult<ListUnspent> {
        // plenty of sats
        Ok(ListUnspent(vec![ListUnspentItem {
            txid: Txid::from_slice(&[1; 32]).unwrap(),
            vout: 0,
            address: "bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw"
                .parse::<Address<_>>()
                .unwrap(),
            label: "test".to_string(),
            script_pubkey: ScriptBuf::from_hex("001478a93a5b649de9deabd9494ae9bc41f3c9c13837")
                .unwrap(),
            amount: SignedAmount::from_btc(100.0).unwrap(),
            confirmations: self.confs as u32,
            spendable: true,
            solvable: true,
            safe: true,
            redeem_script: None,
            descriptor: None,
            parent_descriptors: None,
        }]))
    }

    async fn list_transactions(&self, _count: Option<usize>) -> ClientResult<ListTransactions> {
        Ok(ListTransactions(vec![]))
    }

    async fn list_wallets(&self) -> ClientResult<Vec<String>> {
        Ok(vec![])
    }

    async fn create_raw_transaction(
        &self,
        _raw_tx: CreateRawTransactionArguments,
    ) -> ClientResult<Transaction> {
        let some_tx: Transaction = consensus::encode::deserialize_hex(SOME_TX).unwrap();
        Ok(some_tx)
    }

    async fn wallet_create_funded_psbt(
        &self,
        _inputs: &[CreateRawTransactionInput],
        _outputs: &[CreateRawTransactionOutput],
        _locktime: Option<u32>,
        _options: Option<WalletCreateFundedPsbtOptions>,
        _bip32_derivs: Option<bool>,
    ) -> ClientResult<WalletCreateFundedPsbt> {
        Ok(WalletCreateFundedPsbt {
            // taken from https://docs.rs/bitcoin/0.32.8/src/bitcoin/psbt/mod.rs.html#1365
            psbt: Psbt::from_str("70736274ff01003302000000010000000000000000000000000000000000000000000000000000000000000000ffffffff00ffffffff000000000000420204bb0d5d0cca36e7b9c80f63bc04c1240babb83bcd2803ef7ac8b6e2af594291daec281e856c98d210c5ab14dfd5828761f8ee7d5f45ca21ad3e4c4b41b747a3a047304402204f67e2afb76142d44fae58a2495d33a3419daa26cd0db8d04f3452b63289ac0f022010762a9fb67e94cc5cad9026f6dc99ff7f070f4278d30fbc7d0c869dd38c7fe70100").unwrap(),
            fee: SignedAmount::from_btc(0.001).unwrap(),
            change_position: 0,
        })
    }

    async fn get_address_info(&self, address: &Address) -> ClientResult<GetAddressInfo> {
        Ok(GetAddressInfo {
            address: address.clone().into_unchecked(),
            script_pubkey: ScriptBuf::new(),
            is_mine: true,
            is_watch_only: false,
            solvable: None,
            descriptor: None,
            parent_descriptor: None,
            is_script: None,
            is_change: None,
            is_witness: true,
            sigs_required: None,
            is_compressed: None,
            label: None,
            timestamp: None,
            hd_key_path: None,
            hd_seed_id: None,
            hd_master_fingerprint: None,
            labels: vec![],
            witness_version: None,
            witness_program: None,
            script: None,
            hex: None,
            pubkeys: None,
            pubkey: None,
            embedded: None,
        })
    }
}

impl Signer for TestBitcoinClient {
    async fn sign_raw_transaction_with_wallet(
        &self,
        _tx: &Transaction,
        _prev_outputs: Option<Vec<PreviousTransactionOutput>>,
    ) -> ClientResult<SignRawTransaction> {
        let signed_tx: Transaction = consensus::encode::deserialize_hex(SOME_TX).unwrap();
        Ok(SignRawTransaction {
            tx: signed_tx,
            complete: true,
            errors: vec![],
        })
    }
    async fn get_xpriv(&self) -> ClientResult<Option<Xpriv>> {
        // taken from https://docs.rs/bitcoin/0.32.2/src/bitcoin/bip32.rs.html#1090
        // DO NOT USE THIS BY ANY MEANS IN PRODUCTION WITH REAL FUNDS
        let xpriv = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi".parse::<Xpriv>().unwrap();
        Ok(Some(xpriv))
    }

    async fn import_descriptors(
        &self,
        _descriptors: Vec<ImportDescriptorInput>,
        _wallet_name: String,
    ) -> ClientResult<ImportDescriptors> {
        Ok(ImportDescriptors(vec![ImportDescriptorsResult {
            success: true,
            warnings: None,
            error: None,
        }]))
    }

    async fn wallet_process_psbt(
        &self,
        _psbt: &str,
        _sign: Option<bool>,
        _sighashtype: Option<SighashType>,
        _bip32_derivs: Option<bool>,
    ) -> ClientResult<WalletProcessPsbt> {
        Ok(WalletProcessPsbt {
            // taken from https://docs.rs/bitcoin/0.32.8/src/bitcoin/psbt/mod.rs.html#1365
            psbt: Psbt::from_str("70736274ff01003302000000010000000000000000000000000000000000000000000000000000000000000000ffffffff00ffffffff000000000000420204bb0d5d0cca36e7b9c80f63bc04c1240babb83bcd2803ef7ac8b6e2af594291daec281e856c98d210c5ab14dfd5828761f8ee7d5f45ca21ad3e4c4b41b747a3a047304402204f67e2afb76142d44fae58a2495d33a3419daa26cd0db8d04f3452b63289ac0f022010762a9fb67e94cc5cad9026f6dc99ff7f070f4278d30fbc7d0c869dd38c7fe70100").unwrap(),
            complete: true,
            hex: None,
        })
    }

    async fn psbt_bump_fee(
        &self,
        _txid: &Txid,
        _options: Option<PsbtBumpFeeOptions>,
    ) -> ClientResult<PsbtBumpFee> {
        Ok(PsbtBumpFee {
            // taken from https://docs.rs/bitcoin/0.32.8/src/bitcoin/psbt/mod.rs.html#1365
            psbt: Psbt::from_str("70736274ff01003302000000010000000000000000000000000000000000000000000000000000000000000000ffffffff00ffffffff000000000000420204bb0d5d0cca36e7b9c80f63bc04c1240babb83bcd2803ef7ac8b6e2af594291daec281e856c98d210c5ab14dfd5828761f8ee7d5f45ca21ad3e4c4b41b747a3a047304402204f67e2afb76142d44fae58a2495d33a3419daa26cd0db8d04f3452b63289ac0f022010762a9fb67e94cc5cad9026f6dc99ff7f070f4278d30fbc7d0c869dd38c7fe70100").unwrap(),
            original_fee: Amount::from_btc(0.001).unwrap(),
            fee: Amount::from_btc(0.01).unwrap(),
            errors: vec![],
        })
    }
}

pub fn build_reveal_transaction_test(
    input_transaction: Transaction,
    recipient: Address,
    output_value: u64,
    fee_rate: u64,
    reveal_script: &ScriptBuf,
    tag_script: ScriptBuf,
    control_block: &ControlBlock,
) -> Result<Transaction, EnvelopeError> {
    build_reveal_transaction(
        input_transaction,
        recipient,
        output_value,
        fee_rate,
        reveal_script,
        tag_script,
        control_block,
    )
}

// Create an envelope transaction. The focus here is to create a tapscript, rather than a
// completely valid control block. Includes `n_envelopes` envelopes in the tapscript.
pub fn create_checkpoint_envelope_tx(address: &str, l1_payload: L1Payload) -> Transaction {
    let address = Address::from_str(address)
        .unwrap()
        .require_network(Network::Regtest)
        .unwrap();
    let inp_tx = Transaction {
        version: Version(1),
        lock_time: LockTime::Blocks(Height::from_consensus(1).unwrap()),
        input: vec![],
        output: vec![TxOut {
            value: Amount::from_sat(100000000),
            script_pubkey: address.script_pubkey(),
        }],
    };
    // Concatenate all payload chunks into a single payload
    let concatenated_payload: Vec<u8> = l1_payload.data().iter().flatten().copied().collect();
    let reveal_script = build_envelope_script(&concatenated_payload).unwrap();

    let td = TagDataRef::new(1, 1, &[]).unwrap();
    let tag_script = ParseConfig::new((*b"ALPN").into())
        .encode_script_buf(&td)
        .unwrap();

    // Create controlblock
    let mut rand_bytes = [0; 32];
    OsRng.fill_bytes(&mut rand_bytes);
    let key_pair = UntweakedKeypair::from_seckey_slice(SECP256K1, &rand_bytes).unwrap();
    let public_key = XOnlyPublicKey::from_keypair(&key_pair).0;
    let nodehash: [TapNodeHash; 0] = [];
    let cb = ControlBlock {
        leaf_version: LeafVersion::TapScript,
        output_key_parity: Parity::Even,
        internal_key: public_key,
        merkle_branch: TaprootMerkleBranch::from(nodehash),
    };

    // Create transaction using control block
    let mut tx =
        build_reveal_transaction_test(inp_tx, address, 100, 10, &reveal_script, tag_script, &cb)
            .unwrap();
    tx.input[0].witness.push([1; 3]);
    tx.input[0].witness.push(reveal_script);
    tx.input[0].witness.push(cb.serialize());
    tx
}

#[cfg(test)]
pub(crate) mod test_context {
    use std::sync::Arc;

    use bitcoin::{Address, Network};
    use strata_config::btcio::WriterConfig;
    use strata_status::StatusChannel;
    use strata_test_utils::ArbitraryGenerator;
    use strata_test_utils_l2::gen_params;

    use crate::{test_utils::TestBitcoinClient, writer::context::WriterContext};

    pub(crate) fn get_writer_context() -> Arc<WriterContext<TestBitcoinClient>> {
        let client = Arc::new(TestBitcoinClient::new(1));
        let addr = "bcrt1q6u6qyya3sryhh42lahtnz2m7zuufe7dlt8j0j5"
            .parse::<Address<_>>()
            .unwrap()
            .require_network(Network::Regtest)
            .unwrap();
        let cfg = Arc::new(WriterConfig::default());
        let status_channel = StatusChannel::new(
            ArbitraryGenerator::new().generate(),
            ArbitraryGenerator::new().generate(),
            ArbitraryGenerator::new().generate(),
            None,
            None,
        );
        let params = Arc::new(gen_params());
        let ctx = WriterContext::new(params, cfg, addr, client, status_channel);
        Arc::new(ctx)
    }
}

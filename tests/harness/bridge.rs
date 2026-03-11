//! Bridge subprotocol test utilities
//!
//! Provides helpers for testing bridge subprotocol transactions and their
//! interaction with the checkpoint subprotocol (deposit tracking).
//!
//! # Example
//!
//! ```ignore
//! use harness::bridge::{create_test_bridge_setup, BridgeExt};
//! use harness::test_harness::AsmTestHarnessBuilder;
//!
//! let (bridge_params, ctx) = create_test_bridge_setup();
//! let harness = AsmTestHarnessBuilder::default()
//!     .with_bridge_params(bridge_params)
//!     .build()
//!     .await?;
//! harness.submit_deposit(&ctx, 0).await?;
//! ```

use std::{future::Future, slice};

use bitcoin::{
    absolute::LockTime,
    hashes::Hash,
    key::UntweakedKeypair,
    script,
    secp256k1::{Secp256k1, XOnlyPublicKey, SECP256K1},
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
    transaction::Version,
    Address, Amount, BlockHash, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Witness,
};
use bitcoin_bosd::Descriptor;
use rand::RngCore;
use strata_asm_common::{AnchorState, Subprotocol};
use strata_asm_params::{BridgeV1InitConfig, CheckpointInitConfig};
use strata_asm_proto_bridge_v1::{BridgeV1State, BridgeV1Subproto};
use strata_asm_proto_checkpoint::{state::CheckpointState, subprotocol::CheckpointSubprotocol};
use strata_asm_txs_bridge_v1::{
    deposit::DepositTxHeaderAux,
    deposit_request::{
        build_deposit_request_spend_info, create_deposit_request_locking_script, DrtHeaderAux,
    },
    test_utils::create_test_operators,
};
use strata_btc_types::BitcoinAmount;
use strata_checkpoint_types_ssz::CheckpointTip;
use strata_codec::{encode_to_vec, VarVec};
use strata_codec_utils::CodecSsz;
use strata_crypto::{test_utils::schnorr::Musig2Tweak, EvenPublicKey, EvenSecretKey};
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_l1_txfmt::{ParseConfig, TagData};
use strata_ol_chain_types_new::{OLLog, SimpleWithdrawalIntentLogData};
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;
use strata_test_utils::ArbitraryGenerator;
use strata_test_utils_btcio::{address::derive_musig2_p2tr_address, signing::sign_musig2_keypath};
use strata_test_utils_l2::CheckpointTestHarness;

use super::test_harness::AsmTestHarness;

// ============================================================================
// Bridge Context
// ============================================================================

/// Context for building and submitting bridge deposit transactions.
///
/// Holds operator keys and bridge configuration needed to construct valid
/// deposit request (DRT) and deposit (DT) transaction pairs.
#[derive(Debug)]
pub struct BridgeContext {
    operator_privkeys: Vec<EvenSecretKey>,
    operator_pubkeys: Vec<EvenPublicKey>,
    denomination: BitcoinAmount,
    recovery_delay: u16,
    recovery_pk: [u8; 32],
    destination: Vec<u8>,
}

impl BridgeContext {
    /// Returns the operator private keys.
    pub fn operator_privkeys(&self) -> &[EvenSecretKey] {
        &self.operator_privkeys
    }

    /// Returns the operator public keys.
    pub fn operator_pubkeys(&self) -> &[EvenPublicKey] {
        &self.operator_pubkeys
    }

    /// Returns the bridge denomination.
    pub fn denomination(&self) -> BitcoinAmount {
        self.denomination
    }
}

// ============================================================================
// Bridge Extension Trait
// ============================================================================

/// Extension trait for bridge subprotocol operations on the test harness.
pub trait BridgeExt {
    /// Get bridge V1 subprotocol state.
    fn bridge_state(&self) -> anyhow::Result<BridgeV1State>;

    /// Get checkpoint (ID=1) subprotocol state.
    fn checkpoint_new_state(&self) -> anyhow::Result<CheckpointState>;

    /// Submit a deposit: build DRT + DT, submit both, mine, and wait.
    fn submit_deposit(
        &self,
        ctx: &BridgeContext,
        deposit_idx: u32,
    ) -> impl Future<Output = anyhow::Result<BlockHash>>;

    /// Submit a checkpoint with withdrawal intents.
    ///
    /// Builds a valid checkpoint payload containing OL logs with withdrawal intents
    /// for the given amounts, signs it, and submits it as an SPS-50 envelope transaction.
    fn submit_checkpoint_with_withdrawals(
        &self,
        checkpoint_harness: &mut CheckpointTestHarness,
        withdrawal_amounts: &[u64],
    ) -> impl Future<Output = anyhow::Result<BlockHash>>;
}

impl BridgeExt for AsmTestHarness {
    fn bridge_state(&self) -> anyhow::Result<BridgeV1State> {
        let (_, asm_state) = self
            .get_latest_asm_state()?
            .ok_or_else(|| anyhow::anyhow!("No ASM state available"))?;
        extract_bridge_state(asm_state.state())
    }

    fn checkpoint_new_state(&self) -> anyhow::Result<CheckpointState> {
        let (_, asm_state) = self
            .get_latest_asm_state()?
            .ok_or_else(|| anyhow::anyhow!("No ASM state available"))?;
        extract_checkpoint_new_state(asm_state.state())
    }

    async fn submit_deposit(
        &self,
        ctx: &BridgeContext,
        deposit_idx: u32,
    ) -> anyhow::Result<BlockHash> {
        // 1. Build and submit the DRT (Deposit Request Transaction)
        let drt_tx = self.build_drt_tx(ctx).await?;
        let drt_txid = self.submit_transaction(&drt_tx).await?;

        // Mine the DRT so it's confirmed and fetchable as aux data
        self.mine_block(None).await?;

        // 2. Build the DT (Deposit Transaction) referencing the DRT
        let drt_outpoint = OutPoint::new(drt_txid, 1); // DRT output index 1
        let drt_output = drt_tx.output[1].clone();
        let dt_tx = self.build_dt_tx(ctx, deposit_idx, drt_outpoint, &drt_output)?;

        // 3. Submit and mine the DT
        let hash = self.submit_and_mine_tx(&dt_tx).await?;

        Ok(hash)
    }

    async fn submit_checkpoint_with_withdrawals(
        &self,
        checkpoint_harness: &mut CheckpointTestHarness,
        withdrawal_amounts: &[u64],
    ) -> anyhow::Result<BlockHash> {
        let genesis_l1_height = self.genesis_height as u32;

        // 1. Get manifest hashes from the live ASM MMR
        let mmr_leaves = self.get_mmr_leaves();

        // 2. Build the new checkpoint tip covering all processed L1 blocks
        let new_l1_height = genesis_l1_height + mmr_leaves.len() as u32;
        let new_epoch = checkpoint_harness.verified_tip().epoch + 1;
        let new_ol_slot = checkpoint_harness.verified_tip().l2_commitment().slot() + 1;
        let new_ol_blkid: OLBlockId = ArbitraryGenerator::new().generate();
        let new_ol_commitment = OLBlockCommitment::new(new_ol_slot, new_ol_blkid);
        let new_tip = CheckpointTip::new(new_epoch, new_l1_height, new_ol_commitment);

        // 3. Build OL logs with withdrawal intents
        let ol_logs = build_withdrawal_ol_logs(withdrawal_amounts);

        // 4. Build checkpoint payload with custom OL logs and live manifest hashes
        let payload =
            checkpoint_harness.build_payload_with_tip_and_logs(new_tip, ol_logs, &mmr_leaves);

        // 5. Encode payload with CodecSsz and submit as envelope tx with sequencer keypair (SPS-51)
        let codec_payload = CodecSsz::new(payload);
        let payload_bytes = encode_to_vec(&codec_payload).expect("codec encoding should not fail");
        let checkpoint_tag = TagData::new(1, 1, vec![]).expect("valid checkpoint tag");
        let secp = Secp256k1::new();
        let sequencer_keypair =
            UntweakedKeypair::from_seckey_slice(&secp, checkpoint_harness.sequencer_secret_key())?;
        let tx = self
            .build_envelope_tx_with_keypair(checkpoint_tag, payload_bytes, &sequencer_keypair)
            .await?;
        let block_hash = self.submit_and_mine_tx(&tx).await?;

        // 7. Update harness verified tip
        checkpoint_harness.update_verified_tip(new_tip);

        Ok(block_hash)
    }
}

// ============================================================================
// State Extraction
// ============================================================================

/// Extract bridge V1 subprotocol state from AnchorState.
pub fn extract_bridge_state(anchor_state: &AnchorState) -> anyhow::Result<BridgeV1State> {
    let section = anchor_state
        .find_section(BridgeV1Subproto::ID)
        .ok_or_else(|| anyhow::anyhow!("Bridge V1 section not found"))?;
    let bridge_state = section.try_to_state::<BridgeV1Subproto>()?;
    Ok(bridge_state)
}

/// Extract checkpoint (ID=1) subprotocol state from AnchorState.
pub fn extract_checkpoint_new_state(anchor_state: &AnchorState) -> anyhow::Result<CheckpointState> {
    let section = anchor_state
        .find_section(CheckpointSubprotocol::ID)
        .ok_or_else(|| anyhow::anyhow!("Checkpoint (ID=1) section not found"))?;
    let checkpoint_state = section.try_to_state::<CheckpointSubprotocol>()?;
    Ok(checkpoint_state)
}

// ============================================================================
// Transaction Building
// ============================================================================

impl AsmTestHarness {
    /// Build a Deposit Request Transaction (DRT).
    ///
    /// The DRT has:
    /// - Output 0: OP_RETURN with SPS-50 tag (subproto=2, tx_type=0, aux=recovery_pk+destination)
    /// - Output 1: P2TR deposit request output locked to operator multisig + recovery tapscript
    async fn build_drt_tx(&self, ctx: &BridgeContext) -> anyhow::Result<Transaction> {
        let fee = Self::DEFAULT_FEE;

        // Build the DRT header aux data
        let destination = VarVec::from_vec(ctx.destination.clone())
            .ok_or_else(|| anyhow::anyhow!("invalid destination length"))?;
        let drt_aux = DrtHeaderAux::new(ctx.recovery_pk, destination)?;

        // Build the SPS-50 OP_RETURN tag
        let tag_data = drt_aux.build_tag_data();
        let parse_config = ParseConfig::new(self.asm_params.magic);
        let op_return_script = parse_config.encode_script_buf(&tag_data.as_ref())?;

        // Build the P2TR deposit request locking script
        let (_, internal_key) = derive_musig2_p2tr_address(ctx.operator_privkeys())?;
        let drt_locking_script = create_deposit_request_locking_script(
            &ctx.recovery_pk,
            internal_key,
            ctx.recovery_delay,
        );

        // Fund a trivially-spendable taproot address to use as input.
        // The DRT output[1] carries `denomination + fee` so the DT can pay a
        // mining fee while keeping its deposit output exactly at denomination.
        let deposit_amount: Amount = ctx.denomination.into();
        let drt_output_amount = deposit_amount + fee;
        let (funding_txid, funding_vout, funding_script, funding_spend_info) = self
            .create_trivial_funding_utxo(drt_output_amount + fee + Amount::from_sat(1000))
            .await?;

        // Mine a block to confirm the funding UTXO so Bitcoin Core can
        // resolve its value when computing fees for the DRT.
        self.mine_block(None).await?;

        // Build the reveal script for the funding input
        let trivial_script = build_trivial_script();
        let control_block = funding_spend_info
            .control_block(&(trivial_script.clone(), LeafVersion::TapScript))
            .ok_or_else(|| anyhow::anyhow!("Failed to create control block"))?;

        let mut witness = Witness::new();
        witness.push(trivial_script.as_bytes());
        witness.push(control_block.serialize());

        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(funding_txid, funding_vout),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness,
            }],
            output: vec![
                // Output 0: SPS-50 OP_RETURN tag
                TxOut {
                    value: Amount::ZERO,
                    script_pubkey: op_return_script,
                },
                // Output 1: P2TR deposit request output (denomination + fee for DT)
                TxOut {
                    value: drt_output_amount,
                    script_pubkey: drt_locking_script,
                },
            ],
        };

        // Suppress unused variable warning
        let _ = funding_script;

        Ok(tx)
    }

    /// Build a Deposit Transaction (DT) that spends a DRT output.
    ///
    /// The DT has:
    /// - Input 0: Spends the DRT output[1] via MuSig2 key-path spend
    /// - Output 0: OP_RETURN with SPS-50 tag (subproto=2, tx_type=1, aux=deposit_idx)
    /// - Output 1: P2TR deposit output locked to operator multisig (key-path only)
    fn build_dt_tx(
        &self,
        ctx: &BridgeContext,
        deposit_idx: u32,
        drt_outpoint: OutPoint,
        drt_output: &TxOut,
    ) -> anyhow::Result<Transaction> {
        // Build the SPS-50 OP_RETURN tag for deposit
        let dt_aux = DepositTxHeaderAux::new(deposit_idx);
        let tag_data = dt_aux.build_tag_data();
        let parse_config = ParseConfig::new(self.asm_params.magic);
        let op_return_script = parse_config.encode_script_buf(&tag_data.as_ref())?;

        // Build the P2TR deposit output (key-path only with operator multisig)
        let (_, internal_key) = derive_musig2_p2tr_address(ctx.operator_privkeys())?;
        let deposit_script = ScriptBuf::new_p2tr(SECP256K1, internal_key, None);

        // Build unsigned transaction
        let deposit_amount: Amount = ctx.denomination.into();
        let mut tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: drt_outpoint,
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![
                // Output 0: SPS-50 OP_RETURN tag
                TxOut {
                    value: Amount::ZERO,
                    script_pubkey: op_return_script,
                },
                // Output 1: P2TR deposit output
                TxOut {
                    value: deposit_amount,
                    script_pubkey: deposit_script,
                },
            ],
        };

        // Sign the DRT input with MuSig2 key-path spend.
        // The DRT P2TR has a merkle root from the recovery tapscript, so we need the
        // TaprootScript tweak.
        let spend_info =
            build_deposit_request_spend_info(&ctx.recovery_pk, internal_key, ctx.recovery_delay);
        let tweak = match spend_info.merkle_root() {
            Some(root) => Musig2Tweak::TaprootScript(root.to_raw_hash().to_byte_array()),
            None => Musig2Tweak::TaprootKeySpend,
        };

        let sig = sign_musig2_keypath(
            &tx,
            ctx.operator_privkeys(),
            slice::from_ref(drt_output),
            0,
            tweak,
        )?;
        tx.input[0].witness.push(sig.as_ref());

        Ok(tx)
    }

    /// Create a trivially-spendable taproot UTXO for funding test transactions.
    ///
    /// Returns (txid, vout, script_pubkey, taproot_spend_info).
    async fn create_trivial_funding_utxo(
        &self,
        amount: Amount,
    ) -> anyhow::Result<(bitcoin::Txid, u32, ScriptBuf, TaprootSpendInfo)> {
        let secp = Secp256k1::new();
        let mut rng = rand::thread_rng();
        let mut key_bytes = [0u8; 32];
        rng.fill_bytes(&mut key_bytes);
        let keypair = UntweakedKeypair::from_seckey_slice(&secp, &key_bytes)?;
        let (internal_key, _) = XOnlyPublicKey::from_keypair(&keypair);

        let trivial_script = build_trivial_script();
        let spend_info = TaprootBuilder::new()
            .add_leaf(0, trivial_script)?
            .finalize(&secp, internal_key)
            .map_err(|_| anyhow::anyhow!("Failed to finalize taproot spend info"))?;

        let address = Address::p2tr(
            &secp,
            internal_key,
            spend_info.merkle_root(),
            Network::Regtest,
        );

        let (txid, vout) = self.create_funding_utxo(&address, amount).await?;
        Ok((txid, vout, address.script_pubkey(), spend_info))
    }
}

/// Build a trivially-spendable tapscript (OP_TRUE).
fn build_trivial_script() -> ScriptBuf {
    script::Builder::new().push_int(1).into_script()
}

// ============================================================================
// Test Setup
// ============================================================================

/// Builds OL logs encoding withdrawal intents for the given amounts.
///
/// Each withdrawal intent is a [`SimpleWithdrawalIntentLogData`] encoded via `strata_codec`
/// and wrapped in an [`OLLog`] from the bridge gateway account.
fn build_withdrawal_ol_logs(withdrawal_amounts: &[u64]) -> Vec<OLLog> {
    withdrawal_amounts
        .iter()
        .map(|&amt| {
            // P2WPKH descriptor: type tag 0x00 + 20-byte hash = 21 bytes
            let hash160 = [0x14; 20];
            let descriptor = Descriptor::new_p2wpkh(&hash160);
            let dest = descriptor.to_bytes();

            let withdrawal_data = SimpleWithdrawalIntentLogData::new(amt, dest, 0)
                .expect("withdrawal intent creation should not fail");
            let encoded_payload = strata_codec::encode_to_vec(&withdrawal_data)
                .expect("withdrawal intent encoding should not fail");

            OLLog::new(BRIDGE_GATEWAY_ACCT_SERIAL, encoded_payload)
        })
        .collect()
}

/// Creates matching checkpoint config and test harness for integration tests.
///
/// Generates signing keys and returns a [`CheckpointInitConfig`] (for the harness builder)
/// and a [`CheckpointTestHarness`] (for building checkpoint payloads).
pub fn create_test_checkpoint_setup(
    genesis_l1_height: u32,
) -> (CheckpointInitConfig, CheckpointTestHarness) {
    let genesis_ol_blkid: OLBlockId = ArbitraryGenerator::new().generate();
    let harness = CheckpointTestHarness::new_with_genesis(genesis_l1_height, genesis_ol_blkid);

    let config = CheckpointInitConfig {
        sequencer_predicate: harness.sequencer_predicate(),
        checkpoint_predicate: harness.checkpoint_predicate(),
        genesis_l1_height,
        genesis_ol_blkid,
    };

    (config, harness)
}

/// Creates matching bridge config and context for integration tests.
///
/// Generates operator keys and returns a [`BridgeV1InitConfig`] (for the harness builder)
/// and a [`BridgeContext`] (for submitting deposits).
pub fn create_test_bridge_setup(num_operators: usize) -> (BridgeV1InitConfig, BridgeContext) {
    let (privkeys, pubkeys) = create_test_operators(num_operators);

    let denomination = BitcoinAmount::from_sat(1_000_000);
    let recovery_delay = 1008;
    let operator_fee = BitcoinAmount::from_sat(100_000);

    let config = BridgeV1InitConfig {
        operators: pubkeys.clone(),
        denomination,
        assignment_duration: 144,
        operator_fee,
        recovery_delay,
    };

    // Use a deterministic recovery key for test reproducibility
    let recovery_pk = [42u8; 32];
    let destination = vec![0xDE, 0xAD, 0xBE, 0xEF]; // dummy destination

    let ctx = BridgeContext {
        operator_privkeys: privkeys,
        operator_pubkeys: pubkeys,
        denomination,
        recovery_delay,
        recovery_pk,
        destination,
    };

    (config, ctx)
}

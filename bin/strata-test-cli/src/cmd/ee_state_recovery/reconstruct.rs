//! Reconstructs EE state from an ordered manifest of known Bitcoin DA transactions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{write, File},
    io::{BufWriter, Write},
    path::PathBuf,
};

use alloy_consensus::{
    constants::{EMPTY_OMMER_ROOT_HASH, EMPTY_WITHDRAWALS},
    EMPTY_ROOT_HASH,
};
use alloy_primitives::{hex, keccak256, Address, Bytes, B256, U256};
use alloy_rlp::Encodable;
use alpen_chainspec::chain_value_parser;
use alpen_ee_da_types::{extract_da_chunks, reassemble_da_blob, EvmHeaderSummary};
use alpen_reth_statediff::{
    apply_batch_state_diff_to_ethereum_state, ethereum_state_from_chain_spec, EthereumStateExt,
};
use anyhow::{anyhow, bail, Context, Result};
use bitcoin::{consensus::deserialize, Transaction, Txid};
use rsp_mpt::EthereumState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use strata_acct_types::{BitcoinAmount, Hash, SubjectId};
use strata_ee_acct_types::{EeAccountState, PendingFinclEntry, PendingInputEntry};
use strata_ee_chain_types::SubjectDepositData;
use strata_snark_acct_runtime::IInnerState;

/// Reconstruct and verify EE state from known L1 DA transactions.
#[derive(Debug, PartialEq)]
pub(super) struct ReconstructConfig {
    /// ordered manifest containing known DA transactions
    pub manifest: PathBuf,

    /// chain specification name or path
    pub chain: String,

    /// execution tip block ID reconstructed from OL DA
    pub last_exec_blkid: B256,

    /// proof-backed SnarkAccount inner-state commitment from OL
    pub expected_inner_state_root: B256,

    /// pending input queue reconstructed from OL inbox messages and accepted updates
    pub pending_inputs: Vec<PendingInputEntry>,

    /// pending forced-inclusion queue reconstructed from accepted updates
    pub pending_fincls: Vec<PendingFinclEntry>,

    /// da update sequence referenced by the latest OL-accepted update manifest
    pub target_update_seq_no: u64,

    /// output path for the Reth JSONL state dump
    pub state_dump: PathBuf,

    /// output path for verified reconstruction metadata
    pub metadata: PathBuf,

    /// output path for the reconstructed anchor header
    pub anchor_header: PathBuf,

    /// prints DA account changes and reconstructed values around state-changing batches
    pub trace_diffs: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ReplayManifest {
    pub(super) batches: Vec<BatchManifest>,
    pub(super) raw_transactions: BTreeMap<Txid, String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct BatchManifest {
    pub(super) update_seq_no: u64,
    pub(super) commit_txid: Txid,
    pub(super) reveal_txids: Vec<Txid>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ReconstructedStateMetadata {
    pub(super) update_seq_no: u64,
    pub(super) last_exec_blkid: B256,
    pub(super) last_exec_state_root: B256,
    pub(super) inner_state_root: B256,
    pub(super) pending_inputs: Vec<PendingInputMetadata>,
    pub(super) pending_fincls: Vec<PendingFinclMetadata>,
    pub(super) account_state_verified: bool,
    pub(super) evm_header: HeaderMetadata,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum PendingInputMetadata {
    Deposit { destination: B256, value_sats: u64 },
}

impl From<PendingInputEntry> for PendingInputMetadata {
    fn from(value: PendingInputEntry) -> Self {
        match value {
            PendingInputEntry::Deposit(deposit) => Self::Deposit {
                destination: B256::from(<[u8; 32]>::from(deposit.dest())),
                value_sats: deposit.value().to_sat(),
            },
        }
    }
}

impl From<PendingInputMetadata> for PendingInputEntry {
    fn from(value: PendingInputMetadata) -> Self {
        match value {
            PendingInputMetadata::Deposit {
                destination,
                value_sats,
            } => Self::Deposit(SubjectDepositData::new(
                SubjectId::new(destination.0),
                BitcoinAmount::from_sat(value_sats),
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct PendingFinclMetadata {
    epoch: u32,
    raw_tx_hash: B256,
}

impl From<PendingFinclEntry> for PendingFinclMetadata {
    fn from(value: PendingFinclEntry) -> Self {
        let (epoch, raw_tx_hash) = value.into_parts();
        Self {
            epoch,
            raw_tx_hash: B256::from(raw_tx_hash.0),
        }
    }
}

impl From<PendingFinclMetadata> for PendingFinclEntry {
    fn from(value: PendingFinclMetadata) -> Self {
        Self::new(value.epoch, protocol_hash(value.raw_tx_hash))
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct HeaderMetadata {
    pub(super) block_num: u64,
    pub(super) timestamp: u64,
    pub(super) base_fee: u64,
    pub(super) gas_used: u64,
    pub(super) gas_limit: u64,
}

impl From<EvmHeaderSummary> for HeaderMetadata {
    fn from(value: EvmHeaderSummary) -> Self {
        Self {
            block_num: value.block_num,
            timestamp: value.timestamp,
            base_fee: value.base_fee,
            gas_used: value.gas_used,
            gas_limit: value.gas_limit,
        }
    }
}

fn decode_transaction(manifest: &ReplayManifest, txid: &Txid) -> Result<Transaction> {
    let raw = manifest
        .raw_transactions
        .get(txid)
        .with_context(|| format!("manifest has no raw transaction for {txid}"))?;
    let bytes = hex::decode(raw.trim_start_matches("0x"))
        .with_context(|| format!("invalid transaction hex for {txid}"))?;
    let tx: Transaction = deserialize(&bytes)
        .with_context(|| format!("failed to decode Bitcoin transaction {txid}"))?;
    if tx.compute_txid() != *txid {
        bail!("raw transaction does not match manifest txid {txid}");
    }
    Ok(tx)
}

fn protocol_hash(value: B256) -> Hash {
    value.0.into()
}

fn write_state_dump(
    path: &PathBuf,
    state_root: B256,
    state: &EthereumState,
    addresses: &BTreeSet<Address>,
    slots: &BTreeMap<Address, BTreeSet<U256>>,
    bytecodes: &BTreeMap<B256, Bytes>,
) -> Result<()> {
    let file = File::create(path)
        .with_context(|| format!("failed to create state dump {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, &json!({ "root": state_root }))?;
    writer.write_all(b"\n")?;

    for address in addresses {
        let Some(account) = state.get_account_snapshot(*address)? else {
            continue;
        };

        let mut account_storage = BTreeMap::new();
        for slot in slots.get(address).into_iter().flatten() {
            let value = state.get_storage_slot(*address, *slot)?;
            if !value.is_zero() {
                account_storage.insert(
                    B256::from(slot.to_be_bytes::<32>()),
                    B256::from(value.to_be_bytes::<32>()),
                );
            }
        }

        let code = bytecodes.get(&account.code_hash).cloned();
        let mut record = Map::new();
        record.insert("address".into(), serde_json::to_value(address)?);
        record.insert("balance".into(), serde_json::to_value(account.balance)?);
        record.insert("nonce".into(), Value::from(account.nonce));
        if let Some(code) = code {
            record.insert("code".into(), serde_json::to_value(code)?);
        }
        if !account_storage.is_empty() {
            record.insert("storage".into(), serde_json::to_value(account_storage)?);
        }
        serde_json::to_writer(&mut writer, &record)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

pub(super) fn reconstruct(args: ReconstructConfig) -> Result<()> {
    let manifest: ReplayManifest = serde_json::from_reader(
        File::open(&args.manifest)
            .with_context(|| format!("failed to open {}", args.manifest.display()))?,
    )?;
    let chain_spec = chain_value_parser(&args.chain).map_err(|err| anyhow!("{err:#}"))?;
    let mut state =
        ethereum_state_from_chain_spec(&args.chain).map_err(|err| anyhow!("{err:#}"))?;
    let mut addresses = BTreeSet::new();
    let mut slots: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
    let mut bytecodes = BTreeMap::new();

    for (address, account) in &chain_spec.genesis.alloc {
        addresses.insert(*address);
        if let Some(storage) = &account.storage {
            slots
                .entry(*address)
                .or_default()
                .extend(storage.keys().map(|slot| U256::from_be_bytes(slot.0)));
        }
        if let Some(code) = &account.code {
            bytecodes.insert(keccak256(code), code.clone());
        }
    }

    for (expected_seq_no, batch) in (0u64..).zip(&manifest.batches) {
        if batch.update_seq_no != expected_seq_no {
            bail!(
                "non-contiguous DA sequence: expected {expected_seq_no}, got {}",
                batch.update_seq_no
            );
        }
        let mut transactions = Vec::with_capacity(batch.reveal_txids.len() + 1);
        transactions.push(decode_transaction(&manifest, &batch.commit_txid)?);
        for txid in &batch.reveal_txids {
            transactions.push(decode_transaction(&manifest, txid)?);
        }

        let chunks = extract_da_chunks(transactions.iter())?;
        let blob = reassemble_da_blob(&chunks)?;
        if blob.update_seq_no != batch.update_seq_no {
            bail!(
                "blob sequence {} disagrees with manifest sequence {}",
                blob.update_seq_no,
                batch.update_seq_no
            );
        }

        addresses.extend(blob.state_diff.accounts.keys().copied());
        for (address, storage_diff) in &blob.state_diff.storage {
            addresses.insert(*address);
            slots
                .entry(*address)
                .or_default()
                .extend(storage_diff.iter().map(|(slot, _)| *slot));
        }
        bytecodes.extend(blob.state_diff.deployed_bytecodes.clone());
        if args.trace_diffs && !blob.state_diff.is_empty() {
            println!(
                "seq={} block={} diff_accounts={} diff_storage={} bytecodes={}",
                blob.update_seq_no,
                blob.evm_header.block_num,
                blob.state_diff.accounts.len(),
                blob.state_diff.storage.len(),
                blob.state_diff.deployed_bytecodes.len()
            );
            for (address, change) in &blob.state_diff.accounts {
                println!(
                    "  address={address} before={:?} change={change:?}",
                    state.get_account_snapshot(*address)?
                );
            }
        }
        apply_batch_state_diff_to_ethereum_state(&mut state, &blob.state_diff)?;
        if args.trace_diffs && !blob.state_diff.is_empty() {
            for address in blob.state_diff.accounts.keys() {
                println!(
                    "  address={address} after={:?}",
                    state.get_account_snapshot(*address)?
                );
            }
        }

        let state_root = B256::from(state.state_root_buf32().0);
        println!(
            "seq={} block={} state_root={state_root}",
            blob.update_seq_no, blob.evm_header.block_num
        );
        if blob.update_seq_no == args.target_update_seq_no {
            let account_state = EeAccountState::new(
                protocol_hash(args.last_exec_blkid),
                protocol_hash(state_root),
                args.pending_inputs.clone(),
                args.pending_fincls.clone(),
            );
            let inner_state_root = B256::from(account_state.compute_state_root().0);
            if inner_state_root != args.expected_inner_state_root {
                bail!(
                    "reconstructed EeAccountState root {} does not match proof-backed OL \
                     inner_state_root {}",
                    inner_state_root,
                    args.expected_inner_state_root
                );
            }
            write_state_dump(
                &args.state_dump,
                state_root,
                &state,
                &addresses,
                &slots,
                &bytecodes,
            )?;
            let metadata = ReconstructedStateMetadata {
                update_seq_no: blob.update_seq_no,
                last_exec_blkid: args.last_exec_blkid,
                last_exec_state_root: state_root,
                inner_state_root,
                pending_inputs: account_state
                    .pending_inputs()
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect(),
                pending_fincls: account_state
                    .pending_fincls()
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect(),
                account_state_verified: true,
                evm_header: blob.evm_header.into(),
            };
            serde_json::to_writer_pretty(File::create(&args.metadata)?, &metadata)?;
            let header = reth_primitives::Header {
                ommers_hash: EMPTY_OMMER_ROOT_HASH,
                number: blob.evm_header.block_num,
                timestamp: blob.evm_header.timestamp,
                base_fee_per_gas: Some(blob.evm_header.base_fee),
                gas_used: blob.evm_header.gas_used,
                gas_limit: blob.evm_header.gas_limit,
                state_root,
                transactions_root: EMPTY_ROOT_HASH,
                receipts_root: EMPTY_ROOT_HASH,
                withdrawals_root: Some(EMPTY_WITHDRAWALS),
                blob_gas_used: Some(0),
                excess_blob_gas: Some(0),
                parent_beacon_block_root: Some(B256::ZERO),
                // EIP-7685 empty requests hash (`sha256("")`).
                requests_hash: Some(B256::new([
                    0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99,
                    0x6f, 0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95,
                    0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
                ])),
                ..Default::default()
            };
            let mut encoded_header = Vec::new();
            header.encode(&mut encoded_header);
            write(&args.anchor_header, encoded_header).with_context(|| {
                format!(
                    "failed to write anchor header {}",
                    args.anchor_header.display()
                )
            })?;
            println!(
                "verified reconstructed EeAccountState at seq={} block={} inner_state_root={}",
                blob.update_seq_no, blob.evm_header.block_num, inner_state_root
            );
            return Ok(());
        }
    }

    bail!(
        "OL-accepted DA sequence {} was not present in the manifest",
        args.target_update_seq_no
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_deposit_metadata_round_trips() {
        let input = PendingInputEntry::Deposit(SubjectDepositData::new(
            SubjectId::new([3; 32]),
            BitcoinAmount::from_sat(42),
        ));

        let metadata = PendingInputMetadata::from(input);
        let encoded = serde_json::to_vec(&metadata).unwrap();
        let decoded: PendingInputMetadata = serde_json::from_slice(&encoded).unwrap();
        let PendingInputEntry::Deposit(deposit) = PendingInputEntry::from(decoded);

        assert_eq!(deposit.dest(), SubjectId::new([3; 32]));
        assert_eq!(deposit.value(), BitcoinAmount::from_sat(42));
    }

    #[test]
    fn pending_fincl_metadata_round_trips() {
        let input = PendingFinclEntry::new(7, Hash::from([4; 32]));

        let metadata = PendingFinclMetadata::from(input);
        let encoded = serde_json::to_vec(&metadata).unwrap();
        let decoded: PendingFinclMetadata = serde_json::from_slice(&encoded).unwrap();
        let (epoch, raw_tx_hash) = PendingFinclEntry::from(decoded).into_parts();

        assert_eq!(epoch, 7);
        assert_eq!(raw_tx_hash, Hash::from([4; 32]));
    }
}

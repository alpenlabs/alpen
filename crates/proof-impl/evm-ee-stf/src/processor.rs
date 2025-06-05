// This code is modified from the original implementation of Zeth.
//
// Reference: https://github.com/risc0/zeth
//
// Copyright 2023 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either strata or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
use std::{mem, mem::take};

use alloy_consensus::constants::{GWEI_TO_WEI, MAXIMUM_EXTRA_DATA_SIZE};
use alloy_eips::{eip1559::BaseFeeParams, eip2718::Encodable2718};
use alloy_primitives::map::DefaultHashBuilder;
use alloy_rlp::BufMut;
use alloy_rpc_types_eth::TransactionTrait;
use alloy_trie::root::ordered_trie_root_with_encoder;
use alpen_reth_evm::set_evm_handles;
use anyhow::anyhow;
use reth_primitives::{Header, Receipt, Transaction, TransactionSigned};
use reth_primitives_traits::{constants::MINIMUM_GAS_LIMIT, SignedTransaction};
use revm::{
    db::{AccountState, InMemoryDB},
    interpreter::Host,
    primitives::{SpecId, TransactTo, TxEnv},
    Database, DatabaseCommit, Evm,
};
use revm_primitives::{
    alloy_primitives::{Address, Bloom, TxKind as TransactionKind, U256},
    Account,
};

use crate::{
    mpt::{keccak, RlpBytes, StateAccount},
    EvmBlockStfInput,
};

/// The divisor for the gas limit bound.
pub const GAS_LIMIT_DIVISOR: u64 = 1024;

#[derive(Clone)]
pub struct EvmConfig {
    pub chain_id: u64,
    pub spec_id: SpecId,
}

/// A processor that executes EVM transactions.
#[derive(Clone)]
pub struct EvmProcessor<D> {
    /// An input containing all necessary data to execute the block.
    pub input: EvmBlockStfInput,

    /// A database to store all state changes.
    pub db: Option<D>,

    /// The header to be finalized.
    pub header: Option<Header>,

    /// Evm config
    pub evm_config: EvmConfig,
}

impl<D> EvmProcessor<D> {
    /// Validate the header standalone.
    ///
    /// Reference: <https://github.com/paradigmxyz/reth/blob/main/crates/consensus/common/src/validation.rs#L14>
    pub fn validate_header_standalone(&self) {
        let header = self.header.as_ref().unwrap();

        // Gas used needs to be less then gas limit. Gas used is going to be check after execution.
        if header.gas_used > header.gas_limit {
            panic!("Gas used exceeds gas limit");
        }
    }

    /// Validates the integrity and consistency of a block header in relation to it's parent.
    ///
    /// Reference: <https://github.com/paradigmxyz/reth/blob/main/crates/primitives/src/header.rs#L800>
    pub fn validate_against_parent(&self) {
        let parent_header = &self.input.parent_header;
        let header = self.header.as_ref().unwrap();

        // Parent number is consistent.
        if parent_header.number + 1 != header.number {
            panic!("Parent number is inconsistent with header number");
        }

        // Parent hash is consistent.
        if parent_header.hash_slow() != header.parent_hash {
            panic!("Parent hash is inconsistent with header parent hash");
        }

        // Timestamp in past check.
        if parent_header.timestamp > header.timestamp {
            panic!("Timestamp is in the future");
        }
    }

    /// Checks the gas limit for consistency between parent and self headers.
    ///
    /// Reference: <https://github.com/paradigmxyz/reth/blob/main/crates/primitives/src/header.rs#L738>
    pub fn validate_gas_limit(&self) {
        let parent_header = &self.input.parent_header;
        let header = self.header.as_ref().unwrap();
        let parent_gas_limit = parent_header.gas_limit;

        // Check for an increase in gas limit beyond the allowed threshold.
        if header.gas_limit > parent_gas_limit {
            if header.gas_limit - parent_gas_limit >= parent_gas_limit / GAS_LIMIT_DIVISOR {
                panic!("Gas limit invalid increase");
            }
        }
        // Check for a decrease in gas limit beyond the allowed threshold.
        else if parent_gas_limit - header.gas_limit >= parent_gas_limit / GAS_LIMIT_DIVISOR {
            panic!("Gas limit invalid decrease");
        }
        // Check if the self gas limit is below the minimum required limit.
        else if parent_gas_limit < MINIMUM_GAS_LIMIT {
            panic!("Gas limit below minimum");
        }
    }

    /// Validates the header's extradata according to the beacon consensus rules.
    ///
    /// Reference: <https://github.com/paradigmxyz/reth/blob/main/crates/consensus/beacon-core/src/lib.rs#L118>
    pub fn validate_header_extradata(&self) {
        let header = self.header.as_ref().unwrap();
        if header.extra_data.len() > MAXIMUM_EXTRA_DATA_SIZE {
            panic!("Extra data too large");
        }
    }
}

impl<D: Database + DatabaseCommit + Clone> EvmProcessor<D>
where
    <D as Database>::Error: core::fmt::Debug,
{
    /// Validate input values against the parent header and initialize the current header's
    /// computed fields.
    pub fn initialize(&mut self) {
        let params = BaseFeeParams::ethereum();
        let base_fee = self.input.parent_header.next_block_base_fee(params);
        let header = Header {
            parent_hash: self.input.parent_header.hash_slow(),
            number: self.input.parent_header.number.checked_add(1).unwrap(),
            base_fee_per_gas: base_fee,
            beneficiary: self.input.beneficiary,
            gas_limit: self.input.gas_limit,
            timestamp: self.input.timestamp,
            mix_hash: self.input.mix_hash,
            extra_data: self.input.extra_data.clone(),
            ..Default::default()
        };
        self.header = Some(header);
        self.validate_against_parent();
        self.validate_header_extradata();
    }

    /// Processes each transaction and collect receipts and storage changes.
    pub fn execute(
        &mut self,
    ) -> (
        Vec<reth_primitives::TransactionSigned>,
        Vec<reth_primitives::Receipt>,
    ) {
        let gwei_to_wei: U256 = U256::from(GWEI_TO_WEI);
        let mut evm = Evm::builder()
            .with_spec_id(self.evm_config.spec_id)
            .modify_cfg_env(|cfg_env| {
                cfg_env.chain_id = self.evm_config.chain_id;
            })
            .modify_block_env(|blk_env| {
                blk_env.number = self.header.as_mut().unwrap().number.try_into().unwrap();
                blk_env.coinbase = self.input.beneficiary;
                blk_env.timestamp = U256::from(self.header.as_mut().unwrap().timestamp);
                blk_env.difficulty = U256::ZERO;
                blk_env.prevrandao = Some(self.header.as_mut().unwrap().mix_hash);
                blk_env.basefee =
                    U256::from(self.header.as_mut().unwrap().base_fee_per_gas.unwrap());
                blk_env.gas_limit = U256::from(self.header.as_mut().unwrap().gas_limit);
            })
            .with_db(self.db.take().unwrap())
            .append_handler_register(set_evm_handles)
            .build();

        let mut logs_bloom = Bloom::default();
        let mut cumulative_gas_used = U256::ZERO;
        let mut receipts = Vec::new();
        let mut executed_txs = Vec::new();

        for (tx_no, tx) in self.input.transactions.iter().enumerate() {
            // Recover the sender from the transaction signature.
            let tx_from = tx.recover_signer_unchecked().unwrap();

            // Validate tx gas.
            let block_available_gas = U256::from(self.input.gas_limit) - cumulative_gas_used;
            if block_available_gas < U256::from(tx.transaction.gas_limit()) {
                panic!("Error at transaction {}: gas exceeds block limit", tx_no);
            }

            // Setup EVM from tx.
            fill_eth_tx_env(&mut evm.context.env_mut().tx, &tx.transaction, tx_from);
            // Execute transaction.
            let res = evm
                .transact()
                .map_err(|e| {
                    println!("Error at transaction {}: {:?}", tx_no, e);
                    e
                })
                .unwrap();

            // Update cumulative gas used.
            let gas_used = res.result.gas_used().try_into().unwrap();
            cumulative_gas_used = cumulative_gas_used.checked_add(gas_used).unwrap();

            // Create receipt.
            let receipt = Receipt {
                tx_type: tx.transaction.tx_type(),
                success: res.result.is_success(),
                cumulative_gas_used: cumulative_gas_used.try_into().unwrap(),
                logs: res.result.logs().to_vec(),
            };

            executed_txs.push(tx.clone());
            // Update logs bloom.
            logs_bloom.accrue_bloom(&receipt.bloom_slow());
            receipts.push(receipt);

            // Commit state changes.
            evm.context.evm.db.commit(res.state);
        }

        // Process consensus layer withdrawals.
        for withdrawal in self.input.withdrawals.iter() {
            // Convert withdrawal amount (in gwei) to wei.
            let amount_wei = gwei_to_wei
                .checked_mul(withdrawal.amount.try_into().unwrap())
                .unwrap();

            increase_account_balance(&mut evm.context.evm.db, withdrawal.address, amount_wei)
                .unwrap();
        }

        // Compute header roots and fill out other header fields.
        let h = self.header.as_mut().expect("Header not initialized");
        let txs_signed = take(&mut self.input.transactions)
            .into_iter()
            .collect::<Vec<TransactionSigned>>();
        h.transactions_root = ordered_trie_root_with_encoder(&txs_signed, |tx, buf| {
            tx.eip2718_encode(&tx.signature, buf);
        });
        h.receipts_root = ordered_trie_root_with_encoder(&receipts, |receipt, buf| {
            receipt.with_bloom_ref().encode_2718(buf);
        });
        h.withdrawals_root = Some(ordered_trie_root_with_encoder(
            &self.input.withdrawals,
            |withdrawal, buf| buf.put_slice(&withdrawal.to_rlp()),
        ));
        h.logs_bloom = logs_bloom;
        h.gas_used = cumulative_gas_used.try_into().unwrap();

        self.db = Some(evm.context.evm.db.clone());

        (executed_txs, receipts)
    }
}

impl EvmProcessor<InMemoryDB> {
    /// Process all state changes and finalize the header's state root.
    pub fn finalize(&mut self) {
        let db = self.db.take().expect("DB not initialized");

        let mut state_trie = mem::take(&mut self.input.pre_state_trie);
        for (address, account) in &db.accounts {
            // Ignore untouched accounts.
            if account.account_state == AccountState::None {
                continue;
            }
            let state_trie_index = keccak(address);

            // Remove from state trie if it has been deleted.
            if account.account_state == AccountState::NotExisting {
                state_trie.delete(&state_trie_index).unwrap();
                continue;
            }

            let mut state_account = StateAccount {
                nonce: account.info.nonce,
                balance: account.info.balance,
                storage_root: Default::default(),
                code_hash: account.info.code_hash,
            };

            // Skip insert if the account is empty.
            if state_account.is_account_empty() {
                continue;
            }

            // Update storage root for account.
            let state_storage = &account.storage;
            state_account.storage_root = {
                let (storage_trie, _) = self.input.pre_state_storage.get_mut(address).unwrap();
                // If the account has been cleared, clear the storage trie.
                if account.account_state == AccountState::StorageCleared {
                    storage_trie.clear();
                }

                // Apply all storage changes to the storage trie.
                for (key, value) in state_storage {
                    let storage_trie_index = keccak(key.to_be_bytes::<32>());
                    if value == &U256::ZERO {
                        storage_trie.delete(&storage_trie_index).unwrap();
                    } else {
                        storage_trie
                            .insert_rlp(&storage_trie_index, *value)
                            .unwrap();
                    }
                }

                storage_trie.hash()
            };

            state_trie
                .insert_rlp(&state_trie_index, state_account)
                .expect("MPT is corrupted");
        }

        // Update state trie root in header.
        let header = self.header.as_mut().expect("Header not initialized");
        header.state_root = state_trie.hash();
    }
}

fn fill_eth_tx_env(tx_env: &mut TxEnv, essence: &Transaction, caller: Address) {
    match essence {
        Transaction::Legacy(tx) => {
            tx_env.caller = caller;
            tx_env.gas_limit = tx.gas_limit;
            tx_env.gas_price = U256::from(tx.gas_price);
            tx_env.gas_priority_fee = None;
            tx_env.transact_to = if let TransactionKind::Call(to_addr) = tx.to {
                TransactTo::Call(to_addr)
            } else {
                TransactTo::Create
            };
            tx_env.value = tx.value;
            tx_env.data = tx.input.clone();
            tx_env.chain_id = tx.chain_id;
            tx_env.nonce = Some(tx.nonce);
            tx_env.access_list.clear();
        }
        Transaction::Eip2930(tx) => {
            tx_env.caller = caller;
            tx_env.gas_limit = tx.gas_limit;
            tx_env.gas_price = U256::from(tx.gas_price);
            tx_env.gas_priority_fee = None;
            tx_env.transact_to = if let TransactionKind::Call(to_addr) = tx.to {
                TransactTo::Call(to_addr)
            } else {
                TransactTo::Create
            };
            tx_env.value = tx.value;
            tx_env.data = tx.input.clone();
            tx_env.chain_id = Some(tx.chain_id);
            tx_env.nonce = Some(tx.nonce);
            tx_env.access_list = tx.access_list.to_vec();
        }
        Transaction::Eip1559(tx) => {
            tx_env.caller = caller;
            tx_env.gas_limit = tx.gas_limit;
            tx_env.gas_price = U256::from(tx.max_fee_per_gas);
            tx_env.gas_priority_fee = Some(U256::from(tx.max_priority_fee_per_gas));
            tx_env.transact_to = if let TransactionKind::Call(to_addr) = tx.to {
                TransactTo::Call(to_addr)
            } else {
                TransactTo::Create
            };
            tx_env.value = tx.value;
            tx_env.data = tx.input.clone();
            tx_env.chain_id = Some(tx.chain_id);
            tx_env.nonce = Some(tx.nonce);
            tx_env.access_list = tx.access_list.to_vec();
        }
        Transaction::Eip4844(_) => todo!(),
        _ => todo!(),
    };
}

pub fn increase_account_balance<D>(
    db: &mut D,
    address: Address,
    amount_wei: U256,
) -> anyhow::Result<()>
where
    D: Database + DatabaseCommit,
    <D as Database>::Error: core::fmt::Debug,
{
    // Read account from database
    let mut account: Account = db
        .basic(address)
        .map_err(|db_err| {
            anyhow!(
                "Error increasing account balance for {}: {:?}",
                address,
                db_err
            )
        })?
        .unwrap_or_default()
        .into();
    // Credit withdrawal amount
    account.info.balance = account.info.balance.checked_add(amount_wei).unwrap();
    account.mark_touch();
    // Commit changes to database

    db.commit(
        std::collections::HashMap::<_, _, DefaultHashBuilder>::from_iter([(address, account)]),
    );

    Ok(())
}

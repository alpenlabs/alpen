use rand::{thread_rng, Rng};

use stf_runner::{
    account::{AccountId, MessageData, SnarkAccountMessageEntry, SnarkAccountUpdate, SnarkAccountUpdateData, SnarkAccountProofState, AccountUpdateOutputs, LedgerReferences, OutputTransfer, OutputMessage},
    block::{OLBlock, OLBlockHeader, OLBlockBody, SignedOLBlockHeader, Transaction, TransactionPayload, TransactionExtra, L1Update, AsmManifest, OLLog},
};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{DepositLog, constants::DEPOSIT_LOG_TYPE_ID};
use strata_primitives::buf::{Buf32, Buf64};

pub enum InvalidType {
    BadSlot,
    BadTimestamp,
    BadParent,
    ZeroBodyRoot,
}

pub fn generate_genesis_block() -> OLBlock {
    let header = OLBlockHeader::new(
        1000000000, // timestamp
        0,          // slot  
        0,          // epoch
        Buf32::zero(), // parent (genesis has no parent)
        random_hash(), // body_root
        random_hash(), // state_root
    );

    let signed_header = SignedOLBlockHeader::new(header, random_signature());
    let body = OLBlockBody::new(Vec::new(), None, None);

    OLBlock::new(signed_header, body)
}

pub fn generate_test_block(prev_header: &OLBlockHeader, with_txs: bool) -> OLBlock {
    let mut rng = thread_rng();
    
    let header = OLBlockHeader::new(
        prev_header.timestamp() + rng.gen_range(1000..10000),
        prev_header.slot() + 1,
        prev_header.epoch(),
        prev_header.compute_header_root(),
        random_hash(),
        random_hash(),
    );

    let signed_header = SignedOLBlockHeader::new(header, random_signature());

    let txs = if with_txs {
        // Generate transactions targeting different demo accounts
        let mut transactions = Vec::new();
        let account_count = 3; // Assume we have 3 demo accounts
        
        for i in 0..account_count {
            let target = Buf32::from([i as u8; 32]);
            transactions.push(generate_account_update_transaction(target, 0));
        }
        
        Some(transactions)
    } else {
        None
    };

    let body = OLBlockBody::new(Vec::new(), txs, None);
    OLBlock::new(signed_header, body)
}

pub fn generate_invalid_block(prev_header: &OLBlockHeader, error_type: InvalidType) -> OLBlock {
    let mut rng = thread_rng();
    
    let (timestamp, slot, parent_id, body_root) = match error_type {
        InvalidType::BadSlot => (
            prev_header.timestamp() + 1000,
            prev_header.slot() - 1, // Invalid: slot regression
            prev_header.compute_header_root(),
            random_hash(),
        ),
        InvalidType::BadTimestamp => (
            prev_header.timestamp() - 400000, // Invalid: too far back
            prev_header.slot() + 1,
            prev_header.compute_header_root(),
            random_hash(),
        ),
        InvalidType::BadParent => (
            prev_header.timestamp() + 1000,
            prev_header.slot() + 1,
            random_hash(), // Invalid: wrong parent
            random_hash(),
        ),
        InvalidType::ZeroBodyRoot => (
            prev_header.timestamp() + 1000,
            prev_header.slot() + 1,
            prev_header.compute_header_root(),
            Buf32::zero(), // Invalid: zero body root
        ),
    };

    let header = OLBlockHeader::new(
        timestamp,
        slot,
        prev_header.epoch(),
        parent_id,
        body_root,
        random_hash(),
    );

    let signed_header = SignedOLBlockHeader::new(header, random_signature());
    let body = OLBlockBody::new(Vec::new(), None, None);
    
    OLBlock::new(signed_header, body)
}

pub fn generate_deposit_block(prev_header: &OLBlockHeader, deposits: Vec<(u32, u64)>) -> OLBlock {
    let mut rng = thread_rng();
    
    let header = OLBlockHeader::new(
        prev_header.timestamp() + rng.gen_range(1000..10000),
        prev_header.slot() + 1,
        prev_header.epoch(),
        prev_header.compute_header_root(),
        random_hash(),
        random_hash(),
    );

    let signed_header = SignedOLBlockHeader::new(header, random_signature());

    // Create deposit logs
    let mut asm_logs = Vec::new();
    for (ee_id, amount) in deposits {
        let deposit = DepositLog {
            ee_id: ee_id as u64,
            amount,
            addr: random_address(),
        };
        
        // Create AsmLogEntry - this might need adjustment based on actual AsmLogEntry structure
        let log_entry = AsmLogEntry::new(DEPOSIT_LOG_TYPE_ID, deposit.encode().unwrap());
        asm_logs.push(log_entry);
    }

    let manifest = AsmManifest::new(random_hash(), asm_logs);
    let l1_update = L1Update::new(random_hash(), 100, vec![manifest]);

    let body = OLBlockBody::new(Vec::new(), None, Some(l1_update));
    OLBlock::new(signed_header, body)
}

/// Generate a SNARK account update transaction for a specific account
pub fn generate_account_update_transaction(target: AccountId, seq_no: u64) -> Transaction {
    let update = SnarkAccountUpdate {
        data: SnarkAccountUpdateData {
            new_state: SnarkAccountProofState {
                inner_state_root: random_hash(),
                next_input_idx: 1,
            },
            seq_no,
            processed_msgs: Vec::new(),
            ledger_refs: LedgerReferences {},
            outputs: AccountUpdateOutputs {
                output_transfers: Vec::new(),
                output_messages: Vec::new(),
            },
            extra_data: Vec::new(),
        },
        witness: vec![1, 2, 3, 4], // Dummy witness for demo
    };

    let payload = TransactionPayload::SnarkAccountUpdate { target, update };
    let extra = TransactionExtra::new(None, None);

    Transaction::new(1, payload, extra)
}

/// Generate a transaction that transfers value between accounts
pub fn generate_transfer_transaction(from: AccountId, to: AccountId, amount: u64, seq_no: u64) -> Transaction {
    let transfer = OutputTransfer {
        destination: to,
        transferred_value: amount,
    };

    let update = SnarkAccountUpdate {
        data: SnarkAccountUpdateData {
            new_state: SnarkAccountProofState {
                inner_state_root: random_hash(),
                next_input_idx: 0,
            },
            seq_no,
            processed_msgs: Vec::new(),
            ledger_refs: LedgerReferences {},
            outputs: AccountUpdateOutputs {
                output_transfers: vec![transfer],
                output_messages: Vec::new(),
            },
            extra_data: Vec::new(),
        },
        witness: vec![1, 2, 3, 4], // Dummy witness for demo
    };

    let payload = TransactionPayload::SnarkAccountUpdate { target: from, update };
    let extra = TransactionExtra::new(None, None);

    Transaction::new(1, payload, extra)
}

/// Generate a transaction that sends a message with value to another account
pub fn generate_message_transaction(from: AccountId, to: AccountId, amount: u64, payload: Vec<u8>, seq_no: u64) -> Transaction {
    let message = OutputMessage {
        destination: to,
        data: MessageData {
            transferred_value: amount,
            payload,
        },
    };

    let update = SnarkAccountUpdate {
        data: SnarkAccountUpdateData {
            new_state: SnarkAccountProofState {
                inner_state_root: random_hash(),
                next_input_idx: 0,
            },
            seq_no,
            processed_msgs: Vec::new(),
            ledger_refs: LedgerReferences {},
            outputs: AccountUpdateOutputs {
                output_transfers: Vec::new(),
                output_messages: vec![message],
            },
            extra_data: Vec::new(),
        },
        witness: vec![1, 2, 3, 4], // Dummy witness for demo
    };

    let payload = TransactionPayload::SnarkAccountUpdate { target: from, update };
    let extra = TransactionExtra::new(None, None);

    Transaction::new(1, payload, extra)
}

/// Generate multiple transactions for demonstration
pub fn generate_demo_transactions() -> Vec<Transaction> {
    let mut transactions = Vec::new();
    
    // Account IDs for demo (matching state.rs initialization)
    let account_0 = Buf32::from([0u8; 32]);
    let account_1 = Buf32::from([1u8; 32]);
    let account_2 = Buf32::from([2u8; 32]);
    
    // Account 0 transfers 100 to Account 1
    transactions.push(generate_transfer_transaction(account_0, account_1, 100, 0));
    
    // Account 1 sends message with 50 value to Account 2
    transactions.push(generate_message_transaction(account_1, account_2, 50, b"hello".to_vec(), 0));
    
    // Account 2 does a simple state update
    transactions.push(generate_account_update_transaction(account_2, 0));
    
    transactions
}

pub fn random_account_id() -> AccountId {
    let mut rng = thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    bytes[0] = rng.gen_range(0..5); // Keep to small range for demo
    Buf32::from(bytes)
}

fn random_hash() -> Buf32 {
    let mut rng = thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    Buf32::from(bytes)
}

fn random_signature() -> Buf64 {
    let mut rng = thread_rng();
    let mut bytes = [0u8; 64];
    rng.fill(&mut bytes);
    Buf64::from(bytes)
}

fn random_address() -> [u8; 20] {
    let mut rng = thread_rng();
    let mut addr = [0u8; 20];
    rng.fill(&mut addr);
    addr
}

pub fn parse_invalid_type(s: &str) -> Option<InvalidType> {
    match s {
        "bad_slot" => Some(InvalidType::BadSlot),
        "bad_timestamp" => Some(InvalidType::BadTimestamp),
        "bad_parent" => Some(InvalidType::BadParent),
        "zero_body_root" => Some(InvalidType::ZeroBodyRoot),
        _ => None,
    }
}
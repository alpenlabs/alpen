use stf_runner::{
    account::{
        AccountInnerState, AccountState, AccountUpdateOutputs, LedgerReferences, OutputTransfer,
        SnarkAccountProofState, SnarkAccountState, SnarkAccountUpdate, SnarkAccountUpdateData,
    },
    block::{
        AsmManifest, L1Update, OLBlock, OLBlockBody, OLBlockHeader, SignedOLBlockHeader,
        Transaction, TransactionExtra, TransactionPayload,
    },
    ledger::{InMemoryVectorLedger, LedgerProvider},
    state::{L1View, OLState},
    stf::process_block,
};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::DepositLog;
use strata_chaintsn::context::StateAccessor;
use strata_primitives::{
    block_credential::CredRule,
    buf::{Buf32, Buf64},
    l1::L1BlockCommitment,
    params::{GenesisL1View, OperatorConfig, ProofPublishMode, RollupParams},
    proof::RollupVerifyingKey,
};

// Minimal StateAccessor implementation for demo
pub struct SimpleStateAccessor {
    state: OLState,
}

impl SimpleStateAccessor {
    pub fn new(state: OLState) -> Self {
        Self { state }
    }
}

impl StateAccessor<OLState> for SimpleStateAccessor {
    fn state_untracked(&self) -> &OLState {
        &self.state
    }

    fn state_mut_untracked(&mut self) -> &mut OLState {
        &mut self.state
    }

    fn slot(&self) -> u64 {
        self.state.cur_slot()
    }

    fn set_slot(&mut self, slot: u64) {
        self.state.set_cur_slot(slot);
    }

    fn cur_epoch(&self) -> u64 {
        self.state.cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u64) {
        self.state.set_cur_epoch(epoch);
    }

    fn get_toplevel_state(&mut self) -> &OLState {
        &self.state
    }

    fn set_accounts_root(&mut self, root: Buf32) {
        self.state.set_accounts_root(root);
    }

    // Minimal implementations for required methods
    fn prev_block(&self) -> strata_primitives::l2::L2BlockCommitment {
        strata_primitives::l2::L2BlockCommitment::new(
            0,
            strata_primitives::buf::Buf32::zero().into(),
        )
    }

    fn set_prev_block(&mut self, _block: strata_primitives::l2::L2BlockCommitment) {}

    fn prev_epoch(&self) -> strata_primitives::epoch::EpochCommitment {
        strata_primitives::epoch::EpochCommitment::new(
            0,
            0,
            strata_primitives::buf::Buf32::zero().into(),
        )
    }

    fn set_prev_epoch(&mut self, _epoch: strata_primitives::epoch::EpochCommitment) {}

    fn finalized_epoch(&self) -> strata_primitives::epoch::EpochCommitment {
        strata_primitives::epoch::EpochCommitment::new(
            0,
            0,
            strata_primitives::buf::Buf32::zero().into(),
        )
    }

    fn set_finalized_epoch(&mut self, _epoch: strata_primitives::epoch::EpochCommitment) {}

    fn last_l1_block(&self) -> strata_primitives::prelude::L1BlockCommitment {
        strata_primitives::prelude::L1BlockCommitment::new(
            0,
            strata_primitives::buf::Buf32::zero().into(),
        )
    }

    fn epoch_finishing_flag(&self) -> bool {
        false
    }

    fn set_epoch_finishing_flag(&mut self, _flag: bool) {}
}

fn create_minimal_params() -> RollupParams {
    RollupParams {
        magic_bytes: "demo".as_bytes().try_into().unwrap_or([0u8; 4]).into(),
        block_time: 1000,
        da_tag: "demo-da".to_string(),
        checkpoint_tag: "demo-ckpt".to_string(),
        cred_rule: CredRule::Unchecked,
        genesis_l1_view: GenesisL1View {
            blk: L1BlockCommitment::new(0, Buf32::zero().into()),
            next_target: 0,
            epoch_start_timestamp: 0,
            last_11_timestamps: [0; 11],
        },
        operator_config: OperatorConfig::Static(vec![]),
        evm_genesis_block_hash: Buf32::zero(),
        evm_genesis_block_state_root: Buf32::zero(),
        l1_reorg_safe_depth: 4,
        target_l2_batch_size: 64,
        address_length: 20,
        deposit_amount: 1000000000,
        rollup_vk: RollupVerifyingKey::NativeVerifyingKey,
        dispatch_assignment_dur: 64,
        proof_publish_mode: ProofPublishMode::Timeout(5),
        max_deposits_in_block: 16,
        network: bitcoin::Network::Regtest,
    }
}

fn create_genesis_block() -> OLBlock {
    let body = OLBlockBody::new(Vec::new(), None, None);

    // Create header with dummy state root - STF will compute the correct one
    let header = OLBlockHeader::new(
        1000000000,           // timestamp
        0,                    // slot
        0,                    // epoch
        Buf32::zero(),        // parent (genesis has no parent)
        Buf32::from([1; 32]), // body_root (empty body)
        Buf32::zero(),        // dummy state root - STF will compute correct one
    );

    let signed_header = SignedOLBlockHeader::new(header, Buf64::from([0; 64]));
    OLBlock::new(signed_header, body)
}

fn create_test_block(prev_header: &OLBlockHeader) -> OLBlock {
    let body = OLBlockBody::new(Vec::new(), None, None);

    let header = OLBlockHeader::new(
        prev_header.timestamp() + 5000,
        prev_header.slot() + 1,
        prev_header.epoch(),
        prev_header.compute_header_root(),
        Buf32::from([1; 32]), // body_root (empty body)
        Buf32::zero(),        // dummy state root - STF will compute correct one
    );

    let signed_header = SignedOLBlockHeader::new(header, Buf64::from([1; 64]));

    OLBlock::new(signed_header, body)
}

fn create_invalid_block(prev_header: &OLBlockHeader) -> OLBlock {
    let body = OLBlockBody::new(Vec::new(), None, None);

    // Create block with slot regression (invalid)
    let header = OLBlockHeader::new(
        prev_header.timestamp() + 5000,
        prev_header.slot().saturating_sub(1), // Invalid: slot goes backwards
        prev_header.epoch(),
        prev_header.compute_header_root(),
        Buf32::from([1; 32]), // body_root (empty body)
        Buf32::zero(),        // dummy state root - STF will compute correct one
    );

    let signed_header = SignedOLBlockHeader::new(header, Buf64::from([2; 64]));

    OLBlock::new(signed_header, body)
}

fn create_deposit_block(prev_header: &OLBlockHeader, amount: u64) -> OLBlock {
    // Create deposit log for hardcoded account 0
    let deposit = DepositLog::new(
        0,                                // ee_id = 0 (maps to account serial 0)
        amount,                           // amount in sats
        b"bc1demo_deposit_addr".to_vec(), // dummy Bitcoin address
    );

    // Convert to AsmLogEntry
    let asm_log = AsmLogEntry::from_log(&deposit).expect("Failed to create AsmLogEntry");

    // Create manifest with the deposit log
    let manifest = AsmManifest::new(
        Buf32::from([0xaa; 32]), // dummy L1 block ID
        vec![asm_log],
    );

    // Create L1Update with the manifest
    let l1update = L1Update::new(
        Buf32::zero(), // inner_state_root (placeholder)
        1000,          // new_l1_height (dummy)
        vec![manifest],
    );

    // Create block body with L1Update
    let body = OLBlockBody::new(Vec::new(), None, Some(l1update));

    let header = OLBlockHeader::new(
        prev_header.timestamp() + 5000,
        prev_header.slot() + 1,
        prev_header.epoch(),
        prev_header.compute_header_root(),
        Buf32::from([1; 32]), // body_root
        Buf32::zero(),        // dummy state root - STF will compute correct one
    );

    let signed_header = SignedOLBlockHeader::new(header, Buf64::from([3; 64]));

    OLBlock::new(signed_header, body)
}

fn create_withdrawal_block(prev_header: &OLBlockHeader, amount: u64) -> OLBlock {
    // Create a SNARK account update transaction that represents a withdrawal
    // This includes an output transfer to simulate withdrawing funds
    let account_id = Buf32::from([0u8; 32]); // Target account 0

    // Create an output transfer to simulate withdrawal (sending to bridge/external account)
    let withdrawal_transfer = OutputTransfer {
        destination: Buf32::from([0xff; 32]), // Bridge/withdrawal account
        transferred_value: amount,
    };

    let snark_update = SnarkAccountUpdate {
        data: SnarkAccountUpdateData {
            new_state: SnarkAccountProofState {
                inner_state_root: Buf32::zero(), // Placeholder
                next_input_idx: 0,
            },
            seq_no: 0,
            processed_msgs: Vec::new(),
            ledger_refs: LedgerReferences {},
            outputs: AccountUpdateOutputs {
                output_transfers: vec![withdrawal_transfer], // This should deduct balance
                output_messages: Vec::new(),
            },
            extra_data: format!("withdraw_{}", amount).into_bytes(), // Withdrawal metadata
        },
        witness: vec![0u8; 64], // Dummy proof witness
    };

    let tx_payload = TransactionPayload::SnarkAccountUpdate {
        target: account_id,
        update: snark_update,
    };

    let tx_extra = TransactionExtra::new(None, None);

    let transaction = Transaction::new(1, tx_payload, tx_extra);

    // Create block body with the transaction
    let body = OLBlockBody::new(Vec::new(), Some(vec![transaction]), None);

    let header = OLBlockHeader::new(
        prev_header.timestamp() + 5000,
        prev_header.slot() + 1,
        prev_header.epoch(),
        prev_header.compute_header_root(),
        Buf32::from([1; 32]), // body_root
        Buf32::zero(),        // dummy state root - STF will compute correct one
    );

    let signed_header = SignedOLBlockHeader::new(header, Buf64::from([4; 64]));

    OLBlock::new(signed_header, body)
}

fn main() -> anyhow::Result<()> {
    println!("STF Runner Demo - Interactive CLI");
    println!("==================================");

    // Initialize state
    let genesis_state = OLState::new(Buf32::zero(), L1View::new(Buf32::zero(), 0), 0, 0);
    let mut state_accessor = SimpleStateAccessor::new(genesis_state);
    let mut ledger = InMemoryVectorLedger::new();
    let params = create_minimal_params();
    let mut last_header = OLBlockHeader::new(0, 0, 0, Buf32::zero(), Buf32::zero(), Buf32::zero());
    let mut processed_blocks: Vec<OLBlock> = Vec::new();
    let mut computed_state_roots: Vec<Buf32> = Vec::new();

    // Create demo accounts with zero balance - use deposits to add funds
    println!("\nInitializing demo accounts...");
    for i in 0..2 {
        let account_id = Buf32::from([i as u8; 32]);

        let snark_state = SnarkAccountState {
            update_vk: Buf32::zero(),
            proof_state: SnarkAccountProofState {
                inner_state_root: Buf32::zero(),
                next_input_idx: 0,
            },
            seq_no: 0,
            input: Vec::new(),
        };

        let account_state = AccountState {
            serial: i,
            ty: 1,
            balance: 0, // Start with zero balance - use deposits to add funds
            inner_state: AccountInnerState::Snark(snark_state),
        };

        ledger.create_account(i, account_id, account_state);
        let account_hex = hex::encode(account_id.as_bytes());
        println!("  Account {}: {} balance = 0 sats", i, account_hex);
    }

    // Create withdrawal/bridge account for handling withdrawals
    // NOTE: doesn't match with what's in the stf but this doesn't matter for the demo.
    println!("\nCreating bridge account for withdrawals...");
    let bridge_account_id = Buf32::from([0xff; 32]);

    let bridge_snark_state = SnarkAccountState {
        update_vk: Buf32::zero(),
        proof_state: SnarkAccountProofState {
            inner_state_root: Buf32::zero(),
            next_input_idx: 0,
        },
        seq_no: 0,
        input: Vec::new(),
    };

    let bridge_account_state = AccountState {
        serial: 99, // Special serial for bridge account
        ty: 1,
        balance: 0, // Bridge account starts with zero balance
        inner_state: AccountInnerState::Snark(bridge_snark_state),
    };

    ledger.create_account(99, bridge_account_id, bridge_account_state);
    let bridge_hex = hex::encode(bridge_account_id.as_bytes());
    println!("  Bridge Account: {} for withdrawals", bridge_hex);

    // Update state accessor with the correct accounts root after creating accounts
    let accounts_root = ledger.root().expect("Failed to compute accounts root");
    state_accessor.set_accounts_root(accounts_root);

    // Process genesis block automatically on startup
    println!("\nProcessing genesis block...");
    let genesis_block = create_genesis_block();
    let genesis_prev_header =
        OLBlockHeader::new(0, 0, 0, Buf32::zero(), Buf32::zero(), Buf32::zero());
    let state_clone = state_accessor.get_toplevel_state().clone();

    match process_block(
        &state_clone,
        &genesis_prev_header,
        &genesis_block,
        &params,
        &mut state_accessor,
        &mut ledger,
    ) {
        Ok(result) => {
            println!("Genesis block processed successfully!");
            println!("  State root: {}", result.computed_state_root());
            last_header = genesis_block.signed_header().header().clone();
            processed_blocks.push(genesis_block);
            computed_state_roots.push(*result.computed_state_root());
        }
        Err(e) => {
            panic!("FATAL: Failed to process genesis block: {}", e);
        }
    }

    show_state(&state_accessor);
    show_help();

    // Interactive command loop
    let stdin = std::io::stdin();
    loop {
        print!("\nstf> ");
        std::io::Write::flush(&mut std::io::stdout())?;

        let mut input = String::new();
        if stdin.read_line(&mut input)? == 0 {
            break; // EOF
        }

        let command = input.trim();

        match command {
            "help" | "h" => show_help(),
            "state" | "s" => show_state(&state_accessor),
            "accounts" | "a" => show_accounts(&ledger),
            "blocks" | "l" => show_blocks(&processed_blocks, &computed_state_roots),
            cmd if cmd.starts_with("debug ") => {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if parts.len() != 2 {
                    println!("Usage: debug <slot>");
                    println!("Example: debug 1");
                } else {
                    match parts[1].parse::<u64>() {
                        Ok(slot) => show_block_debug(&processed_blocks, slot),
                        Err(_) => {
                            println!("Invalid slot number. Please enter a valid number.");
                            println!("Example: debug 1");
                        }
                    }
                }
            }
            cmd if cmd.starts_with("deposit ") => {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if parts.len() != 2 {
                    println!("Usage: deposit <amount>");
                    println!("Example: deposit 5000");
                } else {
                    match parts[1].parse::<u64>() {
                        Ok(amount) => {
                            println!("Processing L1 deposit of {} sats to account 0...", amount);
                            let block = create_deposit_block(&last_header, amount);

                            // Create a clone of the state for the first parameter (it's unused
                            // according to comments)
                            let state_clone = state_accessor.get_toplevel_state().clone();
                            match process_block(
                                &state_clone,
                                &last_header,
                                &block,
                                &params,
                                &mut state_accessor,
                                &mut ledger,
                            ) {
                                Ok(result) => {
                                    println!("SUCCESS: Deposit block processed!");
                                    println!("  New state root: {}", result.computed_state_root());
                                    println!("  Account 0 balance updated - use 'accounts' to see changes");
                                    last_header = block.signed_header().header().clone();
                                    processed_blocks.push(block);
                                    computed_state_roots.push(*result.computed_state_root());
                                }
                                Err(e) => {
                                    println!("ERROR: {}", e);
                                    // Still add the block to show it was processed, even if
                                    // validation failed
                                    processed_blocks.push(block);
                                    computed_state_roots.push(Buf32::zero()); // Use zero for failed
                                                                              // blocks
                                }
                            }
                        }
                        Err(_) => {
                            println!("Invalid amount. Please enter a valid number.");
                            println!("Example: deposit 5000");
                        }
                    }
                }
            }
            cmd if cmd.starts_with("withdraw ") => {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if parts.len() != 2 {
                    println!("Usage: withdraw <amount>");
                    println!("Example: withdraw 1000");
                } else {
                    match parts[1].parse::<u64>() {
                        Ok(amount) => {
                            println!("Processing withdrawal of {} sats from account 0...", amount);
                            let block = create_withdrawal_block(&last_header, amount);

                            // Create a clone of the state for the first parameter (it's unused
                            // according to comments)
                            let state_clone = state_accessor.get_toplevel_state().clone();
                            match process_block(
                                &state_clone,
                                &last_header,
                                &block,
                                &params,
                                &mut state_accessor,
                                &mut ledger,
                            ) {
                                Ok(result) => {
                                    println!("SUCCESS: Withdrawal block processed!");
                                    println!("  New state root: {}", result.computed_state_root());
                                    println!("  Account 0 balance updated - use 'accounts' to see changes");
                                    last_header = block.signed_header().header().clone();
                                    processed_blocks.push(block);
                                    computed_state_roots.push(*result.computed_state_root());
                                }
                                Err(e) => {
                                    println!("ERROR: {}", e);
                                    // Still add the block to show it was processed, even if
                                    // validation failed
                                    processed_blocks.push(block);
                                    computed_state_roots.push(Buf32::zero()); // Use zero for failed
                                                                              // blocks
                                }
                            }
                        }
                        Err(_) => {
                            println!("Invalid amount. Please enter a valid number.");
                            println!("Example: withdraw 1000");
                        }
                    }
                }
            }
            "block" | "b" => {
                println!("Processing test block...");
                let block = create_test_block(&last_header);

                // Create a clone of the state for the first parameter (it's unused according to
                // comments)
                let state_clone = state_accessor.get_toplevel_state().clone();
                match process_block(
                    &state_clone,
                    &last_header,
                    &block,
                    &params,
                    &mut state_accessor,
                    &mut ledger,
                ) {
                    Ok(result) => {
                        println!("SUCCESS: Test block processed!");
                        println!("  New state root: {}", result.computed_state_root());
                        last_header = block.signed_header().header().clone();
                        processed_blocks.push(block);
                        computed_state_roots.push(*result.computed_state_root());
                    }
                    Err(e) => {
                        println!("ERROR: {}", e);
                        // Still add the block to show it was processed, even if validation failed
                        processed_blocks.push(block);
                        computed_state_roots.push(Buf32::zero()); // Use zero for failed blocks
                    }
                }
            }
            "invalid" | "i" => {
                println!("Processing invalid block (should fail)...");
                let block = create_invalid_block(&last_header);

                // Create a clone of the state for the first parameter (it's unused according to
                // comments)
                let state_clone = state_accessor.get_toplevel_state().clone();
                match process_block(
                    &state_clone,
                    &last_header,
                    &block,
                    &params,
                    &mut state_accessor,
                    &mut ledger,
                ) {
                    Ok(result) => {
                        println!("UNEXPECTED: Invalid block was accepted!");
                        processed_blocks.push(block);
                        computed_state_roots.push(*result.computed_state_root());
                    }
                    Err(e) => {
                        println!("EXPECTED: Invalid block rejected: {}", e);
                        // Still add the block to show it was processed, even if rejected
                        processed_blocks.push(block);
                        computed_state_roots.push(Buf32::zero()); // Use zero for rejected blocks
                    }
                }
            }
            "quit" | "q" | "exit" => {
                println!("Goodbye!");
                break;
            }
            "" => continue, // Empty input
            _ => println!(
                "Unknown command: '{}'. Type 'help' for available commands.",
                command
            ),
        }
    }
    Ok(())
}

fn show_help() {
    println!("\nAvailable commands:");
    println!("  help (h)         - Show this help message");
    println!("  state (s)        - Show current STF state");
    println!("  accounts (a)     - Show account balances");
    println!("  blocks (l)       - List processed blocks");
    println!("  debug <slot>     - Show debug info for block at slot (e.g., 'debug 1')");
    println!("  block (b)        - Process valid test block");
    println!("  deposit <amt>    - Process L1 deposit to account 0 (e.g., 'deposit 5000')");
    println!("  withdraw <amt>   - Process withdrawal from account 0 (e.g., 'withdraw 1000')");
    println!("  invalid (i)      - Process invalid block (should fail)");
    println!("  quit (q)         - Exit the demo");
}

fn show_state(state_accessor: &SimpleStateAccessor) {
    let state = state_accessor.state_untracked();
    println!("\nCurrent STF State:");
    println!("  Slot: {}", state.cur_slot());
    println!("  Epoch: {}", state.cur_epoch());
    println!("  Accounts Root: {}", state.accounts_root());
    println!("  State Root: {}", state.compute_root());
}

fn show_accounts(ledger: &InMemoryVectorLedger) {
    println!("\nAccount States:");
    for i in 0..2 {
        let account_id = Buf32::from([i as u8; 32]);
        let account_hex = hex::encode(account_id.as_bytes());
        match ledger.account_state(&account_id) {
            Ok(Some(account)) => {
                println!(
                    "  Account {}: {} balance = {} sats",
                    i, account_hex, account.balance
                );
            }
            _ => println!("  Account {}: {} not found", i, account_hex),
        }
    }
}

fn show_blocks(blocks: &[OLBlock], computed_roots: &[Buf32]) {
    println!("\nProcessed Blocks:");
    if blocks.is_empty() {
        println!("  No blocks processed yet");
        return;
    }

    println!("  Slot    Block ID                         State Root");
    println!("  ----    --------                         ----------");
    for (i, block) in blocks.iter().enumerate() {
        let header = block.signed_header().header();
        let slot = header.slot();
        let block_id = header.compute_header_root();
        let state_root = if i < computed_roots.len() {
            computed_roots[i]
        } else {
            *header.state_root() // Fallback to header state root if no computed root available
        };

        println!(
            "  {:4}    {}..{}    {}..{}",
            slot,
            hex::encode(&block_id.as_ref()[..4]),
            hex::encode(&block_id.as_ref()[28..32]),
            hex::encode(&state_root.as_ref()[..4]),
            hex::encode(&state_root.as_ref()[28..32])
        );
    }
}

fn show_block_debug(blocks: &[OLBlock], slot: u64) {
    println!("\nDebug Info for Block at Slot {}:", slot);

    let block = blocks
        .iter()
        .find(|b| b.signed_header().header().slot() == slot);

    match block {
        Some(block) => {
            println!("{:#?}", block);
        }
        None => {
            println!("  Block at slot {} not found.", slot);
            println!(
                "  Available slots: {:?}",
                blocks
                    .iter()
                    .map(|b| b.signed_header().header().slot())
                    .collect::<Vec<_>>()
            );
        }
    }
}

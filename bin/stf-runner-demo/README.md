# STF Runner Demo

An interactive CLI demonstration of the State Transition Function (STF) for the Orchestration Layer (OL) blockchain. This demo showcases block processing, account management, L1->L2 deposits, withdrawals, and state transitions in real-time.

## Overview

The STF Runner Demo provides a hands-on way to understand how the Orchestration Layer processes blocks and manages account states. It demonstrates:

- Block Processing: Genesis, valid blocks, and invalid block rejection
- L1->L2 Bridge: Deposit functionality via ASM logs  
- Account Management: Balance tracking and state updates
- Withdrawals: Balance deduction through SNARK account updates
- State Inspection: Real-time STF state and account balance viewing

## Features

- Automatic Genesis: Genesis block processed on startup
- Deposit System: L1 deposits increase account balances
- Withdrawal System: Deducts balances via output transfers
- State Tracking: View slots, epochs, accounts root, and state root
- Block Inspector: Debug view of any processed block
- Validation: Demonstrates invalid block rejection
- Block Listing: Compact view of all processed blocks

## Getting Started

### Prerequisites

- Rust (latest stable)
- Cargo

### Running the Demo

From the project root:

```bash
cargo run --bin stf-runner-demo
```

Or from the demo directory:

```bash
cd bin/stf-runner-demo
cargo run
```

## Usage

Upon startup, the demo initializes:

```
STF Runner Demo - Interactive CLI
==================================

Initializing demo accounts...
  Account 0: 0000000000000000000000000000000000000000000000000000000000000000 balance = 0 sats
  Account 1: 0101010101010101010101010101010101010101010101010101010101010101 balance = 0 sats

Creating bridge account for withdrawals...
  Bridge Account: ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff for withdrawals

Processing genesis block...
Genesis block processed successfully!
  State root: f98ac7fb8baa471f

Current STF State:
  Slot: 0
  Epoch: 0
  Accounts Root: bcadcb..ac221a
  State Root: f98ac7..aa471f
```

### Available Commands

| Command | Alias | Description | Example |
|---------|-------|-------------|---------|
| help | h | Show available commands | help |
| state | s | Display current STF state | state |
| accounts | a | Show account balances | accounts |
| blocks | l | List all processed blocks | blocks |
| debug <slot> | | Show detailed block info | debug 1 |
| deposit <amount> | | Process L1 deposit | deposit 5000 |
| withdraw <amount> | | Process withdrawal | withdraw 1000 |
| block | b | Process empty test block | block |
| invalid | i | Process invalid block (fails) | invalid |
| quit | q | Exit the demo | quit |

## Example Workflow

### 1. Check Initial State

```
stf> state
Current STF State:
  Slot: 0
  Epoch: 0
  Accounts Root: bcadcb..ac221a
  State Root: f98ac7..aa471f

stf> accounts
Account States:
  Account 0: 0000000000...000 balance = 0 sats
  Account 1: 0101010101...101 balance = 0 sats
```

### 2. Make a Deposit

```
stf> deposit 5000
Processing L1 deposit of 5000 sats to account 0...
SUCCESS: Deposit block processed!
  New state root: 698480..8cf426
  Account 0 balance updated - use 'accounts' to see changes
```

### 3. Check Updated State

```
stf> state
Current STF State:
  Slot: 1
  Epoch: 1
  Accounts Root: 98e418..517a77
  State Root: 698480..8cf426

stf> accounts
Account States:
  Account 0: 0000000000...000 balance = 5000 sats
  Account 1: 0101010101...101 balance = 0 sats
```

### 4. Make a Withdrawal

```
stf> withdraw 1000
Processing withdrawal of 1000 sats from account 0...
SUCCESS: Withdrawal block processed!
  New state root: d480b7..764585
  Account 0 balance updated - use 'accounts' to see changes
```

### 5. Verify Balance Deduction

```
stf> accounts
Account States:
  Account 0: 0000000000...000 balance = 4000 sats
  Account 1: 0101010101...101 balance = 0 sats
```

### 6. View Block History

```
stf> blocks
Processed Blocks:
  Slot    Block ID                         State Root
  ----    --------                         ----------
     0    853c66fe..9273f72a    f98ac7fb..8baa471f
     1    646499ba..7501aa45    69848057..448cf426
     2    6f05d17e..3b9e1587    d480b737..9f764585
```

### 7. Debug a Specific Block

```
stf> debug 1
Debug Info for Block at Slot 1:
OLBlock {
    signed_header: SignedOLBlockHeader {
        header: OLBlockHeader {
            timestamp: 1000005000,
            slot: 1,
            epoch: 0,
            parent_blockid: 853c66fe..9273f72a,
            body_root: 0101010101..01010101,
            state_root: 0000000000..00000000,
        },
        signature: 0303030303..03030303,
    },
    body: OLBlockBody {
        logs: [],
        txs: None,
        l1update: Some(
            L1Update {
                inner_state_root: 0000000000..00000000,
                new_l1_height: 1000,
                manifests: [
                    AsmManifest {
                        l1_block_id: aaaaaaaaaa..aaaaaaaa,
                        logs: [
                            AsmLogEntry { ... }
                        ],
                    },
                ],
            },
        ),
    },
}
```

### 8. Test Invalid Block

```
stf> invalid
Processing invalid block (should fail)...
EXPECTED: Invalid block rejected: invalid block header: Invalid slot sequence
```

## Architecture

### Account System

- Account 0: 0000000000...000 - Primary demo account
- Account 1: 0101010101...101 - Secondary demo account  
- Bridge Account: ffffffff...fff - Withdrawal destination

### Block Types

1. Genesis Block (Slot 0): Establishes initial state
2. Deposit Blocks: Contain L1Update with ASM logs for deposits
3. Withdrawal Blocks: Contain transactions with OutputTransfer
4. Test Blocks: Empty blocks for slot progression
5. Invalid Blocks: Demonstrate validation (slot regression)

### State Tracking

- Slot: Increments with each processed block
- Epoch: Increments when L1Update is processed
- Accounts Root: Hash of all account states
- State Root: Computed root of current STF state

## Implementation Details

### Deposit Flow

1. DepositLog created with amount and target account
2. Converted to AsmLogEntry 
3. Packaged in AsmManifest and L1Update
4. Processed by STF, crediting account balance
5. Epoch incremented

### Withdrawal Flow

1. OutputTransfer created with destination and amount
2. Packaged in SnarkAccountUpdate transaction
3. STF processes transaction, debiting source account
4. Balance saved back to ledger

### State Root Computation

- Block headers contain dummy state roots during creation
- STF computes actual state root during processing
- Demo tracks computed roots for accurate display

## Code Structure

```
bin/stf-runner-demo/
├── main.rs              # Main CLI application
├── Cargo.toml          # Dependencies and binary config
└── README.md           # This file
```

### Key Functions

- main(): CLI loop and initialization
- create_*_block(): Block generation functions
- show_*(): Display functions for state/accounts/blocks
- SimpleStateAccessor: StateAccessor trait implementation

## Troubleshooting

### Common Issues

**Q: Why does my withdrawal not reduce the balance?**  
A: Ensure you have sufficient balance. Check with 'accounts' command first.

**Q: Why do some blocks get rejected?**  
A: The 'invalid' command intentionally creates blocks with slot regression to demonstrate validation.

**Q: What's the difference between state root in blocks vs STF state?**  
A: Block headers contain dummy state roots. The STF computes real state roots during processing.

## Development

### Adding New Commands

1. Add command parsing in main() match statement
2. Create corresponding block generation function
3. Update show_help() with new command

### Modifying Account Behavior

- Edit account initialization in main()
- Modify transaction generation in create_*_block()
- Update display logic in show_accounts()

## Dependencies

- stf-runner: Core STF implementation
- strata-primitives: Basic types and utilities
- strata-asm-logs: ASM log structures
- hex: Hexadecimal encoding for display
- anyhow: Error handling

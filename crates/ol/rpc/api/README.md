# strata-ol-rpc-api

RPC API definitions for the Orchestration Layer (OL).

## Overview

This crate provides the RPC interface traits for interacting with the OL chain. These interfaces expose methods needed by the Execution Layer (EL) and other components to query OL chain state and submit transactions.

## RPC Traits

### `OlApi` (namespace: `"ol"`)

Core methods for EL fullnodes to query OL state:

- `chain_status()` - Get current chain status (latest, confirmed, finalized blocks)
- `block_commitments_in_range(start_slot, end_slot)` - Get block commitments for a slot range

### `OlSequencerApi` (namespace: `"ol"`)

Additional methods for EL sequencer:

- `update_inputs_for_blocks(account_id, blocks)` - Get update input data for specific blocks for an account
- `inputs_for_block_range(account_id, block_ids)` - Get inputs for multiple blocks for an account
- `submit_transaction(tx)` - Submit a transaction to the OL mempool
- `messages_for_blocks(account_id, blocks)` - Get message payloads for specific blocks for an account

## RPC Types

All RPC types are properly structured to mirror their internal counterparts, with full JSON serialization support.

### Core Types

- **`RpcOlChainStatus`** - Chain status with finality levels
- **`RpcMsgPayload`** - Message payload (value in sats + hex-encoded data)
- **`RpcMessageEntry`** - Message entry with source, epoch, and payload
- **`RpcProofState`** - Proof state with inner state hash and message index
- **`RpcUpdateStateData`** - Update state with proof state and extra data
- **`RpcUpdateInputData`** - Complete update input (seq_no, messages, update_state)
- **`RpcTransactionPayload`** - Transaction payload enum:
  - `GenericAccountMessage` - Generic message to an account
  - `SnarkAccountUpdate` - Snark account update (note: excludes accumulator proofs)
- **`RpcTransactionExtra`** - Transaction constraints (min_slot, max_slot)
- **`RpcOlTransaction`** - Complete transaction (payload + extra)

### Container Types

- **`BlockMessages`** - Block ID + message payloads
- **`BlockUpdateInputs`** - Block ID + update input data

## Type Conversions

The crate provides `From`/`Into` implementations for converting between RPC types and their internal counterparts:

- `RpcMsgPayload` ⇔ `MsgPayload`
- `RpcMessageEntry` ⇔ `MessageEntry`
- `RpcProofState` ⇔ `ProofState`
- `RpcUpdateStateData` ⇔ `UpdateStateData`
- `RpcUpdateInputData` ⇔ `UpdateInputData`
- `RpcTransactionExtra` ⇔ `TransactionExtra`

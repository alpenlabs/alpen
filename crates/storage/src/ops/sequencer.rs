//! Sequencer payload data operation interface.

use strata_db_types::traits::SequencerDatabase;
use strata_ol_chain_types::L2BlockId;

use crate::exec::*;

inst_ops_simple! {
    (<D: SequencerDatabase> => SequencerPayloadOps) {
        put_exec_payload(slot: u64, block_id: L2BlockId, payload: Vec<u8>) => ();
        get_exec_payload(slot: u64) => Option<(L2BlockId, Vec<u8>)>;
        get_last_exec_payload_slot() => Option<u64>;
        get_exec_payloads_in_range(start_slot: u64, end_slot: u64) => Vec<(u64, L2BlockId, Vec<u8>)>;
        del_exec_payloads_from_slot(start_slot: u64) => Vec<u64>;
    }
}

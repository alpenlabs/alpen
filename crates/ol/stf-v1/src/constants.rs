use strata_acct_types::{
    AccountId, AccountSerial, BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL, MAX_MESSAGES,
};
use strata_ol_chain_types_v1::MAX_LOGS_PER_BLOCK;

// A SAU emits one update log and at most one log per output message.
const _: () = assert!(
    MAX_MESSAGES < MAX_LOGS_PER_BLOCK,
    "one snark update must fit the block log cap"
);

/// Maximum total encoded OL log payload size per epoch (16 KiB per SPS-ol-chain-structures).
///
/// Set below the full checkpoint envelope limit to reserve room for its other
/// components, such as the state diff. This bounds payloads in aggregate;
/// per-log size and checkpoint log count remain separate limits.
pub const MAX_TOTAL_LOG_PAYLOAD_BYTES: usize = 16 * 1024;

/// ID for sequencer-sent accounts.
// TODO(STR-3677): make this different, really, it should be the sequencer producing the block
pub const SEQUENCER_ACCT_ID: AccountId = BRIDGE_GATEWAY_ACCT_ID;

/// Serial of the bridge gateway account.
// TODO(STR-3677): make this different, really, it should be the sequencer producing the block
pub const SEQUENCER_ACCT_SERIAL: AccountSerial = BRIDGE_GATEWAY_ACCT_SERIAL;

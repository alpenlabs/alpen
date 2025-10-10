use std::{fmt, str};

use arbitrary::Arbitrary;
use const_hex as hex;
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;

/// Data that reflects what's happening around L1
#[derive(Clone, Serialize, Deserialize, Default, Arbitrary)]
pub struct L1Status {
    /// If the last time we tried to poll the client (as of `last_update`)
    /// we were successful.
    pub bitcoin_rpc_connected: bool,

    /// The last error message we received when trying to poll the client, if
    /// there was one.
    pub last_rpc_error: Option<String>,

    /// Current block height.
    pub cur_height: u64,

    /// Current tip block ID as string.
    pub cur_tip_blkid: String,

    /// Last published txid where L2 blob was present
    pub last_published_txid: Option<Buf32>,

    /// UNIX millis time of the last time we got a new update from the L1 connector.
    pub last_update: u64,

    /// Number of published reveal transactions.
    pub published_reveal_txs_count: u64,
}

// Custom debug implementation to print the txid in little endian
impl fmt::Debug for L1Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("L1Status");
        let mut debug_struct = debug
            .field("bitcoin_rpc_connected", &self.bitcoin_rpc_connected)
            .field("last_rpc_error", &self.last_rpc_error)
            .field("cur_height", &self.cur_height);

        // Handle cur_tip_blkid - reverse the hex string if it's a valid 32-byte hex
        if self.cur_tip_blkid.len() == 64 {
            // Try to parse as hex and reverse
            let mut blkid_bytes = [0u8; 32];
            if hex::decode_to_slice(&self.cur_tip_blkid, &mut blkid_bytes).is_ok() {
                blkid_bytes.reverse();
                let mut blkid_buf = [0u8; 64];
                hex::encode_to_slice(blkid_bytes, &mut blkid_buf).expect("buf: enc hex");
                // SAFETY: hex encoding always produces valid UTF-8
                let blkid_str = unsafe { str::from_utf8_unchecked(&blkid_buf) };
                debug_struct = debug_struct.field("cur_tip_blkid", &blkid_str);
            } else {
                debug_struct = debug_struct.field("cur_tip_blkid", &self.cur_tip_blkid);
            }
        } else {
            debug_struct = debug_struct.field("cur_tip_blkid", &self.cur_tip_blkid);
        }

        // Handle last_published_txid
        if let Some(txid) = self.last_published_txid {
            let mut txid_buf = [0u8; 64];
            {
                let mut bytes = txid.0;
                bytes.reverse();
                hex::encode_to_slice(bytes, &mut txid_buf).expect("buf: enc hex");
            }
            // SAFETY: hex encoding always produces valid UTF-8
            let txid_str = unsafe { str::from_utf8_unchecked(&txid_buf) };
            debug_struct = debug_struct.field("last_published_txid", &txid_str);
        } else {
            debug_struct = debug_struct.field("last_published_txid", &"None");
        }

        debug_struct
            .field("last_update", &self.last_update)
            .field(
                "published_reveal_txs_count",
                &self.published_reveal_txs_count,
            )
            .finish()
    }
}

// Custom display information to print the txid in little endian
impl fmt::Display for L1Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(txid) = self.last_published_txid {
            let mut txid_buf = [0u8; 64];
            {
                let mut bytes = txid.0;
                bytes.reverse();
                hex::encode_to_slice(bytes, &mut txid_buf).expect("buf: enc hex");
            }
            // SAFETY: hex encoding always produces valid UTF-8
            let txid_str = unsafe { str::from_utf8_unchecked(&txid_buf) };

            // Handle cur_tip_blkid - reverse the hex string if it's a valid 32-byte hex
            if self.cur_tip_blkid.len() == 64 {
                // Try to parse as hex and reverse
                let mut blkid_bytes = [0u8; 32];
                if hex::decode_to_slice(&self.cur_tip_blkid, &mut blkid_bytes).is_ok() {
                    blkid_bytes.reverse();
                    let mut blkid_buf = [0u8; 64];
                    hex::encode_to_slice(blkid_bytes, &mut blkid_buf).expect("buf: enc hex");
                    // SAFETY: hex encoding always produces valid UTF-8
                    let blkid_str = unsafe { str::from_utf8_unchecked(&blkid_buf) };
                    write!(
                        f,
                        "L1Status {{ bitcoin_rpc_connected: {}, cur_height: {}, cur_tip_blkid: {}, last_published_txid: {}, published_reveal_txs_count: {} }}",
                        self.bitcoin_rpc_connected,
                        self.cur_height,
                        blkid_str,
                        txid_str,
                        self.published_reveal_txs_count
                    )
                } else {
                    write!(
                        f,
                        "L1Status {{ bitcoin_rpc_connected: {}, cur_height: {}, cur_tip_blkid: {}, last_published_txid: {}, published_reveal_txs_count: {} }}",
                        self.bitcoin_rpc_connected,
                        self.cur_height,
                        self.cur_tip_blkid,
                        txid_str,
                        self.published_reveal_txs_count
                    )
                }
            } else {
                write!(
                    f,
                    "L1Status {{ bitcoin_rpc_connected: {}, cur_height: {}, cur_tip_blkid: {}, last_published_txid: {}, published_reveal_txs_count: {} }}",
                    self.bitcoin_rpc_connected,
                    self.cur_height,
                    self.cur_tip_blkid,
                    txid_str,
                    self.published_reveal_txs_count
                )
            }
        } else {
            // Handle cur_tip_blkid - reverse the hex string if it's a valid 32-byte hex
            if self.cur_tip_blkid.len() == 64 {
                // Try to parse as hex and reverse
                let mut blkid_bytes = [0u8; 32];
                if hex::decode_to_slice(&self.cur_tip_blkid, &mut blkid_bytes).is_ok() {
                    blkid_bytes.reverse();
                    let mut blkid_buf = [0u8; 64];
                    hex::encode_to_slice(blkid_bytes, &mut blkid_buf).expect("buf: enc hex");
                    // SAFETY: hex encoding always produces valid UTF-8
                    let blkid_str = unsafe { str::from_utf8_unchecked(&blkid_buf) };
                    write!(
                        f,
                        "L1Status {{ bitcoin_rpc_connected: {}, cur_height: {}, cur_tip_blkid: {}, last_published_txid: None, published_reveal_txs_count: {} }}",
                        self.bitcoin_rpc_connected,
                        self.cur_height,
                        blkid_str,
                        self.published_reveal_txs_count
                    )
                } else {
                    write!(
                        f,
                        "L1Status {{ bitcoin_rpc_connected: {}, cur_height: {}, cur_tip_blkid: {}, last_published_txid: None, published_reveal_txs_count: {} }}",
                        self.bitcoin_rpc_connected,
                        self.cur_height,
                        self.cur_tip_blkid,
                        self.published_reveal_txs_count
                    )
                }
            } else {
                write!(
                    f,
                    "L1Status {{ bitcoin_rpc_connected: {}, cur_height: {}, cur_tip_blkid: {}, last_published_txid: None, published_reveal_txs_count: {} }}",
                    self.bitcoin_rpc_connected,
                    self.cur_height,
                    self.cur_tip_blkid,
                    self.published_reveal_txs_count
                )
            }
        }
    }
}

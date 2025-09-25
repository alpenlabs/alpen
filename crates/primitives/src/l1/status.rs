use std::fmt;

use arbitrary::Arbitrary;
use const_hex as hex;
use serde::{Deserialize, Serialize};

use crate::buf::Buf32;

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
        let last_published_txid_le = self.last_published_txid.map(|txid| {
            let mut bytes = txid.0;
            bytes.reverse();
            hex::encode(bytes)
        });

        f.debug_struct("L1Status")
            .field("bitcoin_rpc_connected", &self.bitcoin_rpc_connected)
            .field("last_rpc_error", &self.last_rpc_error)
            .field("cur_height", &self.cur_height)
            .field("cur_tip_blkid", &self.cur_tip_blkid)
            .field("last_published_txid", &last_published_txid_le)
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
        let last_published_txid_le = self.last_published_txid.map(|txid| {
            let mut bytes = txid.0;
            bytes.reverse();
            hex::encode(bytes)
        });

        write!(
            f,
            "L1Status {{ bitcoin_rpc_connected: {}, cur_height: {}, cur_tip_blkid: {}, last_published_txid: {}, published_reveal_txs_count: {} }}",
            self.bitcoin_rpc_connected,
            self.cur_height,
            self.cur_tip_blkid,
            last_published_txid_le.as_deref().unwrap_or("None"),
            self.published_reveal_txs_count
        )
    }
}

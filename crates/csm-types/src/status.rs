use std::{fmt, str};

use arbitrary::Arbitrary;
use const_hex as hex;
use serde::{Deserialize, Serialize};
use strata_btc_types::BitcoinTxid;

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
    pub last_published_txid: Option<BitcoinTxid>,

    /// UNIX millis time of the last time we got a new update from the L1 connector.
    pub last_update: u64,

    /// Number of published reveal transactions.
    pub published_reveal_txs_count: u64,
}

// Helper to reverse a 32-byte hex string from big-endian to little-endian
fn reverse_hex_32(hex_str: &str) -> Option<String> {
    if hex_str.len() != 64 {
        return None;
    }

    let mut bytes = [0u8; 32];
    hex::decode_to_slice(hex_str, &mut bytes).ok()?;
    bytes.reverse();

    let mut buf = [0u8; 64];
    hex::encode_to_slice(bytes, &mut buf).expect("encoding to fixed buffer");

    // SAFETY: hex encoding always produces valid UTF-8
    Some(unsafe { str::from_utf8_unchecked(&buf).to_string() })
}

// Helper to reverse BitcoinTxid bytes to little-endian hex string
fn txid_to_le_hex(txid: &BitcoinTxid) -> String {
    let mut bytes = txid.inner_raw().0;
    bytes.reverse();

    let mut buf = [0u8; 64];
    hex::encode_to_slice(bytes, &mut buf).expect("encoding to fixed buffer");

    // SAFETY: hex encoding always produces valid UTF-8
    unsafe { str::from_utf8_unchecked(&buf).to_string() }
}

// Helper to format cur_tip_blkid for display (reversed if valid hex, otherwise as-is)
fn format_blkid(blkid: &str) -> String {
    reverse_hex_32(blkid).unwrap_or_else(|| blkid.to_string())
}

// Custom debug implementation to print the txid in little endian
impl fmt::Debug for L1Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("L1Status");
        debug
            .field("bitcoin_rpc_connected", &self.bitcoin_rpc_connected)
            .field("last_rpc_error", &self.last_rpc_error)
            .field("cur_height", &self.cur_height)
            .field("cur_tip_blkid", &format_blkid(&self.cur_tip_blkid));

        if let Some(txid) = &self.last_published_txid {
            debug.field("last_published_txid", &txid_to_le_hex(txid));
        } else {
            debug.field("last_published_txid", &"None");
        }

        debug
            .field("last_update", &self.last_update)
            .field("published_reveal_txs_count", &self.published_reveal_txs_count)
            .finish()
    }
}

// Custom display information to print the txid in little endian
impl fmt::Display for L1Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let blkid = format_blkid(&self.cur_tip_blkid);
        let txid_str = self
            .last_published_txid
            .as_ref()
            .map(|t| txid_to_le_hex(t))
            .unwrap_or_else(|| "None".to_string());

        write!(
            f,
            "L1Status {{ bitcoin_rpc_connected: {}, cur_height: {}, cur_tip_blkid: {}, last_published_txid: {}, published_reveal_txs_count: {} }}",
            self.bitcoin_rpc_connected,
            self.cur_height,
            blkid,
            txid_str,
            self.published_reveal_txs_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_txid() -> BitcoinTxid {
        // Create a sample txid for testing
        // Using a known Bitcoin txid (Satoshi's first transaction)
        let txid_hex = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
        // Parse the txid from hex string
        let bitcoin_txid: bitcoin::Txid = txid_hex.parse().unwrap();
        BitcoinTxid::from(bitcoin_txid)
    }

    #[test]
    fn test_debug_with_valid_blkid_and_txid() {
        let status = L1Status {
            bitcoin_rpc_connected: true,
            last_rpc_error: Some("test error".to_string()),
            cur_height: 12345,
            // Bitcoin genesis block hash (big-endian)
            cur_tip_blkid: "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
                .to_string(),
            last_published_txid: Some(create_test_txid()),
            last_update: 1234567890,
            published_reveal_txs_count: 42,
        };

        let debug_output = format!("{:?}", status);

        // Verify all fields are present
        assert!(debug_output.contains("bitcoin_rpc_connected: true"));
        assert!(debug_output.contains(r#"last_rpc_error: Some("test error")"#));
        assert!(debug_output.contains("cur_height: 12345"));
        // Verify blkid is reversed (little-endian)
        assert!(debug_output.contains("6fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000"));
        // Verify txid appears in output (already in display format from inner_raw reversal)
        assert!(debug_output.contains("4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"));
        assert!(debug_output.contains("last_update: 1234567890"));
        assert!(debug_output.contains("published_reveal_txs_count: 42"));
    }

    #[test]
    fn test_debug_with_invalid_blkid() {
        let status = L1Status {
            bitcoin_rpc_connected: false,
            last_rpc_error: None,
            cur_height: 100,
            cur_tip_blkid: "invalid".to_string(),
            last_published_txid: None,
            last_update: 1000,
            published_reveal_txs_count: 0,
        };

        let debug_output = format!("{:?}", status);

        // Invalid blkid should be shown as-is
        assert!(debug_output.contains(r#"cur_tip_blkid: "invalid""#));
        // None should be shown as string "None"
        assert!(debug_output.contains(r#"last_published_txid: "None""#));
    }

    #[test]
    fn test_debug_with_none_txid() {
        let status = L1Status {
            bitcoin_rpc_connected: true,
            last_rpc_error: None,
            cur_height: 999,
            cur_tip_blkid: "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
                .to_string(),
            last_published_txid: None,
            last_update: 9999999,
            published_reveal_txs_count: 5,
        };

        let debug_output = format!("{:?}", status);

        assert!(debug_output.contains(r#"last_published_txid: "None""#));
    }

    #[test]
    fn test_display_with_valid_blkid_and_txid() {
        let status = L1Status {
            bitcoin_rpc_connected: true,
            last_rpc_error: Some("ignored in display".to_string()),
            cur_height: 12345,
            cur_tip_blkid: "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
                .to_string(),
            last_published_txid: Some(create_test_txid()),
            last_update: 1234567890,
            published_reveal_txs_count: 42,
        };

        let display_output = format!("{}", status);

        // Should be single-line format
        assert!(display_output.starts_with("L1Status {"));
        assert!(display_output.contains("bitcoin_rpc_connected: true"));
        assert!(display_output.contains("cur_height: 12345"));
        // Verify blkid is reversed
        assert!(display_output.contains("6fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000"));
        // Verify txid appears in output
        assert!(display_output.contains("4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"));
        assert!(display_output.contains("published_reveal_txs_count: 42"));

        // Should NOT contain fields that Display omits
        assert!(!display_output.contains("last_rpc_error"));
        assert!(!display_output.contains("last_update"));
    }

    #[test]
    fn test_display_with_none_txid() {
        let status = L1Status {
            bitcoin_rpc_connected: false,
            last_rpc_error: None,
            cur_height: 100,
            cur_tip_blkid: "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
                .to_string(),
            last_published_txid: None,
            last_update: 1000,
            published_reveal_txs_count: 0,
        };

        let display_output = format!("{}", status);

        assert!(display_output.contains("last_published_txid: None"));
        assert!(display_output.contains("bitcoin_rpc_connected: false"));
    }

    #[test]
    fn test_display_with_invalid_blkid() {
        let status = L1Status {
            bitcoin_rpc_connected: true,
            last_rpc_error: None,
            cur_height: 200,
            cur_tip_blkid: "short".to_string(),
            last_published_txid: Some(create_test_txid()),
            last_update: 2000,
            published_reveal_txs_count: 10,
        };

        let display_output = format!("{}", status);

        // Invalid blkid should be shown as-is
        assert!(display_output.contains("cur_tip_blkid: short"));
    }

    #[test]
    fn test_display_with_invalid_blkid_and_none_txid() {
        let status = L1Status {
            bitcoin_rpc_connected: false,
            last_rpc_error: Some("error".to_string()),
            cur_height: 300,
            cur_tip_blkid: "not_valid_hex".to_string(),
            last_published_txid: None,
            last_update: 3000,
            published_reveal_txs_count: 15,
        };

        let display_output = format!("{}", status);

        assert!(display_output.contains("cur_tip_blkid: not_valid_hex"));
        assert!(display_output.contains("last_published_txid: None"));
        assert!(display_output.contains("cur_height: 300"));
    }
}

//! Extension traits for converting between `bitcoin` types and `strata-identifiers` types.

use bitcoin::{BlockHash, Txid, Wtxid, hashes::Hash, secp256k1::schnorr::Signature};
use strata_identifiers::{Buf32, Buf64, L1BlockId, RBuf32};

/// Extension methods for converting [`BlockHash`] to identifier types.
pub trait BlockHashExt {
    /// Converts to a [`Buf32`] containing the raw hash bytes.
    fn to_buf32(&self) -> Buf32;

    /// Converts to an [`L1BlockId`].
    fn to_l1_block_id(&self) -> L1BlockId;
}

impl BlockHashExt for BlockHash {
    fn to_buf32(&self) -> Buf32 {
        Buf32::from(*self.as_raw_hash().as_byte_array())
    }

    fn to_l1_block_id(&self) -> L1BlockId {
        L1BlockId::from(RBuf32(*self.as_raw_hash().as_byte_array()))
    }
}

/// Extension methods for converting [`Txid`] to identifier types.
pub trait TxidExt {
    /// Converts to a [`Buf32`] containing the raw hash bytes.
    fn to_buf32(&self) -> Buf32;
}

impl TxidExt for Txid {
    fn to_buf32(&self) -> Buf32 {
        Buf32::from(*self.as_raw_hash().as_byte_array())
    }
}

/// Extension methods for converting [`Wtxid`] to identifier types.
pub trait WtxidExt {
    /// Converts to a [`Buf32`] containing the raw hash bytes.
    fn to_buf32(&self) -> Buf32;
}

impl WtxidExt for Wtxid {
    fn to_buf32(&self) -> Buf32 {
        Buf32::from(*self.as_raw_hash().as_byte_array())
    }
}

/// Extension methods for converting [`bitcoin::secp256k1::schnorr::Signature`] to identifier
/// types.
pub trait SignatureExt {
    /// Converts to a [`Buf64`] containing the serialized signature bytes.
    fn to_buf64(&self) -> Buf64;
}

impl SignatureExt for Signature {
    fn to_buf64(&self) -> Buf64 {
        Buf64::from(self.serialize())
    }
}

/// Extension methods for converting [`Buf32`] to Bitcoin hash types.
pub trait Buf32BitcoinExt {
    /// Converts to a [`Txid`].
    fn to_txid(&self) -> Txid;

    /// Converts to a [`Wtxid`].
    fn to_wtxid(&self) -> Wtxid;

    /// Converts to a [`BlockHash`].
    fn to_block_hash(&self) -> BlockHash;
}

impl Buf32BitcoinExt for Buf32 {
    fn to_txid(&self) -> Txid {
        Txid::from_byte_array(self.0)
    }

    fn to_wtxid(&self) -> Wtxid {
        Wtxid::from_byte_array(self.0)
    }

    fn to_block_hash(&self) -> BlockHash {
        BlockHash::from_byte_array(self.0)
    }
}

/// Extension methods for converting [`L1BlockId`] to Bitcoin types.
pub trait L1BlockIdBitcoinExt {
    /// Converts to a [`BlockHash`].
    fn to_block_hash(&self) -> BlockHash;
}

impl L1BlockIdBitcoinExt for L1BlockId {
    fn to_block_hash(&self) -> BlockHash {
        BlockHash::from_byte_array(*self.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that `L1BlockId`'s `Debug` output matches Bitcoin's `BlockHash` `Display`,
    /// i.e. the full reversed-byte hex string.
    #[test]
    fn debug_matches_bitcoin_display() {
        let block_hash: BlockHash =
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
                .parse()
                .unwrap();
        let l1_id = block_hash.to_l1_block_id();

        assert_eq!(format!("{:?}", l1_id), format!("{}", block_hash));
    }

    /// Verifies that `L1BlockId`'s human-readable serde serialization matches
    /// Bitcoin's `BlockHash` serialization (both produce the reversed-hex string).
    #[test]
    fn serde_json_matches_bitcoin() {
        let block_hash: BlockHash =
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
                .parse()
                .unwrap();
        let l1_id = block_hash.to_l1_block_id();

        let l1_json = serde_json::to_string(&l1_id).unwrap();
        let btc_json = serde_json::to_string(&block_hash).unwrap();
        assert_eq!(l1_json, btc_json);
    }

    /// Verifies round-trip: deserializing a Bitcoin `BlockHash` JSON string as
    /// `L1BlockId` produces the same value as converting via the extension trait.
    #[test]
    fn serde_json_roundtrip_from_bitcoin() {
        let block_hash: BlockHash =
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
                .parse()
                .unwrap();

        let json = serde_json::to_string(&block_hash).unwrap();
        let deserialized: L1BlockId = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized, block_hash.to_l1_block_id());
    }

    /// Verifies Buf32 ↔ Txid round-trip.
    #[test]
    fn buf32_txid_roundtrip() {
        let buf = Buf32::from([42u8; 32]);
        let txid = buf.to_txid();
        let back = txid.to_buf32();
        assert_eq!(buf, back);
    }

    /// Verifies Buf32 ↔ Wtxid round-trip.
    #[test]
    fn buf32_wtxid_roundtrip() {
        let buf = Buf32::from([7u8; 32]);
        let wtxid = buf.to_wtxid();
        let back = wtxid.to_buf32();
        assert_eq!(buf, back);
    }

    /// Verifies L1BlockId ↔ BlockHash round-trip.
    #[test]
    fn l1_block_id_block_hash_roundtrip() {
        let block_hash: BlockHash =
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
                .parse()
                .unwrap();
        let l1_id = block_hash.to_l1_block_id();
        let back = l1_id.to_block_hash();
        assert_eq!(block_hash, back);
    }
}

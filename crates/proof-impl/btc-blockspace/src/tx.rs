use bitcoin::{consensus::Encodable, Transaction};
use strata_primitives::{buf::Buf32, hash::sha256d};

/// Computes the [`Txid`](bitcoin::Txid) using [RustCrypto's SHA-2 crate](https://github.com/RustCrypto/hashes/tree/master/sha2)
/// for the underlying `sha256d` hash function.
///
/// Equivalent to [`compute_txid`](bitcoin::Transaction::compute_txid)
///
/// This function hashes the transaction **excluding** the segwit data (i.e., the marker, flag
/// bytes, and the witness fields themselves). For non-segwit transactions, which do not have any
/// segwit data, this will be equal to [`compute_wtxid`].
pub fn compute_txid(tx: &Transaction) -> Buf32 {
    let mut vec = Vec::new();

    tx.version.consensus_encode(&mut vec).unwrap();
    tx.input.consensus_encode(&mut vec).unwrap();
    tx.output.consensus_encode(&mut vec).unwrap();
    tx.lock_time.consensus_encode(&mut vec).unwrap();

    sha256d(&vec)
}

/// Computes the segwit version of the transaction id using [RustCrypto's SHA-2 crate](https://github.com/RustCrypto/hashes/tree/master/sha2)
///
/// Equivalent to [`compute_wtxid`](bitcoin::Transaction::compute_wtxid)
///
/// Hashes the transaction **including** all segwit data (i.e. the marker, flag bytes, and the
/// witness fields themselves). For non-segwit transactions which do not have any segwit data,
/// this will be equal to [`compute_txid`].
pub fn compute_wtxid(tx: &Transaction) -> Buf32 {
    let mut vec = Vec::new();
    tx.consensus_encode(&mut vec).expect("engines don't error");
    sha256d(&vec)
}

#[cfg(test)]
mod tests {
    use bitcoin::{hashes::Hash, Txid, Wtxid};
    use strata_test_utils_btc::get_btc_mainnet_block;

    use super::*;

    #[test]
    fn test_txid() {
        let block = get_btc_mainnet_block();
        for tx in &block.txdata {
            assert_eq!(
                tx.compute_txid(),
                Txid::from_byte_array(*compute_txid(tx).as_ref())
            )
        }
    }

    #[test]
    fn test_wtxid() {
        let block = get_btc_mainnet_block();
        for tx in &block.txdata {
            assert_eq!(
                tx.compute_wtxid(),
                Wtxid::from_byte_array(*compute_wtxid(tx).as_ref())
            )
        }
    }
}
